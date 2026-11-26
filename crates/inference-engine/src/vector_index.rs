//! High-throughput in-memory cosine vector index for fast similarity search
//! and anomaly embedding retrieval across edge inference events.

use std::cmp::Ordering;

use crate::error::InferenceError;

/// High-performance flat vector index for normalized cosine similarity search.
///
/// Stores embeddings contiguously in a flat buffer for maximum cache locality
/// and predictable low-latency linear scans across millions of inference ticks.
#[derive(Debug, Clone)]
pub struct VectorIndex {
    dimension: usize,
    ids: Vec<u64>,
    data: Vec<f32>,
}

impl VectorIndex {
    /// Creates a new empty `VectorIndex` for embeddings of the specified dimension.
    pub fn new(dimension: usize) -> Result<Self, InferenceError> {
        if dimension == 0 {
            return Err(InferenceError::EmptyVector);
        }
        Ok(Self {
            dimension,
            ids: Vec::new(),
            data: Vec::new(),
        })
    }

    /// Creates a new `VectorIndex` with pre-allocated capacity for `capacity` vectors.
    pub fn with_capacity(dimension: usize, capacity: usize) -> Result<Self, InferenceError> {
        if dimension == 0 {
            return Err(InferenceError::EmptyVector);
        }
        Ok(Self {
            dimension,
            ids: Vec::with_capacity(capacity),
            data: Vec::with_capacity(capacity * dimension),
        })
    }

    /// Number of vectors stored in the index.
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    /// Returns true if no vectors have been indexed.
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// Dimensionality of the vectors stored in the index.
    pub fn dimension(&self) -> usize {
        self.dimension
    }

    /// Inserts a vector with an associated ID into the index.
    ///
    /// Automatically normalizes the vector to unit L2 norm so dot products
    /// correspond directly to cosine similarity.
    pub fn insert(&mut self, id: u64, vector: &[f32]) -> Result<(), InferenceError> {
        if vector.len() != self.dimension {
            return Err(InferenceError::VectorDimensionMismatch {
                expected: self.dimension,
                actual: vector.len(),
            });
        }

        let norm = (vector.iter().map(|&x| x * x).sum::<f32>()).sqrt();
        if norm <= f32::EPSILON {
            return Err(InferenceError::EmptyVector);
        }

        let inv_norm = 1.0 / norm;
        self.ids.push(id);
        self.data.reserve(self.dimension);
        for &val in vector {
            self.data.push(val * inv_norm);
        }

        Ok(())
    }

    /// Computes dot product between a normalized query and indexed vector at index `idx`.
    #[inline]
    fn dot(&self, idx: usize, query: &[f32]) -> f32 {
        let offset = idx * self.dimension;
        let slice = &self.data[offset..offset + self.dimension];
        slice.iter().zip(query.iter()).map(|(&a, &b)| a * b).sum()
    }

    /// Normalizes query vector or returns an error if invalid.
    fn prepare_query(&self, query: &[f32]) -> Result<Vec<f32>, InferenceError> {
        if query.len() != self.dimension {
            return Err(InferenceError::VectorDimensionMismatch {
                expected: self.dimension,
                actual: query.len(),
            });
        }
        let norm = (query.iter().map(|&x| x * x).sum::<f32>()).sqrt();
        if norm <= f32::EPSILON {
            return Err(InferenceError::EmptyVector);
        }
        let inv_norm = 1.0 / norm;
        Ok(query.iter().map(|&x| x * inv_norm).collect())
    }

    /// Returns the top-k nearest neighbors sorted by cosine similarity descending.
    pub fn top_k(&self, query: &[f32], k: usize) -> Result<Vec<(u64, f32)>, InferenceError> {
        if k == 0 || self.is_empty() {
            return Ok(Vec::new());
        }

        let norm_query = self.prepare_query(query)?;
        let mut scored: Vec<(u64, f32)> = self
            .ids
            .iter()
            .enumerate()
            .map(|(idx, &id)| (id, self.dot(idx, &norm_query)))
            .collect();

        // Sort descending by score
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));
        if scored.len() > k {
            scored.truncate(k);
        }

        Ok(scored)
    }

    /// Returns all vectors with cosine similarity above or equal to `min_similarity`.
    pub fn find_threshold(
        &self,
        query: &[f32],
        min_similarity: f32,
    ) -> Result<Vec<(u64, f32)>, InferenceError> {
        if self.is_empty() {
            return Ok(Vec::new());
        }

        let norm_query = self.prepare_query(query)?;
        let mut matches: Vec<(u64, f32)> = Vec::new();

        for (idx, &id) in self.ids.iter().enumerate() {
            let sim = self.dot(idx, &norm_query);
            if sim >= min_similarity {
                matches.push((id, sim));
            }
        }

        matches.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));
        Ok(matches)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_and_capacity() {
        let idx = VectorIndex::with_capacity(4, 10).expect("valid capacity");
        assert_eq!(idx.dimension(), 4);
        assert_eq!(idx.len(), 0);
        assert!(idx.is_empty());

        let res = idx.top_k(&[1.0, 0.0, 0.0, 0.0], 5).expect("valid search");
        assert!(res.is_empty());
    }

    #[test]
    fn test_dimension_validation() {
        let mut idx = VectorIndex::new(3).expect("valid index");
        assert!(idx.insert(1, &[1.0, 0.0]).is_err());
        assert!(idx.insert(1, &[0.0, 0.0, 0.0]).is_err()); // Zero norm
        assert!(idx.insert(1, &[1.0, 0.0, 0.0]).is_ok());

        assert!(idx.top_k(&[1.0, 0.0], 1).is_err());
    }

    #[test]
    fn test_exact_cosine_similarity() {
        let mut idx = VectorIndex::new(3).expect("valid index");
        idx.insert(101, &[1.0, 0.0, 0.0]).expect("insert unit X"); // Unit X
        idx.insert(102, &[0.0, 2.0, 0.0]).expect("insert unit Y"); // Unit Y (unnormalized input)
        idx.insert(103, &[-1.0, 0.0, 0.0]).expect("insert -unit X"); // -Unit X

        let query = [1.0, 0.0, 0.0];
        let top = idx.top_k(&query, 3).expect("top_k query");
        assert_eq!(top.len(), 3);

        // Identical direction -> similarity ~1.0
        assert_eq!(top[0].0, 101);
        assert!((top[0].1 - 1.0).abs() < 1e-6);

        // Orthogonal direction -> similarity ~0.0
        assert_eq!(top[1].0, 102);
        assert!(top[1].1.abs() < 1e-6);

        // Opposite direction -> similarity ~ -1.0
        assert_eq!(top[2].0, 103);
        assert!((top[2].1 - -1.0).abs() < 1e-6);
    }

    #[test]
    fn test_top_k_ordering_and_truncation() {
        let mut idx = VectorIndex::new(2).expect("valid index");
        idx.insert(1, &[1.0, 0.0]).expect("insert v1");
        idx.insert(2, &[0.9, 0.1]).expect("insert v2");
        idx.insert(3, &[0.5, 0.5]).expect("insert v3");
        idx.insert(4, &[0.0, 1.0]).expect("insert v4");

        let top2 = idx.top_k(&[1.0, 0.0], 2).expect("query top 2");
        assert_eq!(top2.len(), 2);
        assert_eq!(top2[0].0, 1);
        assert_eq!(top2[1].0, 2);
        assert!(top2[0].1 > top2[1].1);
    }

    #[test]
    fn test_find_threshold() {
        let mut idx = VectorIndex::new(2).expect("valid index");
        idx.insert(1, &[1.0, 0.0]).expect("insert v1");
        idx.insert(2, &[0.8, 0.6]).expect("insert v2");
        idx.insert(3, &[0.0, 1.0]).expect("insert v3");

        let query = [1.0, 0.0];
        let threshold_matches = idx.find_threshold(&query, 0.75).expect("query threshold");
        assert_eq!(threshold_matches.len(), 2);
        assert_eq!(threshold_matches[0].0, 1);
        assert_eq!(threshold_matches[1].0, 2);
    }
}
