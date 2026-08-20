#!/usr/bin/env bash
# Line coverage via cargo-llvm-cov (source-based instrumentation, not gcov):
#
#   scripts/coverage.sh          # threshold 80 %
#   COVERAGE_MIN=90 scripts/coverage.sh
#
# Writes a browsable HTML report to target/llvm-cov/html/index.html and fails if
# line coverage is below the threshold.
set -euo pipefail

cd "$(dirname "$0")/.."

# Same preamble as verify.sh — see the rationale there. VERIFY_NO_NIX is shared
# on purpose: one escape hatch for all scripts in this directory.
if [ -z "${IN_NIX_SHELL:-}" ] && [ -z "${VERIFY_NO_NIX:-}" ] && command -v nix >/dev/null; then
  exec nix develop -c env VERIFY_NO_NIX=1 "$0" "$@"
fi

# Overridable so CI can ratchet the number up without editing the script.
COVERAGE_MIN="${COVERAGE_MIN:-80}"

printf '\n\033[1;34m== coverage (threshold: %s%% lines)\033[0m\n' "$COVERAGE_MIN"

# Three steps instead of the obvious one-liner, and deliberately so:
# `--no-report` collects the raw profiles without rendering anything, then the
# HTML report is generated BEFORE the threshold is enforced. That way a run that
# fails the gate still leaves behind the report you need to see which lines are
# uncovered — the single most useful artifact exactly when the build is red.
cargo llvm-cov nextest --all-features --locked --no-report

# Default output location of --html is target/llvm-cov/html; spelled out so the
# path stays stable if cargo-llvm-cov ever changes its default.
cargo llvm-cov report --html --output-dir target/llvm-cov

# The gate. Runs last so the exit code of the script is the coverage verdict.
cargo llvm-cov report --summary-only --fail-under-lines "$COVERAGE_MIN"

printf '\n\033[1;32mCoverage >= %s%%. Report: target/llvm-cov/html/index.html\033[0m\n' "$COVERAGE_MIN"
