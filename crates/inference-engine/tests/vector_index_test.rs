//! Integration tests for the in-memory cosine vector index.

use inference_engine::VectorIndex;

#[test]
fn vector_index_lifecycle_and_top_k() {
    let mut index = VectorIndex::with_capacity(3, 16).expect("index created");
    assert_eq!(index.dimension(), 3);
    assert_eq!(index.len(), 0);
    assert!(index.is_empty());

    // Insert reference vectors:
    // ID 10: pure X axis
    // ID 20: pure Y axis
    // ID 30: pure Z axis
    // ID 40: diagonal in XY plane (unnormalized input)
    index.insert(10, &[1.0, 0.0, 0.0]).expect("insert ID 10");
    index.insert(20, &[0.0, 1.0, 0.0]).expect("insert ID 20");
    index.insert(30, &[0.0, 0.0, 1.0]).expect("insert ID 30");
    index.insert(40, &[3.0, 3.0, 0.0]).expect("insert ID 40");

    assert_eq!(index.len(), 4);
    assert!(!index.is_empty());

    // Query along X axis
    let query_x = [1.0, 0.0, 0.0];
    let top_matches = index.top_k(&query_x, 2).expect("query top 2");

    assert_eq!(top_matches.len(), 2);
    // Highest similarity is exact match ID 10 (~1.0)
    assert_eq!(top_matches[0].0, 10);
    assert!((top_matches[0].1 - 1.0).abs() < 1e-5);

    // Second highest is diagonal ID 40 (~0.7071)
    assert_eq!(top_matches[1].0, 40);
    assert!((top_matches[1].1 - 0.707106).abs() < 1e-4);

    // Threshold search for similarity >= 0.5
    let threshold_matches = index.find_threshold(&query_x, 0.5).expect("threshold search");
    assert_eq!(threshold_matches.len(), 2);
    assert_eq!(threshold_matches[0].0, 10);
    assert_eq!(threshold_matches[1].0, 40);
}

#[test]
fn vector_index_dimension_and_edge_cases() {
    let mut index = VectorIndex::new(4).expect("valid index");

    // Rejection on dimension mismatch
    assert!(index.insert(1, &[1.0, 2.0, 3.0]).is_err());
    assert!(index.top_k(&[1.0, 2.0], 1).is_err());

    // Rejection on zero norm
    assert!(index.insert(2, &[0.0, 0.0, 0.0, 0.0]).is_err());

    // Valid insert
    index.insert(100, &[0.5, 0.5, 0.5, 0.5]).expect("insert valid");

    // Empty result for k = 0
    let empty_k = index.top_k(&[1.0, 0.0, 0.0, 0.0], 0).expect("k=0 search");
    assert!(empty_k.is_empty());
}
