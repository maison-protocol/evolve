"""EvoBench reference scorer (format v0).

Usage:
    python3 bench/run.py bench/cases/0001-date-format-drift
    python3 bench/run.py bench/cases/0001-date-format-drift --program path/to/candidate.py

Scores a program against a case's visible examples and hidden holdout, and checks invariants.
Exit code 0 when every holdout case and every invariant passes, 1 otherwise.
"""
import argparse
import datetime
import importlib.util
import json
import pathlib
import sys
import tomllib


def load_program(path, gene):
    spec = importlib.util.spec_from_file_location("evobench_candidate", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return getattr(module, gene)


def to_json(value):
    if isinstance(value, (datetime.date, datetime.datetime)):
        return value.isoformat()
    return value


# What a candidate may raise. SystemExit is included so that a program calling
# `sys.exit(0)` fails its row instead of ending the scorer with a "healed" exit code.
CANDIDATE_ERRORS = (Exception, SystemExit)


def run_set(fn, path, result_invariants):
    with open(path, encoding="utf-8") as f:
        passed, rows = 0, [json.loads(line) for line in f if line.strip()]
    violations = []
    for row in rows:
        try:
            result = fn(row["input"])
        except CANDIDATE_ERRORS as exc:  # a raised error is a valid outcome only when the row expects it
            ok = type(exc).__name__ == row.get("raises")
        else:
            ok = "raises" not in row and to_json(result) == row.get("expected")
            for inv in result_invariants:
                if not _safe_eval(inv["rule"], {"date": datetime.date, "result": result}):
                    violations.append(f'{inv["name"]} on input {row["input"]!r}')
        passed += ok
    return passed, len(rows), violations


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("case", type=pathlib.Path)
    ap.add_argument("--program", type=pathlib.Path, help="candidate program (default: the case's original)")
    args = ap.parse_args()

    case = tomllib.loads((args.case / "case.toml").read_text(encoding="utf-8"))
    gene = case["program"]["gene"]
    try:
        fn = load_program(args.program or args.case / case["program"]["file"], gene)
    except CANDIDATE_ERRORS as exc:
        print(f'case      {case["id"]}')
        print(f"program   failed to load: {type(exc).__name__}: {exc}")
        sys.exit(1)

    invariants = case.get("invariants", [])
    per_result = [i for i in invariants if "result" in i["rule"]]
    global_violations = [i["name"] for i in invariants if "result" not in i["rule"]
                         and not _safe_eval(i["rule"], {gene: fn, "date": datetime.date})]

    vis = run_set(fn, args.case / case["fitness"]["examples"], per_result)
    hold = run_set(fn, args.case / case["fitness"]["holdout"], per_result)
    violations = global_violations + vis[2] + hold[2]

    print(f'case      {case["id"]}')
    print(f"examples  {vis[0]}/{vis[1]}")
    print(f"holdout   {hold[0]}/{hold[1]}")
    print(f"invariants {'ok' if not violations else 'VIOLATED: ' + '; '.join(violations)}")
    sys.exit(0 if hold[0] == hold[1] and not violations else 1)


def _safe_eval(rule, ns):
    """An invariant holds only when its rule evaluates to a true value; a rule that raises is violated."""
    try:
        return bool(eval(rule, ns))
    except CANDIDATE_ERRORS:
        return False


if __name__ == "__main__":
    main()
