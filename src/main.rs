//! jev-cli command-line interface.
//!
//! Ask Jev probabilistic questions before acting: outcome prediction, path
//! choice, scoring. Built to be called by LLM agents and shell scripts.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};

use jev_cli::run::{self, AskOptions, ChooseOptions, Outcome, PredictOptions, ScoreOptions};
use jev_cli::{Error, OutputFormat, StateInput};

const EXAMPLES: &str = "\
Examples:
  # Will this action succeed? (prints a probability, 0..1)
  jev-cli predict \\
      --state \"Deploying v2.3 to prod; tests pass; canary is healthy\" \\
      --question \"Will the deploy complete without errors?\"

  # Gate a script on the answer: exit 1 below the threshold
  jev-cli predict --state \"$CONTEXT\" \\
      --question \"Is this change safe to auto-apply?\" --threshold 0.8 \\
      && ./apply.sh

  # Which path should an agent take?
  jev-cli choose --state \"$CONTEXT\" \\
      --question \"Which approach is most likely to succeed?\" \\
      --option \"retry=Retry the failed request with backoff\" \\
      --option \"fallback=Use the cached response\" \\
      --option \"abort=Stop and ask the user\"

  # Rate a situation against an ordered rubric
  jev-cli score --state \"$CONTEXT\" --question \"How risky is this plan?\" \\
      --level trivial --level moderate --level severe

  # Full power: one JSON document in, one JSON document out
  echo '{\"state\": \"...\", \"questions\": {\"ok\": {\"type\": \"noul\", \"instructions\": \"Will it work?\"}}}' \\
      | jev-cli ask -

  # Capture just the value in a shell variable
  PROB=$(jev-cli predict --format text --state \"$CONTEXT\" --question \"...\")

Exit codes: 0 ok | 1 gate not met (--threshold / --min-confidence) | 2 error.
Docs: docs/usage.md and docs/output.md; man page: docs/jev-cli.1";

/// Ask Jev probabilistic questions before acting: outcome prediction, path
/// choice, scoring. Built for LLM agents and shell scripts.
///
/// Configuration, lowest precedence first: ~/.jevcli/jev.conf (user config,
/// holds the API key) < ./.env (project) < real environment variables <
/// CLI flags. Files only ever carry TYPESAFE_API_KEY and
/// TYPESAFE_DEFAULT_MODEL — TYPESAFE_BASE_URL is never read from files and
/// must come from the real environment.
#[derive(Parser)]
#[command(name = "jev-cli", version, about, long_about = None, after_long_help = EXAMPLES)]
struct Cli {
    /// Load environment from this file instead of ./.env ("none" disables).
    #[arg(long, global = true, value_name = "FILE")]
    env_file: Option<String>,
    /// Load user config from this file instead of ~/.jevcli/jev.conf
    /// ("none" disables).
    #[arg(long, global = true, value_name = "FILE")]
    config_file: Option<String>,
    /// Output format: a full JSON document, or the bare answer value.
    #[arg(long, global = true, value_enum, default_value_t = OutputFormat::Json)]
    format: OutputFormat,
    /// Model override (default: jev-latest or TYPESAFE_DEFAULT_MODEL).
    #[arg(long, global = true, value_name = "MODEL")]
    model: Option<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Probability that something is true or will happen ("will this
    /// succeed?"). With --threshold, exits 1 when the probability is below
    /// it — shell-gate friendly.
    #[command(visible_alias = "will")]
    Predict(PredictArgs),
    /// Pick the best path among several options; answers with the chosen
    /// option, the full probability distribution, and a confidence.
    #[command(visible_alias = "pick")]
    Choose(ChooseArgs),
    /// Rate the state against ordered rubric levels (e.g. trivial <
    /// moderate < severe).
    #[command(visible_alias = "rate")]
    Score(ScoreArgs),
    /// Full power: read a request document ({"state": ..., "questions":
    /// {...}, "model"?}) from a JSON file or stdin ("-"), print the full
    /// response as JSON.
    Ask(AskArgs),
    /// List the models available to the account.
    Models,
}

/// The situation the model evaluates. Exactly one source is required.
#[derive(Debug, Clone, Args)]
struct StateArgs {
    /// The situation as plain text.
    #[arg(long, value_name = "TEXT", conflicts_with_all = ["state_json", "state_file"])]
    state: Option<String>,
    /// The situation as a JSON value (object, array, string, ...).
    #[arg(long, value_name = "JSON")]
    state_json: Option<String>,
    /// Read the state from a file; "-" reads stdin.
    #[arg(long, value_name = "FILE")]
    state_file: Option<PathBuf>,
}

impl From<StateArgs> for StateInput {
    fn from(args: StateArgs) -> Self {
        StateInput {
            state: args.state,
            state_json: args.state_json,
            state_file: args.state_file,
        }
    }
}

#[derive(Debug, Args)]
struct PredictArgs {
    /// The yes/no question to evaluate (e.g. "Will this deploy succeed
    /// without downtime?").
    #[arg(long, value_name = "TEXT")]
    question: String,
    /// What "yes" means (calibration hint for the model).
    #[arg(long, value_name = "TEXT")]
    yes: Option<String>,
    /// What "no" means (calibration hint for the model).
    #[arg(long, value_name = "TEXT")]
    no: Option<String>,
    /// Exit 1 when the probability is below T (a number in [0, 1]). The
    /// answer still prints; only the exit code differs.
    #[arg(long, value_name = "T")]
    threshold: Option<f64>,
    #[command(flatten)]
    state: StateArgs,
}

#[derive(Debug, Args)]
struct ChooseArgs {
    /// What to decide (e.g. "Which approach is most likely to succeed?").
    #[arg(long, value_name = "TEXT")]
    question: String,
    /// A candidate path: "name" or "name=description". Repeatable; at least
    /// two required.
    #[arg(long = "option", value_name = "NAME[=DESC]", required = true)]
    options: Vec<String>,
    /// Exit 1 when the answer's confidence is below C (a number in
    /// [0, 1]): the model abstains rather than guessing.
    #[arg(long, value_name = "C")]
    min_confidence: Option<f64>,
    #[command(flatten)]
    state: StateArgs,
}

#[derive(Debug, Args)]
struct ScoreArgs {
    /// What to rate (e.g. "How risky is this plan?").
    #[arg(long, value_name = "TEXT")]
    question: String,
    /// An ordered level description, lowest first. Repeatable; at least two
    /// required.
    #[arg(long = "level", value_name = "TEXT", required = true)]
    levels: Vec<String>,
    #[command(flatten)]
    state: StateArgs,
}

#[derive(Debug, Args)]
struct AskArgs {
    /// Path to the JSON request document; "-" reads stdin. Shape:
    /// {"state": <any JSON>, "questions": {"id": {"type": "noul"|"choice"|
    /// "score", ...}}, "model": "..." (optional)}.
    #[arg(value_name = "FILE")]
    request: PathBuf,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    // Load configuration before anything reads the environment. The project
    // .env loads first so it wins over the user config; the real environment
    // always wins over both (files only fill unset variables). A missing
    // default file is fine; a broken one warns; an explicitly requested file
    // is a hard error. "none" disables a file entirely.
    let env_path = match cli.env_file.as_deref() {
        Some("none") => None,
        other => Some(other.map(PathBuf::from)),
    };
    if let Some(path) = env_path {
        let explicit = path.is_some();
        match jev_cli::env::load_env_file(path.as_deref()) {
            Ok(()) => {}
            Err(e) if explicit => return fail(&e),
            Err(e) => eprintln!("jev-cli: warning: {e}"),
        }
    }
    let config_path = match cli.config_file.as_deref() {
        Some("none") => None,
        other => Some(other.map(PathBuf::from)),
    };
    if let Some(path) = config_path {
        let explicit = path.is_some();
        match jev_cli::config::load_config_file(path.as_deref()) {
            Ok(outcome) => {
                for warning in &outcome.warnings {
                    eprintln!("jev-cli: warning: {warning}");
                }
            }
            Err(e) if explicit => return fail(&e),
            Err(e) => eprintln!("jev-cli: warning: {e}"),
        }
    }

    match dispatch(&cli).await {
        Ok(outcome) => {
            print!("{}", outcome.stdout);
            ExitCode::from(outcome.code)
        }
        Err(e) => fail(&e),
    }
}

/// Print an error and exit with the tool error code.
fn fail(error: &jev_cli::Error) -> ExitCode {
    eprintln!("jev-cli: error: {error}");
    ExitCode::from(run::EXIT_ERROR)
}

/// Validate local inputs first, so usage errors fire even when no API key
/// is configured yet; build the client only for valid invocations.
async fn dispatch(cli: &Cli) -> Result<Outcome, Error> {
    let client = |cli: &Cli| run::build_client(cli.model.as_deref());
    match &cli.command {
        Command::Predict(args) => {
            let opts = PredictOptions {
                question: args.question.clone(),
                yes: args.yes.clone(),
                no: args.no.clone(),
                threshold: args.threshold,
                state: args.state.clone().into(),
                format: cli.format,
                model: cli.model.clone(),
            };
            opts.validate()?;
            run::run_predict(&client(cli)?, &opts).await
        }
        Command::Choose(args) => {
            let opts = ChooseOptions {
                question: args.question.clone(),
                options: args.options.clone(),
                min_confidence: args.min_confidence,
                state: args.state.clone().into(),
                format: cli.format,
                model: cli.model.clone(),
            };
            opts.validate()?;
            run::run_choose(&client(cli)?, &opts).await
        }
        Command::Score(args) => {
            let opts = ScoreOptions {
                question: args.question.clone(),
                levels: args.levels.clone(),
                state: args.state.clone().into(),
                format: cli.format,
                model: cli.model.clone(),
            };
            opts.validate()?;
            run::run_score(&client(cli)?, &opts).await
        }
        Command::Ask(args) => {
            run::run_ask(
                &client(cli)?,
                &AskOptions {
                    request: args.request.clone(),
                    format: cli.format,
                    model: cli.model.clone(),
                },
            )
            .await
        }
        Command::Models => run::run_models(&client(cli)?, cli.format).await,
    }
}
