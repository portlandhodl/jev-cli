//! Command executors: validated options in, an [`Outcome`] out.
//!
//! Everything here is network-aware but terminal-free: functions return the
//! text to print and the exit code to use, so tests can drive them against
//! a mock server. `main` owns stdout/stderr and process exit.

use std::path::PathBuf;

use jev_sdk::{Choice, Noul, NoulCriteria, Question, Score, TypeSafeClient};

use crate::error::Error;
use crate::input::{read_text_source, StateInput};
use crate::output::{self, OutputFormat};
use crate::spec::{AskRequest, MAX_REQUEST_BYTES};

/// The command succeeded.
pub const EXIT_OK: u8 = 0;
/// `predict`/`choose`: the answer fell below `--threshold` /
/// `--min-confidence`. The answer still prints; only the code differs.
pub const EXIT_NOT_MET: u8 = 1;
/// Tool, input, or API error. Nothing is printed to stdout.
pub const EXIT_ERROR: u8 = 2;

/// What a run produced: text for stdout and the exit code to exit with.
#[derive(Debug)]
pub struct Outcome {
    /// Text to print to stdout (already newline-terminated).
    pub stdout: String,
    /// Process exit code (`0` ok, `1` gate not met).
    pub code: u8,
}

/// Build the API client, honoring `--model` over `TYPESAFE_DEFAULT_MODEL`.
pub fn build_client(model: Option<&str>) -> Result<TypeSafeClient, Error> {
    let mut builder = TypeSafeClient::builder();
    if let Some(model) = model {
        builder = builder.model(model);
    }
    Ok(builder.build()?)
}

/// `jev-cli predict`: probability that a yes/no statement about the state
/// is true ("will this deploy succeed?", "is this input safe?").
#[derive(Debug)]
pub struct PredictOptions {
    /// The yes/no question to evaluate.
    pub question: String,
    /// What a "yes" means (calibration hint).
    pub yes: Option<String>,
    /// What a "no" means (calibration hint).
    pub no: Option<String>,
    /// Exit code gate: exit 1 when the probability is below this.
    pub threshold: Option<f64>,
    /// The situation to evaluate.
    pub state: StateInput,
    /// Output format.
    pub format: OutputFormat,
    /// Model override.
    pub model: Option<String>,
}

impl PredictOptions {
    /// Check local inputs before any network or credential work.
    pub fn validate(&self) -> Result<(), Error> {
        if self.question.trim().is_empty() {
            return Err(Error::Usage("--question must not be empty".into()));
        }
        validate_probability(self.threshold, "--threshold")?;
        Ok(())
    }
}

/// Run `predict`.
pub async fn run_predict(client: &TypeSafeClient, opts: &PredictOptions) -> Result<Outcome, Error> {
    opts.validate()?;
    let state = opts.state.resolve().await?;

    let mut question = Noul::new(opts.question.clone());
    if opts.yes.is_some() || opts.no.is_some() {
        question = question.criteria(NoulCriteria {
            yes: opts.yes.clone(),
            no: opts.no.clone(),
        });
    }

    let model = opts.model.as_deref().unwrap_or_else(|| client.model());
    let response = client
        .system_one_with_model(model, state, [("outcome", Question::from(question))])
        .await?;
    let answer = response
        .noul("outcome")
        .ok_or_else(|| Error::Unexpected("missing noul answer for \"outcome\"".into()))?;

    let code = match opts.threshold {
        Some(t) if answer.noul < t => EXIT_NOT_MET,
        _ => EXIT_OK,
    };
    let stdout = output::noul_output(
        opts.format,
        &opts.question,
        answer,
        &response.model,
        &response.usage,
        opts.threshold,
    );
    Ok(Outcome { stdout, code })
}

/// `jev-cli choose`: pick the best path among several options.
#[derive(Debug)]
pub struct ChooseOptions {
    /// What to decide.
    pub question: String,
    /// Raw option specs: `name` or `name=description`.
    pub options: Vec<String>,
    /// Exit code gate: exit 1 when the answer's confidence is below this.
    pub min_confidence: Option<f64>,
    /// The situation to evaluate.
    pub state: StateInput,
    /// Output format.
    pub format: OutputFormat,
    /// Model override.
    pub model: Option<String>,
}

/// A parsed `--option` value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptionSpec {
    /// The option name (what the answer's `choice` will be).
    pub name: String,
    /// An optional description sharpening the option's meaning.
    pub description: Option<String>,
}

/// Parse `name[=description]` option specs: at least two, non-empty,
/// de-duplicated. `=` splits on the first occurrence; an empty description
/// after `=` is treated as absent.
pub fn parse_option_specs(raw: &[String]) -> Result<Vec<OptionSpec>, Error> {
    if raw.len() < 2 {
        return Err(Error::Usage(
            "choose needs at least two --option values".into(),
        ));
    }
    let mut seen = std::collections::HashSet::new();
    let mut specs = Vec::with_capacity(raw.len());
    for spec in raw {
        let (name, description) = match spec.split_once('=') {
            Some((n, d)) => {
                let d = d.trim();
                (n.trim(), (!d.is_empty()).then(|| d.to_owned()))
            }
            None => (spec.trim(), None),
        };
        if name.is_empty() {
            return Err(Error::Usage(
                "option names must not be empty (use --option NAME or --option NAME=DESCRIPTION)"
                    .into(),
            ));
        }
        if !seen.insert(name.to_owned()) {
            return Err(Error::Usage(format!("duplicate option {name:?}")));
        }
        specs.push(OptionSpec {
            name: name.to_owned(),
            description,
        });
    }
    Ok(specs)
}

impl ChooseOptions {
    /// Check local inputs; on success returns the parsed options.
    pub fn validate(&self) -> Result<Vec<OptionSpec>, Error> {
        if self.question.trim().is_empty() {
            return Err(Error::Usage("--question must not be empty".into()));
        }
        validate_probability(self.min_confidence, "--min-confidence")?;
        parse_option_specs(&self.options)
    }
}

/// Run `choose`.
pub async fn run_choose(client: &TypeSafeClient, opts: &ChooseOptions) -> Result<Outcome, Error> {
    let specs = opts.validate()?;
    let state = opts.state.resolve().await?;

    let question = Choice::new(
        opts.question.clone(),
        specs
            .iter()
            .map(|s| (s.name.clone(), s.description.clone())),
    );

    let model = opts.model.as_deref().unwrap_or_else(|| client.model());
    let response = client
        .system_one_with_model(model, state, [("choice", Question::from(question))])
        .await?;
    let answer = response
        .choice("choice")
        .ok_or_else(|| Error::Unexpected("missing choice answer for \"choice\"".into()))?;

    let code = match opts.min_confidence {
        Some(c) if answer.confidence < c => EXIT_NOT_MET,
        _ => EXIT_OK,
    };
    let stdout = output::choice_output(
        opts.format,
        &opts.question,
        answer,
        &response.model,
        &response.usage,
        opts.min_confidence,
    );
    Ok(Outcome { stdout, code })
}

/// `jev-cli score`: rate the state against ordered rubric levels.
#[derive(Debug)]
pub struct ScoreOptions {
    /// What to rate.
    pub question: String,
    /// Ordered level descriptions, lowest first (at least two).
    pub levels: Vec<String>,
    /// The situation to evaluate.
    pub state: StateInput,
    /// Output format.
    pub format: OutputFormat,
    /// Model override.
    pub model: Option<String>,
}

impl ScoreOptions {
    /// Check local inputs before any network or credential work.
    pub fn validate(&self) -> Result<(), Error> {
        if self.question.trim().is_empty() {
            return Err(Error::Usage("--question must not be empty".into()));
        }
        if self.levels.len() < 2 {
            return Err(Error::Usage(
                "score needs at least two --level values, ordered lowest to highest".into(),
            ));
        }
        if self.levels.iter().any(|l| l.trim().is_empty()) {
            return Err(Error::Usage("--level values must not be empty".into()));
        }
        Ok(())
    }
}

/// Run `score`.
pub async fn run_score(client: &TypeSafeClient, opts: &ScoreOptions) -> Result<Outcome, Error> {
    opts.validate()?;
    let state = opts.state.resolve().await?;

    let question = Score::new(opts.question.clone(), opts.levels.clone());

    let model = opts.model.as_deref().unwrap_or_else(|| client.model());
    let response = client
        .system_one_with_model(model, state, [("score", Question::from(question))])
        .await?;
    let answer = response
        .score("score")
        .ok_or_else(|| Error::Unexpected("missing score answer for \"score\"".into()))?;

    let stdout = output::score_output(
        opts.format,
        &opts.question,
        answer,
        &response.model,
        &response.usage,
    );
    Ok(Outcome {
        stdout,
        code: EXIT_OK,
    })
}

/// `jev-cli ask`: a full request document (arbitrary typed questions) from
/// a JSON file or stdin.
#[derive(Debug)]
pub struct AskOptions {
    /// Path to the request document; `-` reads stdin.
    pub request: PathBuf,
    /// Output format (note: `ask` always emits a full JSON document).
    pub format: OutputFormat,
    /// Model override; wins over the document's `"model"`.
    pub model: Option<String>,
}

/// Run `ask`.
pub async fn run_ask(client: &TypeSafeClient, opts: &AskOptions) -> Result<Outcome, Error> {
    let text = read_text_source(&opts.request, MAX_REQUEST_BYTES, "request document").await?;
    let request = AskRequest::parse(&text)?;

    let model = opts
        .model
        .as_deref()
        .or(request.model.as_deref())
        .unwrap_or_else(|| client.model());
    let response = client
        .system_one_with_model(model, request.state.clone(), request.typed_questions()?)
        .await?;

    Ok(Outcome {
        stdout: output::ask_output(&response)?,
        code: EXIT_OK,
    })
}

/// Run `models`: list the models available to the account.
pub async fn run_models(client: &TypeSafeClient, format: OutputFormat) -> Result<Outcome, Error> {
    let models = client.list_models().await?;
    Ok(Outcome {
        stdout: output::models_output(format, &models),
        code: EXIT_OK,
    })
}

/// A gate argument must be a finite probability in [0, 1].
fn validate_probability(value: Option<f64>, flag: &str) -> Result<(), Error> {
    if let Some(v) = value {
        if !v.is_finite() || !(0.0..=1.0).contains(&v) {
            return Err(Error::Usage(format!(
                "{flag} must be a finite number in [0, 1], got {v}"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn option_specs_parse_names_and_descriptions() {
        let specs = parse_option_specs(&[
            "retry".to_owned(),
            "fallback=Use the cached response".to_owned(),
            "abort=".to_owned(), // empty description means none
            "explain=Split on the FIRST '=' only: a=b".to_owned(),
        ])
        .unwrap();
        assert_eq!(
            specs,
            vec![
                OptionSpec {
                    name: "retry".into(),
                    description: None
                },
                OptionSpec {
                    name: "fallback".into(),
                    description: Some("Use the cached response".into())
                },
                OptionSpec {
                    name: "abort".into(),
                    description: None
                },
                OptionSpec {
                    name: "explain".into(),
                    description: Some("Split on the FIRST '=' only: a=b".into())
                },
            ]
        );
    }

    #[test]
    fn option_specs_reject_too_few_empty_and_duplicate() {
        assert!(parse_option_specs(&["only".to_owned()]).is_err());
        assert!(parse_option_specs(&["a".to_owned(), "  ".to_owned()]).is_err());
        assert!(parse_option_specs(&["a".to_owned(), "a=again".to_owned()]).is_err());
        assert!(parse_option_specs(&["=no-name".to_owned(), "b".to_owned()]).is_err());
    }

    #[test]
    fn gates_must_be_probabilities() {
        assert!(validate_probability(Some(0.5), "--x").is_ok());
        assert!(validate_probability(Some(0.0), "--x").is_ok());
        assert!(validate_probability(Some(1.0), "--x").is_ok());
        assert!(validate_probability(Some(-0.1), "--x").is_err());
        assert!(validate_probability(Some(1.1), "--x").is_err());
        assert!(validate_probability(Some(f64::NAN), "--x").is_err());
        assert!(validate_probability(Some(f64::INFINITY), "--x").is_err());
        assert!(validate_probability(None, "--x").is_ok());
    }

    #[test]
    fn predict_requires_a_question() {
        let opts = PredictOptions {
            question: "   ".into(),
            yes: None,
            no: None,
            threshold: None,
            state: StateInput::default(),
            format: OutputFormat::Json,
            model: None,
        };
        assert!(opts.validate().is_err());
    }
}
