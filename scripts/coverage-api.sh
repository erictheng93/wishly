#!/usr/bin/env sh
set -eu
cd "$(dirname "$0")/.."
coverage_floors=$(python3 - <<'PY'
import json
import subprocess
import sys
baseline = json.load(open("apps/api/tests/coverage-baseline.json"))
rust = subprocess.check_output(["rustc", "--version"], text=True).split()[1]
cov = subprocess.check_output(["cargo", "llvm-cov", "--version"], text=True).split()[1]
actual = f"rustc {rust} / cargo-llvm-cov {cov}"
if actual != baseline["toolchain"]:
    sys.exit(f"Coverage toolchain mismatch: {actual}; expected {baseline['toolchain']}. Remeasure before updating the baseline.")
print(*(baseline["thresholds"][key] for key in ("lines", "functions", "regions")))
PY
)
set -- $coverage_floors
cargo llvm-cov --manifest-path apps/api/Cargo.toml \
  --json --summary-only --ignore-filename-regex '/(tests|examples)/' \
  --output-path apps/api/target/coverage.json \
  --fail-under-lines "$1" --fail-under-functions "$2" --fail-under-regions "$3"
