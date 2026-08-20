#!/usr/bin/env bash
# The one chain that decides whether the tree is healthy. Same steps a CI job
# would run (see ci.plan.md), so "it worked on my machine" cannot happen.
#
#   scripts/verify.sh
#
# The defining property: it does NOT stop at the first failure. Every step runs,
# failures are recorded, and a summary is printed at the end — one run must
# surface every problem, not just the earliest one. Exit code is nonzero if any
# step failed.
set -euo pipefail

cd "$(dirname "$0")/.."

# On a Nix host without an active devshell, step into one — otherwise `cargo`,
# `cargo-nextest`, `cargo-deny` and `cargo-machete` simply would not exist.
# VERIFY_NO_NIX is the escape hatch for containers/CI images that ship the
# toolchain themselves and have no `nix` binary at all; it is also set on the
# re-exec so the child cannot loop back into here.
if [ -z "${IN_NIX_SHELL:-}" ] && [ -z "${VERIFY_NO_NIX:-}" ] && command -v nix >/dev/null; then
  exec nix develop -c env VERIFY_NO_NIX=1 "$0" "$@"
fi

if [ "$#" -gt 0 ]; then
  echo "verify.sh takes no arguments (got: $*)" >&2
  exit 2
fi

FAILED=()
step() { printf '\n\033[1;34m== %s\033[0m\n' "$*"; }
run() {
  local name="$1"
  shift
  step "$name"
  if "$@"; then
    printf '\033[32mPASS %s\033[0m\n' "$name"
  else
    printf '\033[31mFAIL %s\033[0m\n' "$name"
    FAILED+=("$name")
  fi
}

# Order is cheapest-first: fmt is instant, clippy warms the same build cache
# nextest then reuses, and the two dependency audits need no compilation at all.
run "fmt" cargo fmt --all --check

# --all-targets pulls tests/ and benches in as well; without it a lint violation
# in tests/ only shows up in CI. -D warnings turns the [lints.*] set from
# Cargo.toml into an error wall. --locked: a verification run must never silently
# resolve a different dependency graph than the committed Cargo.lock.
run "clippy" cargo clippy --all-targets --all-features --locked -- -D warnings

run "nextest" cargo nextest run --all-features --locked

# cargo-audit is deliberately NOT run in addition: it checks the same RUSTSEC
# database as `cargo deny check advisories`, but would need a second, separately
# maintained ignore list (audit.toml). deny.toml stays the single source of truth.
# --deny warnings: an unused skip/allow rule in deny.toml should be noticed.
run "cargo-deny" cargo deny check --deny warnings

# Catches dependencies that are declared but never referenced — the crate graph
# is 21 crates and every one of them is build time and attack surface.
run "cargo-machete" cargo machete

echo
if [ ${#FAILED[@]} -eq 0 ]; then
  printf '\033[1;32mAll green.\033[0m\n'
else
  printf '\033[1;31mFailed: %s\033[0m\n' "${FAILED[*]}"
  exit 1
fi
