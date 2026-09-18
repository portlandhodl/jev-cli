//! # jev-cli
//!
//! An agent-facing command line for [Jev](https://typesafe.ai/blog/introducing-system-one-models-and-jev),
//! a System One model — via the unofficial
//! [`jev-sdk`](https://crates.io/crates/jev-sdk).
//!
//! jev-cli lets LLM agents and shell scripts ask probabilistic questions
//! *before* acting:
//!
//! * **predict** — the probability that something is true or will succeed
//!   ("will this migration apply cleanly?"), optionally gating the exit code
//!   on a threshold.
//! * **choose** — which of several paths to take, with the full probability
//!   distribution and a confidence.
//! * **score** — a probability-weighted rating against ordered rubric levels.
//! * **ask** — full power: arbitrary typed questions as one JSON document
//!   in, one JSON document out.
//! * **models** — list the models available to the account.
//!
//! The crate is the engine; the `jev-cli` binary is a thin clap shell over
//! [`run`]. Output goes to stdout (`json` by default, `text` as a bare
//! value for shell capture), diagnostics to stderr, and the exit code
//! contract is: `0` ok, `1` gate not met (`--threshold` /
//! `--min-confidence`), `2` error.
//!
//! See `docs/usage.md` and `docs/output.md` for the full contract, and the
//! crate's `AGENTS.md` for instructions aimed at LLM callers.

pub mod config;
pub mod env;
pub mod error;
pub mod input;
pub mod output;
pub mod run;
pub mod spec;

pub use error::Error;
pub use input::{StateInput, MAX_STATE_BYTES};
pub use output::OutputFormat;
pub use run::{
    build_client, run_ask, run_choose, run_models, run_predict, run_score, AskOptions,
    ChooseOptions, Outcome, PredictOptions, ScoreOptions, EXIT_ERROR, EXIT_NOT_MET, EXIT_OK,
};
