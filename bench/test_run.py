"""Tests for the EvoBench reference scorer.

    python3 -m unittest discover -s bench -p "test_*.py"

The scorer's contract is its exit code (0 = healed), so each test runs it the way a
healer would: as a subprocess, with a candidate program on disk.
"""
import pathlib
import subprocess
import sys
import tempfile
import textwrap
import unittest

BENCH = pathlib.Path(__file__).resolve().parent
CASE = BENCH / "cases" / "0001-date-format-drift"

HEALED = """
    import datetime

    def parse_order_date(raw):
        raw = raw.strip()
        for fmt in ("%Y-%m-%d", "%d/%m/%Y"):
            try:
                return datetime.datetime.strptime(raw, fmt).date()
            except ValueError:
                pass
        raise ValueError(raw)
"""


def score(candidate=None):
    """Runs the scorer on case 0001, optionally with `candidate` as the program source."""
    with tempfile.TemporaryDirectory() as tmp:
        cmd = [sys.executable, str(BENCH / "run.py"), str(CASE)]
        if candidate is not None:
            program = pathlib.Path(tmp) / "candidate.py"
            program.write_text(textwrap.dedent(candidate), encoding="utf-8")
            cmd += ["--program", str(program)]
        return subprocess.run(cmd, capture_output=True, text=True, timeout=60)


class ScorerTest(unittest.TestCase):
    def test_original_program_is_not_healed(self):
        run = score()
        self.assertEqual(run.returncode, 1, run.stderr)
        self.assertIn("examples  2/5", run.stdout)
        self.assertIn("holdout   3/6", run.stdout)

    def test_healed_candidate_exits_zero(self):
        run = score(HEALED)
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)
        self.assertIn("holdout   6/6", run.stdout)
        self.assertIn("invariants ok", run.stdout)

    def test_constant_answer_is_not_healed(self):
        run = score("""
            import datetime

            def parse_order_date(raw):
                return datetime.date(2026, 11, 21)
        """)
        self.assertEqual(run.returncode, 1)

    def test_candidate_cannot_pick_the_exit_code_from_the_gene(self):
        run = score("""
            import sys

            def parse_order_date(raw):
                sys.exit(0)
        """)
        self.assertEqual(run.returncode, 1, "sys.exit(0) in the gene was scored as healed")
        self.assertIn("holdout   0/6", run.stdout)

    def test_candidate_cannot_pick_the_exit_code_at_import(self):
        run = score("""
            import sys

            sys.exit(0)
        """)
        self.assertEqual(run.returncode, 1, "sys.exit(0) at import was scored as healed")
        self.assertIn("failed to load", run.stdout)

    def test_candidate_that_fails_to_import_is_reported_without_a_traceback(self):
        run = score("def parse_order_date(raw) -> oops")
        self.assertEqual(run.returncode, 1)
        self.assertIn("failed to load", run.stdout)
        self.assertNotIn("Traceback", run.stderr)

    def test_invariant_that_raises_counts_as_violated(self):
        # Right text, wrong type: `result.year` raises AttributeError on a str.
        run = score(HEALED.replace(".date()", ".date().isoformat()"))
        self.assertEqual(run.returncode, 1)
        self.assertNotIn("Traceback", run.stderr)
        self.assertIn("invariants VIOLATED", run.stdout)
        self.assertIn("never-invent-dates on input", run.stdout)


if __name__ == "__main__":
    unittest.main()
