# jev-cli usage recipes

All examples assume the API key is configured (see the README: `~/.jevcli/jev.conf`,
`.env`, or `TYPESAFE_API_KEY`). Run any command with `--help` for its flags.

## Predict: go/no-go before acting

```sh
jev-cli predict \
    --state "Deploying v2.3 to production. All 412 tests pass. The canary has \
             been green for 30 minutes. Only one schema migration, additive." \
    --question "Will the deploy complete without errors?"
```

```json
{
  "type": "noul",
  "question": "Will the deploy complete without errors?",
  "probability": 0.97,
  "threshold": null,
  "pass": null,
  "model": "jev-1.13.0",
  "usage": {"input_tokens": 282, "output_tokens": 21}
}
```

Gate on it — the answer prints either way, the exit code decides:

```sh
if jev-cli predict --state "$CONTEXT" \
       --question "Is this change safe to auto-apply without human review?" \
       --yes "Applies cleanly, reversible, no data at risk" \
       --no "Could lose data, break production, or is irreversible" \
       --threshold 0.9 --format text; then
    ./apply.sh
else
    echo "below threshold (or p < 0.9); asking a human"
fi
```

`--yes`/`--no` are optional rubric hints that sharpen calibration when the
question is ambiguous.

## Choose: pick a path

```sh
CHOICE=$(jev-cli choose --format text --state "$CONTEXT" \
    --question "Which approach is most likely to succeed?" \
    --option "retry=Retry the failed request with exponential backoff" \
    --option "fallback=Serve the cached response from 5 minutes ago" \
    --option "abort=Stop and ask the user")
case "$CHOICE" in
    retry)    ./retry.sh ;;
    fallback) ./serve-cache.sh ;;
    abort)    notify-user ;;
esac
```

Add `--min-confidence 0.6` to get exit `1` when the model is unsure — treat
that as an abstention and escalate rather than following a coin flip.

## Score: grade on your rubric

```sh
jev-cli score --state "$CONTEXT" --question "How risky is this plan?" \
    --level "trivial: read-only, local, reversible" \
    --level "moderate: writes state but recoverable" \
    --level "severe: data loss or outage possible"
```

`score` is probability-weighted over the levels (0..N-1) and can land
between them; `legend` in the JSON echoes your level descriptions;
`probabilities` gives the per-level distribution.

## Ask: several questions, one call

One API round-trip evaluates many typed questions against the same state —
cheaper and more consistent than separate calls:

```sh
cat > request.json <<'EOF'
{
  "state": {
    "goal": "upgrade postgres 15 -> 17 on the primary",
    "context": ["replica is 2s behind", "no maintenance window", "last backup 1h ago"]
  },
  "questions": {
    "safe_now":  {"type": "noul", "instructions": "Safe to start the upgrade now?"},
    "strategy":  {"type": "choice", "instructions": "Best upgrade strategy?",
                  "criteria": {"in_place": "pg_upgrade on the primary",
                               "switchover": "promote an upgraded replica",
                               "defer": "wait for a maintenance window"}},
    "risk":      {"type": "score", "instructions": "Risk of unrecoverable data loss",
                  "criteria": ["negligible", "low", "moderate", "high"]}
  }
}
EOF
jev-cli ask request.json          # or: cat request.json | jev-cli ask -
```

The response is one JSON document with an answer per question id, in the
same order. See [output.md](output.md) for the schema.

## Models

```sh
jev-cli models                  # JSON
jev-cli models --format text    # name <tab> release date <tab> description
```

## Structured state

`--state-json` passes a JSON value (object, array, ...) instead of text:

```sh
jev-cli predict --state-json '{"task":"rm -rf build/","cwd":"/repo","dirty":false}' \
    --question "Is this command safe to run?"
```

## Troubleshooting

| Symptom | Cause |
|---------|-------|
| `missing API key` | key not in env, `.env`, or `~/.jevcli/jev.conf` |
| `no state provided` | pass exactly one of `--state` / `--state-json` / `--state-file` |
| `choose needs at least two --option values` | repeat `--option`, once per candidate |
| `permissions ... too open` | `chmod 600 ~/.jevcli/jev.conf` |
| `must use https` in an error | `TYPESAFE_BASE_URL` overridden to plain HTTP (only loopback is allowed) |
