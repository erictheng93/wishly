#!/usr/bin/env sh
set -eu
cd "$(dirname "$0")/.."
set -- $(python3 -c 'import json; b=json.load(open("apps/api/tests/coverage-baseline.json")); print(*(b["thresholds"][k] for k in ("lines", "functions", "regions")))')
cargo llvm-cov --manifest-path apps/api/Cargo.toml \
  --json --summary-only --ignore-filename-regex '/(tests|examples)/' \
  --output-path apps/api/target/coverage.json \
  --fail-under-lines "$1" --fail-under-functions "$2" --fail-under-regions "$3"
