#!/usr/bin/env python3
"""Exercise receipts and source restoration without cargo, a DB, or real signals."""
import hashlib
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location("mutator", Path(__file__).with_name("mutate.py"))
mutator = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mutator)
GREEN = "test result: ok. 1 passed; 0 failed;"
RED = "test result: FAILED. 0 passed; 1 failed;"
ORIGINAL = b'pub async fn create() { let value = "original"; }\n'


class MutatorTests(unittest.TestCase):
    def exercise(self, mutant_log, interrupted=False, expected_marker="semantic violation"):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "source.rs"
            source.write_bytes(ORIGINAL)
            output = root / "output"
            receipt = output / "unit" / "result.json"
            receipt.parent.mkdir(parents=True)
            receipt.write_text('{"result":"killed"}')
            (receipt.parent / "restored.log").write_text("stale restored log")
            phases = []

            def spawn(_command, *, stdout, **_options):
                phase = Path(stdout.name).stem
                phases.append(phase)
                self.assertEqual(source.read_bytes(), ORIGINAL if phase != "mutant" else ORIGINAL.replace(b"original", b"changed"))
                stdout.write(GREEN if phase != "mutant" else mutant_log)

                class Child:
                    pid = 12345
                    should_interrupt = interrupted and phase == "mutant"

                    def __enter__(self):
                        return self

                    def __exit__(self, *_args):
                        return False

                    def wait(self):
                        if self.should_interrupt:
                            self.should_interrupt = False
                            raise KeyboardInterrupt()
                        return 101 if phase == "mutant" else 0

                return Child()

            with patch.object(mutator, "ROOT", root), patch.dict(mutator.CASES, {"unit": (
                "source.rs", "create", "original", "changed", "fake", "fake_test",
            )}, clear=True), patch.dict(mutator.FAILURE_MARKERS, {"unit": expected_marker}, clear=True), \
                patch.object(mutator.subprocess, "check_output", return_value="fake provenance"), \
                patch.object(mutator.subprocess, "Popen", side_effect=spawn), patch.object(mutator.os, "killpg") as killpg:
                if interrupted:
                    with self.assertRaises(KeyboardInterrupt):
                        mutator.run("unit", output)
                    killpg.assert_called_once_with(12345, mutator.signal.SIGKILL)
                else:
                    killed = mutator.run("unit", output)
                    killpg.assert_not_called()
            data = json.loads(receipt.read_text())
            self.assertEqual(source.read_bytes(), ORIGINAL)
            self.assertTrue(data["source_restored"])
            self.assertEqual(data["original_sha256"], hashlib.sha256(ORIGINAL).hexdigest())
            self.assertEqual(data["restored_sha256"], data["original_sha256"])
            self.assertNotEqual(data["mutant_sha256"], data["original_sha256"])
            self.assertEqual(data["test"], "fake_test")
            self.assertEqual(data["runner_sha256"], hashlib.sha256(Path(mutator.__file__).read_bytes()).hexdigest())
            if interrupted:
                self.assertEqual(phases, ["baseline", "mutant"])
                self.assertFalse((receipt.parent / "restored.log").exists())
                self.assertEqual(data["result"], "interrupted")
                self.assertEqual(data["phase"], "mutant")
                self.assertIn("KeyboardInterrupt", data["error"])
            else:
                self.assertEqual(phases, ["baseline", "mutant", "restored"])
                self.assertEqual(data["phase"], "complete")
                return killed, data

    def test_semantic_failure_is_killed_after_restored_green(self):
        killed, data = self.exercise("semantic violation\n" + RED)
        self.assertTrue(killed)
        self.assertEqual(data["result"], "killed")

    def test_http500_and_generic_failure_do_not_count_as_kills(self):
        for log in ["INTERNAL_ERROR semantic violation\n" + RED, "assertion `left == right` failed\n" + RED]:
            with self.subTest(log=log):
                killed, data = self.exercise(log)
                self.assertFalse(killed)
                self.assertEqual(data["result"], "invalid-mutant")

    def test_concurrency_kills_require_the_specific_database_evidence(self):
        for name in ("delete-item-for-update", "reallocation-source-first",
                     "idempotency-claim-on-conflict", "idempotency-pledge-on-conflict"):
            marker = mutator.FAILURE_MARKERS[name]
            with self.subTest(case=name):
                killed, _ = self.exercise(marker + " 0 -> 1\n" + RED, expected_marker=marker)
                self.assertTrue(killed)
                for diagnostic in ("race did not finish (possible deadlock)",
                                   "expected 2 blocked statements", "SQLSTATE 23505",
                                   "assertion failed: HTTP 500"):
                    killed, receipt = self.exercise(diagnostic + "\n" + RED, expected_marker=marker)
                    self.assertFalse(killed)
                    self.assertEqual(receipt["result"], "invalid-mutant")

    def test_interrupted_rerun_restores_source_and_replaces_old_receipt(self):
        self.exercise("semantic violation", interrupted=True)


if __name__ == "__main__":
    unittest.main()
