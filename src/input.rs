//! Resolving the evaluation *state* from command-line input.
//!
//! The state is the situation the model evaluates. It can be supplied as
//! inline text (`--state`), inline JSON (`--state-json`), or a file
//! (`--state-file`, with `-` for stdin). Reads are bounded: a runaway pipe
//! or a giant file must not exhaust memory.

use std::path::{Path, PathBuf};

use serde_json::Value;
use tokio::io::AsyncReadExt;

use crate::error::Error;

/// States larger than this are refused. Real situations are a few KiB; this
/// is generous headroom. Guards against an unbounded stdin or a hostile
/// file exhausting memory.
pub const MAX_STATE_BYTES: u64 = 16 * 1024 * 1024; // 16 MiB

/// How the state was supplied on the command line. Exactly one source must
/// be set (validated by [`StateInput::resolve`], and by clap for the CLI).
#[derive(Debug, Clone, Default)]
pub struct StateInput {
    /// Inline plain-text state (`--state`).
    pub state: Option<String>,
    /// Inline JSON state (`--state-json`).
    pub state_json: Option<String>,
    /// State read from a file, `-` for stdin (`--state-file`).
    pub state_file: Option<PathBuf>,
}

impl StateInput {
    /// Resolve the state to a JSON value ready for the API.
    pub async fn resolve(&self) -> Result<Value, Error> {
        let value = match (&self.state, &self.state_json, &self.state_file) {
            (Some(text), None, None) => Value::String(non_empty(text, "--state")?.to_owned()),
            (None, Some(raw), None) => {
                let value: Value = serde_json::from_str(raw)
                    .map_err(|e| Error::Usage(format!("--state-json is not valid JSON: {e}")))?;
                if let Value::String(s) = &value {
                    non_empty(s, "--state-json")?;
                }
                value
            }
            (None, None, Some(path)) => {
                let text = read_text_source(path, MAX_STATE_BYTES, "state").await?;
                Value::String(text)
            }
            (None, None, None) => {
                return Err(Error::Usage(
                    "no state provided: pass --state, --state-json, or --state-file \
                     (\"-\" reads stdin)"
                        .into(),
                ));
            }
            _ => {
                return Err(Error::Usage(
                    "choose exactly one of --state, --state-json, or --state-file".into(),
                ));
            }
        };
        Ok(value)
    }
}

/// Refuse empty states: evaluating nothing is always a mistake, and the
/// error message teaches the calling agent more than an API 422 would.
fn non_empty<'a>(text: &'a str, what: &str) -> Result<&'a str, Error> {
    if text.trim().is_empty() {
        return Err(Error::Usage(format!(
            "{what} is empty; the state must describe the situation to evaluate"
        )));
    }
    Ok(text)
}

/// Read UTF-8 text from a file (or stdin for `-`), capped at `limit` bytes;
/// anything past the cap is an error, not a silent truncation. `what` names
/// the input in error messages ("state", "request document", ...).
pub async fn read_text_source(path: &Path, limit: u64, what: &str) -> Result<String, Error> {
    let text = if path == Path::new("-") {
        read_limited(tokio::io::stdin(), limit).await?
    } else {
        read_limited(tokio::fs::File::open(path).await?, limit).await?
    };
    if text.trim().is_empty() {
        return Err(Error::Usage(format!("{what} is empty")));
    }
    Ok(text)
}

/// Read at most `limit` bytes from an async source.
async fn read_limited<R>(reader: R, limit: u64) -> Result<String, Error>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut buf = Vec::new();
    reader.take(limit + 1).read_to_end(&mut buf).await?;
    if buf.len() as u64 > limit {
        return Err(Error::Usage(format!(
            "input exceeds the {limit}-byte limit"
        )));
    }
    String::from_utf8(buf).map_err(|_| Error::Usage("input is not valid UTF-8 text".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn read_limited_accepts_up_to_the_cap() {
        let data: &[u8] = b"hello world";
        let text = read_limited(data, data.len() as u64).await.unwrap();
        assert_eq!(text, "hello world");
    }

    #[tokio::test]
    async fn read_limited_errors_past_the_cap() {
        let data: &[u8] = b"hello world";
        let err = read_limited(data, 5).await.unwrap_err();
        assert!(matches!(err, Error::Usage(_)));
        assert!(err.to_string().contains("limit"), "{err}");
    }

    #[tokio::test]
    async fn read_limited_rejects_non_utf8() {
        let data: &[u8] = &[0xff, 0xfe, 0x00];
        assert!(read_limited(data, 1024).await.is_err());
    }

    #[tokio::test]
    async fn resolve_requires_exactly_one_source() {
        assert!(StateInput::default().resolve().await.is_err());
        let both = StateInput {
            state: Some("a".into()),
            state_json: Some(r#"{"b":1}"#.into()),
            state_file: None,
        };
        assert!(both.resolve().await.is_err());
    }

    #[tokio::test]
    async fn resolve_inline_text_and_json() {
        let text = StateInput {
            state: Some("the situation".into()),
            ..Default::default()
        };
        assert_eq!(
            text.resolve().await.unwrap(),
            Value::String("the situation".into())
        );

        let json = StateInput {
            state_json: Some(r#"{"task":"deploy","risk":"low"}"#.into()),
            ..Default::default()
        };
        assert_eq!(json.resolve().await.unwrap()["task"], "deploy");
    }

    #[tokio::test]
    async fn resolve_rejects_empty_and_invalid() {
        let empty = StateInput {
            state: Some("   ".into()),
            ..Default::default()
        };
        assert!(empty.resolve().await.is_err());

        let bad_json = StateInput {
            state_json: Some("{nope".into()),
            ..Default::default()
        };
        assert!(bad_json.resolve().await.is_err());
    }

    #[tokio::test]
    async fn resolve_reads_files() {
        let path = std::env::temp_dir().join("jev-cli-input-test-state.txt");
        std::fs::write(&path, "file state").unwrap();
        let input = StateInput {
            state_file: Some(path.clone()),
            ..Default::default()
        };
        assert_eq!(
            input.resolve().await.unwrap(),
            Value::String("file state".into())
        );
        std::fs::remove_file(path).ok();
    }
}
