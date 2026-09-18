# jev-cli output contract

Stable rules any caller (especially an LLM agent) can rely on:

- **stdout** carries exactly one answer artifact and nothing else. **stderr**
  carries diagnostics (`jev-cli: warning: ...` / `jev-cli: error: ...`).
- Default `--format json`: stdout is one pretty-printed JSON document.
- `--format text`: stdout is one line containing only the primary value.
- Field order in JSON is not contractual; presence of documented fields is.
  Unknown future fields may appear — ignore them.

## Exit codes

| Code | Name | Meaning |
|------|------|---------|
| `0` | ok | Answer delivered; if a gate was given, it was met |
| `1` | not met | Answer delivered, but below `--threshold` (`predict`) or `--min-confidence` (`choose`) |
| `2` | error | Usage, configuration, IO, or API error. Nothing on stdout |

`1` means "no, with a valid answer"; `2` means "no answer exists". Scripts
must not treat them the same.

## `predict` (JSON)

```json
{
  "type": "noul",
  "question": "Will the deploy succeed?",
  "probability": 0.97,
  "threshold": 0.8,
  "pass": true,
  "model": "jev-1.13.0",
  "usage": {"input_tokens": 282, "output_tokens": 21}
}
```

| Field | Type | Notes |
|-------|------|-------|
| `probability` | number 0..1 | P(yes) |
| `threshold` | number \| null | the gate, if given |
| `pass` | boolean \| null | `probability >= threshold`; null when no gate |
| `model` | string | resolved model version (not necessarily your alias) |

`--format text`: the probability, e.g. `0.97`.

## `choose` (JSON)

```json
{
  "type": "choice",
  "question": "Which approach?",
  "choice": "retry",
  "probabilities": {"retry": 0.9, "abort": 0.1},
  "confidence": 0.88,
  "min_confidence": 0.6,
  "pass": true,
  "model": "jev-1.13.0",
  "usage": {"input_tokens": 305, "output_tokens": 34}
}
```

| Field | Type | Notes |
|-------|------|-------|
| `choice` | string | highest-probability option name (verbatim from `--option`) |
| `probabilities` | object | every option → probability; sums to 1 |
| `confidence` | number 0..1 | derived from the distribution; low ≈ coin flip |
| `min_confidence`, `pass` | number/boolean \| null | gate fields, null when no gate |

`--format text`: the chosen option name, e.g. `retry`.

## `score` (JSON)

```json
{
  "type": "score",
  "question": "How risky is this plan?",
  "score": 1.9,
  "legend": {"0": "trivial", "1": "moderate", "2": "severe"},
  "probabilities": {"0": 0.02, "1": 0.16, "2": 0.82},
  "confidence": 0.9,
  "model": "jev-1.13.0",
  "usage": {"input_tokens": 298, "output_tokens": 30}
}
```

| Field | Type | Notes |
|-------|------|-------|
| `score` | number | probability-weighted, range `0..(levels-1)`; can land between levels |
| `legend` | object | level index (as string) → your level description |
| `probabilities` | object | level index → probability; may be omitted by the model (then `{}`) |

`--format text`: the score, e.g. `1.9`.

## `ask` (always JSON)

`ask` passes the API response through, one answer per question id, in the
order the questions were given:

```json
{
  "model": "jev-1.13.0",
  "answers": {
    "safe_now": {"type": "noul", "noul": 0.87},
    "strategy": {"type": "choice", "choice": "switchover",
                 "probabilities": {"in_place": 0.1, "switchover": 0.8, "defer": 0.1},
                 "confidence": 0.76},
    "risk": {"type": "score", "score": 2.3,
             "legend": {"0": "negligible", "1": "low", "2": "moderate", "3": "high"},
             "probabilities": {"0": 0.0, "1": 0.1, "2": 0.5, "3": 0.4},
             "confidence": 0.83}
  },
  "usage": {"input_tokens": 402, "output_tokens": 55}
}
```

`--format text` has no effect here: with several questions there is no
single primary value.

## `models`

JSON: `{"models": [{"name", "description", "release_date"}, ...]}`.
Text: one line per model, `name <TAB> release_date <TAB> description`.

## Request document (input to `ask`)

```json
{
  "state": "<any JSON value; a string is fine>",
  "questions": {
    "<id>": {"type": "noul", "instructions": "...", "criteria": {"true": "...", "false": "..."}},
    "<id>": {"type": "choice", "instructions": "...", "criteria": {"opt": "desc or null"}},
    "<id>": {"type": "score", "instructions": "...", "criteria": ["low", "...", "high"]}
  },
  "model": "optional; a --model flag wins over this"
}
```

Unknown top-level fields are rejected (typos fail loudly). Question ids must
be non-empty and are echoed back as the `answers` keys. `instructions` may
be any JSON value, not just a string.
