"""Report semantics; no browser, network or Rust binary required."""
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location(
    "refcompare", Path(__file__).resolve().parents[1] / "tools/refcompare.py")
refcompare = importlib.util.module_from_spec(spec)
spec.loader.exec_module(refcompare)


class ReportTests(unittest.TestCase):
    def test_dynamic_scores_do_not_change_static_average(self):
        rows = [("demo1", .9), ("demo13-grid", .8),
                ("demo22-svg-gauges", .1), ("demo21-panel", .2),
                ("demo5-js-clock", .3), ("demo29-fetch", .4)]
        output = refcompare.report(rows)
        self.assertIn("| Static snapshots | 2 | 85.00% |", output)
        self.assertIn("| Animations and timers | 1 | 10.00% |", output)
        self.assertIn("| Random content | 1 | 20.00% |", output)
        for name, value in rows:
            self.assertEqual(output.count(f"| {name} |"), 1)
        self.assertNotIn("| **average**", output)

    def test_empty_and_dynamic_only_runs(self):
        for rows in ([], [("demo26-pixels", 0)]):
            output = refcompare.report(rows)
            self.assertIn("| Static snapshots | 0 | n/a |", output)
            self.assertNotIn("## Static snapshots", output)
        output = refcompare.report([("demo26-pixels", 0)])
        self.assertIn("| Animations and timers | 1 | 0.00% |", output)
        self.assertIn("time-dependent pixel animation", output)

    def test_timer_and_timestamp_demos_are_not_static(self):
        for name in ("demo8-js-hmi", "demo21-panel", "demo22-svg-gauges",
                     "demo28-preact"):
            self.assertIn(name, refcompare.SCENARIOS)


if __name__ == "__main__":
    unittest.main()
