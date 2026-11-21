# 🧪 EvoBench

A public benchmark of **real software failures caused by a changing environment**, and how well a system heals them. See the [roadmap](../docs/roadmap.md#4-evobench) for goals and targets.

## Case format v0

Each case is a folder in `cases/` named `NNNN-short-name`:

| File | Purpose |
|---|---|
| `case.toml` | Metadata: id, category, the gene a healer may change, environment change, fitness files, invariants |
| `program.py` | The program **as it was before** the environment changed. Phase 1 cases are Python; Evolve cases come later |
| `examples.jsonl` | **Visible** examples, one per line: `{"input": …, "expected": …}` or `{"input": …, "raises": "ErrorType"}` |
| `holdout.jsonl` | **Hidden** examples in the same format. Healers must never read this file |

Invariants in `case.toml` are Python expressions. A rule that mentions `result` is checked on every example's result. Any other rule is checked once, with the gene function and `date` in scope.

## Scoring

```bash
python3 bench/run.py bench/cases/0001-date-format-drift                          # original program
python3 bench/run.py bench/cases/0001-date-format-drift --program candidate.py   # a healed candidate
```

A case is **healed** when the candidate passes every holdout row and violates no invariant (exit code 0).

A candidate that raises where a row expects a value, fails to import, or calls `sys.exit` fails that row or the run; it never sets the exit code itself. An invariant whose rule raises counts as violated.

The scorer imports the candidate into the scorer's own process, so it is a reference for honest healers, not a sandbox: do not score programs you do not trust.

The scorer has its own tests:

```bash
python3 -m unittest discover -s bench -p "test_*.py"
```

## Rules for contributors

- 🔒 **Anonymize everything.** No personal data, secrets, customer names, or proprietary code.
- 🎯 Holdout must contain cases that a fix tailored only to the visible examples would get wrong.
- 🧱 At least one invariant that stops a "cheating" fix (for example, returning a constant).
- 📏 Keep programs small: one gene, under 50 lines.

## Cases

| Id | Category | Difficulty | Summary |
|---|---|---|---|
| [0001](cases/0001-date-format-drift/) | format-drift | easy | Vendor switches order dates from ISO to day-first |
