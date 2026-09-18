# jev-cli

An agent-facing command line for [Jev](https://typesafe.ai/blog/introducing-system-one-models-and-jev),
a System One model — via the unofficial
[`jev-sdk`](https://crates.io/crates/jev-sdk). Not affiliated with
TypeSafe AI.

LLM agents act first and find out later. jev-cli flips that: before starting
an action, an agent asks a *typed* question and gets back **calibrated
probabilities**, not chatbot prose — so it can gate on a threshold, or pick
the best of several paths, in one shell call:

```sh
# Will this action succeed? → a probability
jev-cli predict --state "$CONTEXT" --question "Will this deploy complete without errors?"
# {"type":"noul","probability":0.97,"threshold":null,"pass":null,...}

# Which of these paths should the agent take? → the chosen option
jev-cli choose --state "$CONTEXT" --question "Which approach is most likely to succeed?" \
    --option "retry=Retry the failed request with backoff" \
    --option "fallback=Use the cached response" \
    --option "abort=Stop and ask the user"

# How bad is it? → a probability-weighted score on your rubric
jev-cli score --state "$CONTEXT" --question "How risky is this plan?" \
    --level trivial --level moderate --level severe
```

Because it is built for scripts and tool-use loops, the contract is strict:
**stdout carries only the answer** (JSON by default), diagnostics go to
stderr, and exit codes are meaningful (see below).

## Install & setup

```sh
cargo install --git https://github.com/portlandhodl/jev-cli
# or: git clone https://github.com/portlandhodl/jev-cli && cd jev-cli && cargo install --path .
```

Put your API key (from <https://console.typesafe.ai/settings/keys>) in the
user config file:

```sh
mkdir -p ~/.jevcli && chmod 700 ~/.jevcli
printf 'TYPESAFE_API_KEY=apikey_...\n' > ~/.jevcli/jev.conf
chmod 600 ~/.jevcli/jev.conf          # it holds a credential
```

`jev.conf` is dotenv-style `KEY=value`. Alternatively use a project-local
`.env` (gitignored) or plain environment variables. Precedence, lowest
first:

```
~/.jevcli/jev.conf  <  ./.env  <  real environment  <  CLI flags
```

Only `TYPESAFE_API_KEY` and `TYPESAFE_DEFAULT_MODEL` are ever read from
files. `TYPESAFE_BASE_URL` is deliberately **not** loadable from files — a
planted `.env` in a cloned repo could otherwise redirect your key to a
hostile host; set it in the real environment if you need it. Override the
file locations with `--env-file` / `--config-file` (`none` disables either).

## Commands

| Command | Question type | Answers |
|---------|---------------|---------|
| `predict` (alias `will`) | yes/no | `probability` 0..1; `--threshold T` gates the exit code |
| `choose` (alias `pick`)  | one of N options | `choice`, `probabilities`, `confidence`; `--min-confidence C` gates |
| `score` (alias `rate`)   | ordered rubric | `score`, `legend`, `probabilities`, `confidence` |
| `ask` | arbitrary typed questions | full response document (JSON in, JSON out) |
| `models` | — | models available to the account |

Every question takes a **state** — the situation to evaluate — via exactly
one of `--state TEXT`, `--state-json JSON`, or `--state-file FILE` (`-` =
stdin). `ask` instead reads a whole request document
(`{"state": ..., "questions": {...}, "model"?}`) from a file or stdin.

Global flags: `--format json|text` (default `json`; `text` prints only the
bare answer value for `$()` capture), `--model MODEL`.

## Output & exit codes

```sh
jev-cli predict --format text --state "$C" --question "Is this safe to auto-apply?" \
    --threshold 0.8 && ./apply.sh       # runs apply.sh only if p >= 0.8
```

| Code | Meaning |
|------|---------|
| `0` | success (and the gate, if any, was met) |
| `1` | gate not met: probability below `--threshold` / confidence below `--min-confidence` (the answer still prints) |
| `2` | tool, input, or API error (nothing on stdout) |

The full JSON schema is documented in [docs/output.md](docs/output.md);
recipes live in [docs/usage.md](docs/usage.md). There is a man page:

```sh
man -l docs/jev-cli.1                                   # read it in place
install -m644 docs/jev-cli.1 /usr/local/share/man/man1/ # system install
```

## For LLM agents

This tool is designed to be called by agents. Read [AGENTS.md](AGENTS.md) in
this directory for a machine-oriented calling contract (command templates,
output schema, exit codes, cost notes). Point your agent at it.

## Caveats

- **The state you pass is sent to the TypeSafe API.** Unlike jev-guard,
  jev-cli does **not** redact secrets — you choose the state verbatim, so
  keep credentials, personal data, and code you may not share out of it.
- **Answers are calibrated probabilities, not guarantees.** Use thresholds
  and confidence gates; treat a low-confidence answer as "abstain".
- Calibration shifts between model versions; re-tune thresholds when the
  model changes (`jev-cli models` shows what your account resolves).
- State and request documents are capped at 16 MiB; the API key never
  appears in output, logs, or errors.

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at
your option.
