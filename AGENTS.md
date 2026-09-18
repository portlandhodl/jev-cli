# AGENTS.md — how to call `jev-cli`


Instructions for LLM agents using this tool. jev-cli answers **typed
probabilistic questions** about a situation you describe, so you can decide
*before* acting: predict an outcome, or pick among candidate paths. Answers
are calibrated probabilities from a System One model (Jev) — fast, cheap,
structured; no generated text.

## When to call it

- **Before an irreversible or costly action** — `predict` the probability
  that it succeeds / is safe / is permitted; gate on `--threshold`.
- **At a branch point** — `choose` among the paths you are considering
  instead of guessing; check `confidence` and abstain on low values.
- **To grade a situation** — `score` it on an ordered rubric you define.
- **For anything richer** — `ask` sends arbitrary typed questions in one
  JSON document and returns one JSON document.

Do **not** call it for facts you can look up, questions with no uncertainty,
or as a substitute for doing the action's real validation.

## Calling contract

1. **stdout carries only the answer.** Parse stdout; never parse stderr.
   Default output is a single JSON document (pretty-printed). With
   `--format text`, stdout is exactly one line: the bare value (a
   probability, an option name, a score) for shell capture.
2. **Exit codes:** `0` = answer delivered (and gate met); `1` = answer
   delivered but the gate was *not* met (below `--threshold` /
   `--min-confidence`); `2` = error (nothing on stdout; stderr has
   `jev-cli: error: ...`). Treat `1` as "no" with a valid answer, `2` as
   "no answer" — do not proceed on `2`.
3. **Every call needs a state**: exactly one of `--state TEXT`,
   `--state-json '<json>'`, `--state-file FILE` (`-` = stdin). The state is
   the *only* thing the model sees: make it self-contained — goal, relevant
   facts, constraints, what you intend to do. A few sentences beat a data
   dump.
4. **Quote carefully.** Wrap `--state` and `--question` in double quotes in
   shell; prefer `--state-json`/`ask -` with structured input over
   interpolating text into strings.
5. **Never put secrets in the state.** The state is sent to the TypeSafe
   API verbatim — no redaction. Strip tokens, keys, and personal data first.

## Command templates

```sh
# Go/no-go before acting; exit 1 = below threshold
jev-cli predict --state "$STATE" \
    --question "Will <the action> succeed without errors?" \
    --threshold 0.8

# Pick a path; exit 1 = model is not confident enough (abstain, ask the user)
jev-cli choose --state "$STATE" \
    --question "Which approach is most likely to succeed?" \
    --option "NAME1=one-line description" --option "NAME2=..." \
    --min-confidence 0.6

# Rubric rating (levels ordered lowest → highest)
jev-cli score --state "$STATE" --question "How risky is this plan?" \
    --level trivial --level moderate --level severe

# Batch several questions in one API call (pipe a request document)
printf '%s' '{"state": "...", "questions": {
  "will_succeed": {"type": "noul", "instructions": "..."},
  "approach": {"type": "choice", "instructions": "...",
               "criteria": {"a": "desc", "b": null}},
  "risk": {"type": "score", "instructions": "...", "criteria": ["low", "high"]}
}}' | jev-cli ask -
```

## Output shape (JSON, default)

```jsonc
// predict
{"type":"noul","question":"...","probability":0.97,
 "threshold":0.8,"pass":true,"model":"jev-1.13.0",
 "usage":{"input_tokens":282,"output_tokens":21}}
// choose
{"type":"choice","question":"...","choice":"retry",
 "probabilities":{"retry":0.9,"abort":0.1},"confidence":0.88,
 "min_confidence":0.6,"pass":true,"model":"...","usage":{...}}
// score
{"type":"score","question":"...","score":1.9,
 "legend":{"0":"trivial","1":"moderate","2":"severe"},
 "probabilities":{...},"confidence":0.9,"model":"...","usage":{...}}
// ask → {"model":"...","answers":{"<id>":<typed answer>, ...},"usage":{...}}
// models → {"models":[{"name":"...","description":"...","release_date":"..."}]}
```

`pass`/`threshold`/`min_confidence` are `null` when no gate was given.
`score` is probability-weighted and can land between levels; `legend` maps
level indices to your descriptions.

## Practical notes

- Prefer named, `snake_case` option names for `choose` — the answer's
  `choice` echoes the name verbatim; `--format text` prints just it.
- Sharpen calibration with `--yes`/`--no` on `predict` and
  `name=description` on `choose` when option names alone are ambiguous.
- `--model` overrides per call; `jev-cli models` lists what your account
  can use. The response `model` is the *resolved* version, not your alias.
- Each call is one API round-trip (sub-second typical, billed in tokens —
  see `usage` in JSON output). Batch related questions with `ask`.
- If `jev-cli: error: ... missing API key` appears, the key is not
  configured: it belongs in `~/.jevcli/jev.conf` (`chmod 600`), `.env`, or
  `TYPESAFE_API_KEY` in the environment. Never ask the user to paste a key
  into a command line.
- Full contract: `docs/usage.md`, `docs/output.md`, and `man jev-cli`
  (`man -l docs/jev-cli.1` from the repo).

# Development

Guidance for agents *editing this repository* (as opposed to calling the
tool).

## Layout

```
Cargo.toml            # the jev-cli crate (binary + library)
src/
├── lib.rs            # crate docs + public re-exports
├── main.rs           # thin clap shell; dispatch() → run::* → Outcome
├── env.rs            # dotenvy parsing: parse_allowlisted (pure) + apply
├── config.rs         # ~/.jevcli/jev.conf user config (same allowlist,
│                     #   chmod-600 warning; loads after .env, both only
│                     #   fill unset vars)
├── input.rs          # StateInput (--state/--state-json/--state-file, "-"
│                     #   = stdin) + bounded reads (MAX_STATE_BYTES)
├── spec.rs           # ask request document (deny_unknown_fields)
├── output.rs         # json/text renderers (OutputFormat; text = bare value)
└── run.rs            # executors: validate → resolve state → API → Outcome
                      #   {stdout, code}; no printing in the lib
tests/
├── common/mod.rs     # in-process mock server (state-pattern-driven answers,
│                     #   echoes requested model, serves GET /v1/models)
├── e2e.rs            # full pipeline vs. the mock server
└── live.rs           # real-API tests, skip silently without TYPESAFE_API_KEY
docs/                 # usage.md, output.md, jev-cli.1 (man page)
```

The SDK is a crates.io dependency
([jev-sdk](https://github.com/portlandhodl/jev-sdk)); its conventions and
security invariants live in that repo.

## Commands

```sh
cargo build
cargo test                          # unit + e2e (live tests skip w/o key)
TYPESAFE_API_KEY=... cargo test     # includes live API tests
cargo clippy --all-targets          # must be warning-free
cargo fmt -- --check                # must be clean
cargo audit                         # must be clean
man -l docs/jev-cli.1               # check man page after editing it
```

## Conventions & invariants

- **Output contract is the product.** stdout carries only the answer;
  diagnostics go to stderr; exit 0 ok / 1 gate-not-met / 2 error. The
  library never prints — executors return `run::Outcome { stdout, code }`
  and `main` owns stdout/stderr/exit. Do not regress this.
- No `unwrap`/`expect`/panics in library code paths that user or network
  input can reach; return `Error` instead. (Tests are exempt.)
- **Credentials:** keys come from the real environment, `./.env`, or
  `~/.jevcli/jev.conf`. File loading is allowlisted to
  `TYPESAFE_API_KEY`/`TYPESAFE_DEFAULT_MODEL` — `TYPESAFE_BASE_URL` must
  never load from a file (a planted file could exfiltrate the key;
  regression-tested). Never log or print credential values. `.env` is
  gitignored; never commit secrets.
- Input reads are bounded (`MAX_STATE_BYTES` / `MAX_REQUEST_BYTES`); a
  runaway stdin must error, not exhaust memory.
- Parsing helpers (`env::parse_allowlisted`, `spec::AskRequest`) are pure;
  env-mutating tests must not touch the same variable from two tests (the
  lib test binary is one multi-threaded process).
- Model answers vary by version: live tests assert *directionally*
  (thresholds, ordering), never exact probabilities.
- Add tests for every behavior change; update `README.md`, `docs/`, the man
  page, and this file when behavior, layout, or commands change.
