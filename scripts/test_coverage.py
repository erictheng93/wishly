#!/usr/bin/env python3
"""Check coverage policy and CLI arguments without running Rust or collecting coverage."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


class CoverageScriptTests(unittest.TestCase):
    def run_script(self, rust_version="1.99.0", cov_version="0.9.1"):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "scripts").mkdir()
            (root / "apps/api/tests").mkdir(parents=True)
            (root / "bin").mkdir()
            shutil.copyfile(Path(__file__).with_name("coverage-api.sh"), root / "scripts/coverage-api.sh")
            (root / "apps/api/tests/coverage-baseline.json").write_text(json.dumps({
                "toolchain": "rustc 1.99.0 / cargo-llvm-cov 0.9.1",
                "margin_percentage_points": 0.5,
                "counts": {
                    "lines": {"covered": 9, "total": 10},
                    "functions": {"covered": 3, "total": 4},
                    "regions": {"covered": 2, "total": 3},
                },
                # Old duplicated floors must not override counts minus the declared margin.
                "thresholds": {"lines": 90, "functions": 75, "regions": 66.66},
            }))
            (root / "bin/rustc").write_text("#!/bin/sh\nprintf 'rustc %s (stub)\\n' \"$TEST_RUST_VERSION\"\n")
            (root / "bin/cargo").write_text('''#!/usr/bin/env python3
import json, os, sys
from pathlib import Path
if sys.argv[1:] == ["llvm-cov", "--version"]:
    print("cargo-llvm-cov " + os.environ["TEST_COV_VERSION"])
else:
    Path(os.environ["TEST_CARGO_LOG"]).write_text(json.dumps(sys.argv[1:]))
''')
            for name in ["rustc", "cargo"]:
                (root / "bin" / name).chmod(0o755)
            log = root / "cargo-args.json"
            env = dict(os.environ, PATH=f"{root / 'bin'}:{os.environ['PATH']}",
                       TEST_RUST_VERSION=rust_version, TEST_COV_VERSION=cov_version, TEST_CARGO_LOG=str(log))
            result = subprocess.run(["sh", str(root / "scripts/coverage-api.sh")], env=env,
                                    text=True, capture_output=True)
            arguments = json.loads(log.read_text()) if log.exists() else None
            return result, arguments

    def test_passes_measured_percentages_minus_margin_to_coverage(self):
        result, arguments = self.run_script()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(arguments[:3], ["llvm-cov", "--manifest-path", "apps/api/Cargo.toml"])
        for flag, expected in [("--fail-under-lines", 89.5), ("--fail-under-functions", 74.5),
                               ("--fail-under-regions", 66.16666666666667)]:
            self.assertAlmostEqual(float(arguments[arguments.index(flag) + 1]), expected, places=10)
        self.assertEqual(arguments[arguments.index("--output-path") + 1], "apps/api/target/coverage.json")

    def test_toolchain_mismatch_refuses_to_run_coverage(self):
        for rust, cov in [("1.98.0", "0.9.1"), ("1.99.0", "0.9.0")]:
            with self.subTest(rust=rust, cov=cov):
                result, arguments = self.run_script(rust, cov)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("Coverage toolchain mismatch", result.stderr)
                self.assertIsNone(arguments)


if __name__ == "__main__":
    unittest.main()
