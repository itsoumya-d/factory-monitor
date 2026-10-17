//! EU AI Act Art 5(1)(f): emotion/mood inference is forbidden outside the
//! medical & safety whitelist. The lock has three prongs:
//! 1. vocabulary scan on every prompt that may reach an open-vocab head,
//! 2. an emotion head type that cannot be constructed,
//! 3. the whitelist of permitted *observable* states (no valence labels).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExceptionKind {
    /// Clinical use of affect recognition in a hospital setting.
    MedicalDiagnosis,
    /// Fatigue/impairment monitoring that protects the worker.
    WorkplaceSafety,
    /// Mobility assistance (fall-risk, stagger detection).
    MobilityAssistance,
}

/// Words whose presence in a prompt means "this prompt infers an
/// emotional state". Case-insensitive, compared per whitespace token.
pub const FORBIDDEN_PROMPT_WORDS: &[&str] = &[
    "happy",
    "sad",
    "angry",
    "smiling",
    "frowning",
    "depressed",
    "grinning",
    "enjoying",
    "excited",
    "scared",
    "fear",
    "surprised",
    "disgust",
];

/// The only human-state labels the whole platform may produce on persons.
pub const ALLOWED_PERSON_STATES: &[&str] = &[
    "standing",
    "bent_over",
    "sitting",
    "walking",
    "lying_down",
    "hands_raised",
    "hands_up",
    "fatigue",
    "staggering",
    "fall",
];

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Article5Error {
    #[error("prompt or label infers emotional state (word: {0}) — forbidden by Art 5(1)(f)")]
    ForbiddenInfersState(&'static str),
    #[error("person-state label {0} is outside the medical/safety whitelist")]
    NotWhitelisted(String),
    #[error("emotion head construction is always forbidden")]
    EmotionHeadLocked,
}

/// Vocabulary scan over prompt text. Every open-vocab target, rule label
/// and agent prompt goes through this before load — and again at ingest.
pub fn scan_prompt(prompt: &str) -> Result<(), Article5Error> {
    let lowered = prompt.to_lowercase();
    for word in FORBIDDEN_PROMPT_WORDS {
        // Compare with punctuation stripped so "smiling," still matches.
        for token in lowered.split_whitespace() {
            let stripped: String = token.chars().filter(|c| c.is_ascii_alphabetic()).collect();
            if stripped == *word {
                return Err(Article5Error::ForbiddenInfersState(word));
            }
            // Compounds such as "sadfaced" also infer state.
            if stripped.len() > word.len() && stripped.starts_with(word) {
                return Err(Article5Error::ForbiddenInfersState(word));
            }
        }
    }
    Ok(())
}

/// Whitelist check for a person-state taxonomy entry. Only posture and
/// safety states — never affect labels — may exist in this platform.
pub fn whitelist_person_state(state: &str) -> Result<(), Article5Error> {
    if ALLOWED_PERSON_STATES.contains(&state.to_lowercase().as_str()) {
        Ok(())
    } else {
        Err(Article5Error::NotWhitelisted(state.to_string()))
    }
}

/// Prong 2 — the emotion head is a type that cannot exist.
///
/// The constructor only ever returns a typed refusal. There is no code
/// path that can construct this head, and the test below asserts that.
#[derive(Debug)]
pub struct EmotionHead {
    _never: std::convert::Infallible,
}

impl EmotionHead {
    pub fn try_new() -> Result<Self, Article5Error> {
        Err(Article5Error::EmotionHeadLocked)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn forbidden_words_detected_case_insensitively() {
        for p in [
            "a happy customer",
            "SMILING worker",
            "an angry machine operator",
            "she was sadfaced,",
        ] {
            let got = scan_prompt(p);
            assert!(got.is_err(), "{p} must be rejected");
        }
    }

    #[test]
    fn neutral_observables_pass() {
        for p in [
            "a worker bent at the waist",
            "a person lying on the floor",
            "a hand raised to shoulder height",
        ] {
            assert!(scan_prompt(p).is_ok(), "{p} is an observable, allowed");
        }
    }

    #[test]
    fn whitelist_only_known_states() {
        assert!(whitelist_person_state("bent_over").is_ok());
        assert!(whitelist_person_state("fall").is_ok());
        assert_eq!(
            whitelist_person_state("joyful"),
            Err(Article5Error::NotWhitelisted("joyful".to_string()))
        );
    }

    #[test]
    fn emotion_head_is_unconstructible() {
        assert!(matches!(
            EmotionHead::try_new(),
            Err(Article5Error::EmotionHeadLocked)
        ));
    }
}
