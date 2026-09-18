//! Output rendering.
//!
//! Two formats, one contract:
//!
//! * **json** (default) — a full JSON document on stdout. This is what
//!   calling agents should parse.
//! * **text** — only the primary answer value on a single line (a
//!   probability, an option name, a score), for capture like
//!   `PROB=$(jev-cli predict ... --format text)`.
//!
//! `ask` always emits JSON: with several questions there is no single
//! primary value. Diagnostics never go to stdout.

use std::fmt;

use jev_sdk::{
    ChoiceAnswer, ListModelsResponse, NoulAnswer, ScoreAnswer, SystemOneResponse, Usage,
};
use serde_json::{json, Value};

use crate::error::Error;

/// Output format (`--format`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum OutputFormat {
    /// A full JSON document (default; what agents should parse).
    Json,
    /// Just the primary answer value on one line (for shell capture).
    Text,
}

impl fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            OutputFormat::Json => "json",
            OutputFormat::Text => "text",
        })
    }
}

/// Render a `predict` answer.
pub fn noul_output(
    format: OutputFormat,
    question: &str,
    answer: &NoulAnswer,
    model: &str,
    usage: &Usage,
    threshold: Option<f64>,
) -> String {
    match format {
        OutputFormat::Text => scalar_line(answer.noul),
        OutputFormat::Json => pretty(json!({
            "type": "noul",
            "question": question,
            "probability": answer.noul,
            "threshold": threshold,
            "pass": threshold.map(|t| answer.noul >= t),
            "model": model,
            "usage": usage,
        })),
    }
}

/// Render a `choose` answer.
pub fn choice_output(
    format: OutputFormat,
    question: &str,
    answer: &ChoiceAnswer,
    model: &str,
    usage: &Usage,
    min_confidence: Option<f64>,
) -> String {
    match format {
        OutputFormat::Text => format!("{}\n", answer.choice),
        OutputFormat::Json => pretty(json!({
            "type": "choice",
            "question": question,
            "choice": answer.choice,
            "probabilities": answer.probabilities,
            "confidence": answer.confidence,
            "min_confidence": min_confidence,
            "pass": min_confidence.map(|c| answer.confidence >= c),
            "model": model,
            "usage": usage,
        })),
    }
}

/// Render a `score` answer.
pub fn score_output(
    format: OutputFormat,
    question: &str,
    answer: &ScoreAnswer,
    model: &str,
    usage: &Usage,
) -> String {
    match format {
        OutputFormat::Text => scalar_line(answer.score),
        OutputFormat::Json => pretty(json!({
            "type": "score",
            "question": question,
            "score": answer.score,
            "legend": answer.legend,
            "probabilities": answer.probabilities,
            "confidence": answer.confidence,
            "model": model,
            "usage": usage,
        })),
    }
}

/// Render an `ask` response: the full response document, always JSON.
pub fn ask_output(response: &SystemOneResponse) -> Result<String, Error> {
    let value = serde_json::to_value(response)
        .map_err(|e| Error::Unexpected(format!("failed to serialize response: {e}")))?;
    Ok(pretty(value))
}

/// Render a `models` response.
pub fn models_output(format: OutputFormat, response: &ListModelsResponse) -> String {
    match format {
        OutputFormat::Text => {
            let mut out = String::new();
            for m in &response.models {
                out.push_str(&format!(
                    "{}\t{}\t{}\n",
                    m.name, m.release_date, m.description
                ));
            }
            out
        }
        OutputFormat::Json => {
            let value = serde_json::to_value(response).unwrap_or(Value::Null);
            pretty(value)
        }
    }
}

/// A bare scalar on one line, in shortest round-trip form.
fn scalar_line(value: f64) -> String {
    format!("{value}\n")
}

fn pretty(value: Value) -> String {
    // Values built here contain no non-finite floats worth failing over;
    // fall back to a null document rather than panicking if that changes.
    let rendered = serde_json::to_string_pretty(&value).unwrap_or_else(|_| "null".into());
    rendered + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;
    use jev_sdk::{ListModelsResponse, ModelCard};

    fn usage() -> Usage {
        Usage {
            input_tokens: 10,
            output_tokens: 4,
        }
    }

    #[test]
    fn noul_text_is_the_bare_probability() {
        let a = NoulAnswer { noul: 0.9 };
        assert_eq!(
            noul_output(OutputFormat::Text, "q?", &a, "m", &usage(), None),
            "0.9\n"
        );
    }

    #[test]
    fn noul_json_carries_threshold_and_pass() {
        let a = NoulAnswer { noul: 0.9 };
        let out = noul_output(
            OutputFormat::Json,
            "Will it work?",
            &a,
            "jev-1",
            &usage(),
            Some(0.8),
        );
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["type"], "noul");
        assert_eq!(v["probability"], 0.9);
        assert_eq!(v["threshold"], 0.8);
        assert_eq!(v["pass"], true);
        assert_eq!(v["model"], "jev-1");
        assert_eq!(v["usage"]["input_tokens"], 10);
    }

    #[test]
    fn noul_json_without_threshold_has_null_fields() {
        let a = NoulAnswer { noul: 0.1 };
        let out = noul_output(OutputFormat::Json, "q", &a, "m", &usage(), None);
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["threshold"], Value::Null);
        assert_eq!(v["pass"], Value::Null);
    }

    #[test]
    fn choice_text_is_the_bare_option_name() {
        let a = ChoiceAnswer {
            choice: "retry".into(),
            probabilities: [("retry".to_owned(), 0.9), ("abort".to_owned(), 0.1)]
                .into_iter()
                .collect(),
            confidence: 0.8,
        };
        assert_eq!(
            choice_output(OutputFormat::Text, "q?", &a, "m", &usage(), None),
            "retry\n"
        );
    }

    #[test]
    fn score_json_carries_legend_and_confidence() {
        let a = ScoreAnswer {
            score: 1.5,
            legend: [
                ("0".to_owned(), "low".to_owned()),
                ("1".to_owned(), "high".to_owned()),
            ]
            .into_iter()
            .collect(),
            probabilities: Default::default(),
            confidence: 0.7,
        };
        let out = score_output(OutputFormat::Json, "How risky?", &a, "m", &usage());
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["score"], 1.5);
        assert_eq!(v["legend"]["1"], "high");
        assert_eq!(v["confidence"], 0.7);
    }

    #[test]
    fn models_text_is_tab_separated_lines() {
        let resp = ListModelsResponse {
            models: vec![ModelCard {
                name: "jev-latest".into(),
                description: "The model".into(),
                release_date: "2026-01-01T00:00:00Z".into(),
            }],
        };
        assert_eq!(
            models_output(OutputFormat::Text, &resp),
            "jev-latest\t2026-01-01T00:00:00Z\tThe model\n"
        );
    }
}
