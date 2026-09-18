//! `.env` file loading for the CLI.
//!
//! Variables from `.env` fill in the process environment *without*
//! overriding variables that are already set, so the precedence is:
//! explicit CLI options > real environment > `.env` file.
//!
//! This lives in the CLI crate on purpose: a library (jev-sdk) must not
//! mutate process-global state as a side effect.

use std::path::Path;

use crate::error::Error;

/// Default file looked up when no explicit path is given.
pub const DEFAULT_ENV_FILE: &str = ".env";

/// Variables a file (`.env`, `jev.conf`) may set. Deliberately an
/// allowlist: a planted `.env` in an untrusted directory must not be able
/// to redirect transport (`TYPESAFE_BASE_URL`) or influence any other
/// behavior — only carry credentials and the model choice. The base URL
/// must come from the real environment, or a cloned repo could exfiltrate
/// your API key.
pub(crate) const ALLOWED_FILE_VARS: &[&str] = &["TYPESAFE_API_KEY", "TYPESAFE_DEFAULT_MODEL"];

/// Parse a dotenv-style file down to the allowlisted assignments, without
/// touching the process environment. Pure, so tests never mutate state.
pub(crate) fn parse_allowlisted(path: &Path) -> Result<Vec<(String, String)>, Error> {
    let iter = dotenvy::from_path_iter(path).map_err(|e| sanitize_dotenv_error(path, &e))?;
    let mut vars = Vec::new();
    for item in iter {
        let (key, value) = item.map_err(|e| sanitize_dotenv_error(path, &e))?;
        if ALLOWED_FILE_VARS.contains(&key.as_str()) {
            vars.push((key, value));
        }
    }
    Ok(vars)
}

/// Render a dotenvy error without echoing file contents. `LineParse`'s
/// `Display` embeds the offending line verbatim, and these files may hold
/// credentials — report only the line index instead.
fn sanitize_dotenv_error(path: &Path, err: &dotenvy::Error) -> Error {
    match err {
        dotenvy::Error::LineParse(_, index) => Error::Usage(format!(
            "{}: malformed line at line index {index} \
             (line content suppressed: the file may hold credentials)",
            path.display()
        )),
        other => Error::Usage(format!("cannot load {}: {other}", path.display())),
    }
}

/// Apply assignments, never overriding the real environment. Anything in a
/// file loses to a variable that is already set.
pub(crate) fn apply_missing(vars: &[(String, String)]) {
    for (key, value) in vars {
        if std::env::var_os(key).is_none() {
            // set_var mutates process env; the CLI calls this before any
            // worker threads start.
            std::env::set_var(key, value);
        }
    }
}

/// Load environment variables from a `.env`-style file.
///
/// * `None` looks for `.env` in the current directory; a missing default
///   file is not an error (returns `Ok(())`).
/// * `Some(path)` is strict: any failure (missing file, parse error) is an
///   error.
///
/// Never logs the values it loads.
pub fn load_env_file(path: Option<&Path>) -> Result<(), Error> {
    let explicit = path.is_some();
    let path = path.unwrap_or_else(|| Path::new(DEFAULT_ENV_FILE));
    if !explicit && file_missing(path) {
        return Ok(());
    }
    let vars = parse_allowlisted(path)?;
    apply_missing(&vars);
    Ok(())
}

fn file_missing(path: &Path) -> bool {
    matches!(path.metadata(), Err(e) if e.kind() == std::io::ErrorKind::NotFound)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    fn temp_file(name: &str, contents: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("jev-cli-env-test-{name}"));
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(contents.as_bytes()).unwrap();
        path
    }

    #[test]
    fn missing_default_file_is_not_an_error() {
        let dir = std::env::temp_dir().join("jev-cli-env-test-emptydir");
        std::fs::create_dir_all(&dir).unwrap();
        let cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(&dir).unwrap();
        let result = load_env_file(None);
        std::env::set_current_dir(cwd).unwrap();
        assert!(result.is_ok());
    }

    #[test]
    fn explicit_missing_file_errors() {
        let result = load_env_file(Some(Path::new("/definitely/not/here.env")));
        assert!(result.is_err());
    }

    #[test]
    fn parse_errors_never_echo_line_content() {
        // dotenvy's LineParse Display embeds the offending line verbatim;
        // these files may hold credentials, so errors must carry only the
        // line index.
        let secret = "apikey_lonesecret_no_equals_sign";
        let path = temp_file("broken-secret", &format!("{secret}\n"));
        let err = load_env_file(Some(&path)).unwrap_err();
        let msg = err.to_string();
        assert!(!msg.contains(secret), "secret leaked into error: {msg}");
        assert!(msg.contains("line index"), "{msg}");
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn loads_variables_without_overriding_existing() {
        // One variable, exercised both ways: set in the real environment
        // (env wins), then unset (file value loads). Keeping to a single
        // variable avoids racing other tests that mutate the allowlisted
        // vars in this shared process.
        let path = temp_file(
            "basic",
            "TYPESAFE_DEFAULT_MODEL=\"jev-from-file\"\n# comment\n",
        );

        std::env::set_var("TYPESAFE_DEFAULT_MODEL", "jev-from-env");
        load_env_file(Some(&path)).unwrap();
        assert_eq!(
            std::env::var("TYPESAFE_DEFAULT_MODEL").unwrap(),
            "jev-from-env",
            "real environment must win over the file"
        );

        std::env::remove_var("TYPESAFE_DEFAULT_MODEL");
        load_env_file(Some(&path)).unwrap();
        assert_eq!(
            std::env::var("TYPESAFE_DEFAULT_MODEL").unwrap(),
            "jev-from-file",
            "file value should load when the env is unset"
        );

        std::env::remove_var("TYPESAFE_DEFAULT_MODEL");
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn base_url_cannot_be_set_from_env_file() {
        // A planted .env must never redirect transport — that would
        // exfiltrate the API key.
        let path = temp_file(
            "allowlist",
            "TYPESAFE_BASE_URL=https://evil.example\nTYPESAFE_API_KEY=planted-key\n",
        );
        std::env::remove_var("TYPESAFE_BASE_URL");
        std::env::remove_var("TYPESAFE_API_KEY");

        load_env_file(Some(&path)).unwrap();
        assert!(
            std::env::var_os("TYPESAFE_BASE_URL").is_none(),
            "TYPESAFE_BASE_URL must not load from .env"
        );
        assert_eq!(
            std::env::var("TYPESAFE_API_KEY").unwrap(),
            "planted-key",
            "credentials should load"
        );

        std::env::remove_var("TYPESAFE_API_KEY");
        std::fs::remove_file(path).ok();
    }
}
