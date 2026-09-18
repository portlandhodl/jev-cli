//! The JSON request document accepted by `jev-cli ask`.
//!
//! `ask` is the full-power interface: one JSON document in, one JSON
//! document out. The document mirrors the System One API request:
//!
//! ```json
//! {
//!   "state": "anything JSON: a string, object, array, ...",
//!   "questions": {
//!     "will_succeed": {"type": "noul", "instructions": "Will this work?"},
//!     "approach": {
//!       "type": "choice",
//!       "instructions": "Which approach?",
//!       "criteria": {"retry": "Try again", "abort": null}
//!     },
//!     "risk": {"type": "score", "instructions": "How risky?", "criteria": ["low", "high"]}
//!   },
//!   "model": "jev-latest"
//! }
//! ```
//!
//! `model` is optional; a CLI `--model` flag wins over it.

use serde::Deserialize;
use serde_json::{Map, Value};

use crate::error::Error;

/// Request documents larger than this are refused (they come from files or
/// stdin; see [`crate::input::MAX_STATE_BYTES`]).
pub const MAX_REQUEST_BYTES: u64 = 16 * 1024 * 1024; // 16 MiB

/// A full System One request as a JSON document. Unknown fields are
/// rejected so typos surface as errors instead of silently ignored intent.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AskRequest {
    /// The situation to evaluate: any JSON value.
    pub state: Value,
    /// Question id -> question object, in the API's wire shape. Insertion
    /// order is preserved and echoed back in the response.
    pub questions: Map<String, Value>,
    /// Optional per-request model override.
    pub model: Option<String>,
}

impl AskRequest {
    /// Parse and validate a request document.
    pub fn parse(raw: &str) -> Result<Self, Error> {
        let request: AskRequest = serde_json::from_str(raw)
            .map_err(|e| Error::Usage(format!("request document is not valid: {e}")))?;
        request.validate()?;
        Ok(request)
    }

    fn validate(&self) -> Result<(), Error> {
        if self.questions.is_empty() {
            return Err(Error::Usage(
                "request has no questions: \"questions\" must map at least one id to a question"
                    .into(),
            ));
        }
        for (id, raw) in &self.questions {
            if id.trim().is_empty() {
                return Err(Error::Usage("question ids must not be empty".into()));
            }
            // Parse each question now so shape errors name the offending id
            // instead of failing deep inside the SDK request builder.
            serde_json::from_value::<jev_sdk::Question>(raw.clone()).map_err(|e| {
                Error::Usage(format!("question {id:?} is not a valid question: {e}"))
            })?;
        }
        if let Some(model) = &self.model {
            if model.trim().is_empty() {
                return Err(Error::Usage("\"model\" must not be empty".into()));
            }
        }
        Ok(())
    }

    /// The questions as typed SDK values, in document order. Parsing has
    /// already succeeded in [`AskRequest::parse`], so failures here are
    /// impossible — but this returns `Result` rather than dropping entries.
    pub fn typed_questions(&self) -> Result<Vec<(String, jev_sdk::Question)>, Error> {
        self.questions
            .iter()
            .map(|(id, raw)| {
                serde_json::from_value::<jev_sdk::Question>(raw.clone())
                    .map(|q| (id.clone(), q))
                    .map_err(|e| Error::Usage(format!("question {id:?}: {e}")))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_full_document() {
        let raw = r#"{
            "state": {"task": "deploy v2"},
            "questions": {
                "ok": {"type": "noul", "instructions": "Will it work?"},
                "how": {"type": "choice", "instructions": "Which?",
                        "criteria": {"a": null, "b": "the other"}},
                "risk": {"type": "score", "instructions": "How risky?",
                         "criteria": ["low", "high"]}
            },
            "model": "jev-latest"
        }"#;
        let req = AskRequest::parse(raw).unwrap();
        assert_eq!(req.questions.len(), 3);
        assert_eq!(req.model.as_deref(), Some("jev-latest"));
        // Order preserved, and every question converts to a typed value.
        let typed = req.typed_questions().unwrap();
        let ids: Vec<_> = typed.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, ["ok", "how", "risk"]);
    }

    #[test]
    fn rejects_unknown_top_level_fields() {
        let raw = r#"{"state": "s", "question": {}, "questions": {"q": {"type": "noul", "instructions": "i"}}}"#;
        let err = AskRequest::parse(raw).unwrap_err();
        assert!(err.to_string().contains("question"), "{err}");
    }

    #[test]
    fn rejects_empty_questions_and_empty_ids() {
        let no_questions = r#"{"state": "s", "questions": {}}"#;
        assert!(AskRequest::parse(no_questions).is_err());

        let empty_id =
            r#"{"state": "s", "questions": {" ": {"type": "noul", "instructions": "i"}}}"#;
        assert!(AskRequest::parse(empty_id).is_err());
    }

    #[test]
    fn rejects_malformed_questions_with_context() {
        let raw = r#"{"state": "s", "questions": {"risk": {"type": "scor", "instructions": "i"}}}"#;
        let err = AskRequest::parse(raw).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("risk"), "{msg}");
    }

    #[test]
    fn rejects_missing_state() {
        let raw = r#"{"questions": {"q": {"type": "noul", "instructions": "i"}}}"#;
        assert!(AskRequest::parse(raw).is_err());
    }
}
