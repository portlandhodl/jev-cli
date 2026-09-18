//! User-level configuration: `~/.jevcli/jev.conf`.
//!
//! Holds long-lived settings so the API key does not need to be exported in
//! every shell. Same dotenv-style `KEY=value` format and the same allowlist
//! as `.env` (`TYPESAFE_API_KEY`, `TYPESAFE_DEFAULT_MODEL`) — transport
//! settings like `TYPESAFE_BASE_URL` are never read from files, so neither
//! a planted `.env` nor a careless paste into the config can redirect the
//! key to another host.
//!
//! Precedence: real environment > `./.env` (project) > `~/.jevcli/jev.conf`
//! (user). Files only fill variables that are not already set, and the
//! project file is loaded first.
//!
//! The file holds a credential: keep it `chmod 600`. On unix a warning is
//! emitted when any group/other permission bits are set.

use std::path::{Path, PathBuf};

use crate::env;
use crate::error::Error;

/// The directory inside the user's home that holds the config file.
pub const DEFAULT_CONFIG_DIR: &str = ".jevcli";
/// The config file name.
pub const DEFAULT_CONFIG_NAME: &str = "jev.conf";

/// The default config path: `$HOME/.jevcli/jev.conf` (falling back to
/// `%USERPROFILE%`). `None` when no home directory is known.
pub fn user_config_path() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .map(|home| config_path_in(&home))
}

/// `<home>/.jevcli/jev.conf` — split out so tests don't touch real paths.
pub fn config_path_in(home: &Path) -> PathBuf {
    home.join(DEFAULT_CONFIG_DIR).join(DEFAULT_CONFIG_NAME)
}

/// What happened during a load. Only ever carries diagnostics — never
/// loaded values.
#[derive(Debug, Default)]
pub struct LoadOutcome {
    /// Non-fatal problems worth surfacing on stderr (e.g. loose
    /// permissions).
    pub warnings: Vec<String>,
}

/// Load the user config file into the process environment (missing
/// variables only).
///
/// * `None` uses [`user_config_path`]; a missing default file (or no known
///   home directory) is fine and loads nothing.
/// * `Some(path)` is strict: any failure (missing file, parse error) is an
///   error.
pub fn load_config_file(path: Option<&Path>) -> Result<LoadOutcome, Error> {
    let explicit = path.is_some();
    let owned;
    let path = match path {
        Some(p) => p,
        None => match user_config_path() {
            Some(p) => {
                owned = p;
                &owned
            }
            // No home directory known: nothing to load, not an error.
            None => return Ok(LoadOutcome::default()),
        },
    };
    if !explicit && !path.exists() {
        return Ok(LoadOutcome::default());
    }
    let vars = env::parse_allowlisted(path)?;
    env::apply_missing(&vars);
    let mut outcome = LoadOutcome::default();
    if let Some(warning) = permissions_warning(path) {
        outcome.warnings.push(warning);
    }
    Ok(outcome)
}

/// Warn when a credential file is readable by group or others (unix only).
#[cfg(unix)]
fn permissions_warning(path: &Path) -> Option<String> {
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(path).ok()?.permissions().mode() & 0o777;
    if mode & 0o077 == 0 {
        return None;
    }
    Some(format!(
        "{}: permissions {mode:04o} are too open for a file holding an API key; \
         run: chmod 600 {}",
        path.display(),
        path.display()
    ))
}

/// No permission model check on non-unix platforms.
#[cfg(not(unix))]
fn permissions_warning(_path: &Path) -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    fn temp_file(name: &str, contents: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("jev-cli-config-test-{name}"));
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(contents.as_bytes()).unwrap();
        path
    }

    #[test]
    fn config_path_joins_home() {
        assert_eq!(
            config_path_in(Path::new("/home/u")),
            PathBuf::from("/home/u/.jevcli/jev.conf")
        );
    }

    #[test]
    fn explicit_missing_file_errors() {
        assert!(load_config_file(Some(Path::new("/definitely/not/here.conf"))).is_err());
    }

    #[test]
    fn loads_allowlisted_and_ignores_the_rest() {
        // TYPESAFE_BASE_URL must never load from a file: that would let a
        // file redirect the API key to another host. (Regression guard; the
        // allowlist itself lives in env.rs.)
        let path = temp_file(
            "allowlist",
            "TYPESAFE_BASE_URL=https://evil.example\nTYPESAFE_DEFAULT_MODEL=jev-x\n",
        );
        let vars = env::parse_allowlisted(&path).unwrap();
        assert_eq!(
            vars,
            vec![("TYPESAFE_DEFAULT_MODEL".to_owned(), "jev-x".to_owned())]
        );
        std::fs::remove_file(path).ok();
    }

    #[cfg(unix)]
    #[test]
    fn warns_on_loose_permissions_only() {
        use std::os::unix::fs::PermissionsExt;

        // No allowlisted vars in this file: loading it must not mutate the
        // process environment (tests run multi-threaded in one process).
        let path = temp_file("perms", "UNRELATED_SETTING=1\n");
        let perms = |m: u32| std::fs::set_permissions(&path, std::fs::Permissions::from_mode(m));

        perms(0o644).unwrap();
        let outcome = load_config_file(Some(&path)).unwrap();
        assert_eq!(outcome.warnings.len(), 1);
        assert!(outcome.warnings[0].contains("chmod 600"), "{outcome:?}");

        perms(0o600).unwrap();
        let outcome = load_config_file(Some(&path)).unwrap();
        assert!(outcome.warnings.is_empty(), "{outcome:?}");

        std::fs::remove_file(path).ok();
    }
}
