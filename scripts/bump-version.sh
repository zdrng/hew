#!/usr/bin/env bash
# Rewrites the package version in the three files that carry it:
#
#   Cargo.toml   [package] version
#   flake.nix    packages.default version
#   Cargo.lock   the hew package block
#
#   scripts/bump-version.sh 0.2.0
#
# Deliberately needs no cargo: the release job runs this without a toolchain,
# and every build job later runs `cargo build --locked`, which fails loudly if
# the Cargo.lock edit here ever went wrong. Portable awk only (macOS runs this
# too), no sed -i. Idempotent — re-applying the current version is a no-op.
set -euo pipefail

cd "$(dirname "$0")/.."

if [ "$#" -ne 1 ]; then
  echo "usage: scripts/bump-version.sh <version>" >&2
  exit 2
fi
NEW="$1"

# Semver with optional pre-release/build metadata. Also everything the awk
# programs below need: no quote, ampersand or backslash can reach sub().
if ! [[ "$NEW" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?(\+[0-9A-Za-z.]+)?$ ]]; then
  echo "Not a semver version: $NEW" >&2
  exit 2
fi

# Same extraction release.sh uses for artifact names — the two must agree.
current() {
  awk -F'"' '/^\[/{s=$1} s ~ /^\[package\]/ && /^[[:space:]]*version[[:space:]]*=/{print $2; exit}' Cargo.toml
}
OLD="$(current)"

rewrite() { # $1 = file, $2 = awk program (gets -v v="$NEW")
  awk -v v="$NEW" "$2" "$1" >"$1.tmp"
  mv "$1.tmp" "$1"
}

# First `version =` under [package] only — never a dependency's.
rewrite Cargo.toml '
  /^\[/ { s = $0 }
  s == "[package]" && !done && /^[[:space:]]*version[[:space:]]*=/ {
    sub(/"[^"]*"/, "\"" v "\""); done = 1
  }
  { print }
'

# The buildRustPackage version line — the only `version = "...";` in the file;
# the anchored pattern keeps this safe should another one ever appear first.
rewrite flake.nix '
  !done && /^[[:space:]]*version = "[^"]*";$/ {
    sub(/"[^"]*"/, "\"" v "\""); done = 1
  }
  { print }
'

# The version line directly under `name = "hew"` in its [[package]] block.
rewrite Cargo.lock '
  /^name = "hew"$/ { p = 1 }
  p && /^version = / { $0 = "version = \"" v "\""; p = 0 }
  { print }
'

# Prove all three files now agree before anything downstream trusts them.
FAILED=0
check() { # $1 = file, $2 = extracted value
  if [ "$2" != "$NEW" ]; then
    echo "FAIL $1: expected $NEW, found ${2:-<nothing>}" >&2
    FAILED=1
  fi
}
check Cargo.toml "$(current)"
check flake.nix "$(awk -F'"' '/^[[:space:]]*version = "[^"]*";$/{print $2; exit}' flake.nix)"
check Cargo.lock "$(awk -F'"' '/^name = "hew"$/{p=1} p && /^version = /{print $2; exit}' Cargo.lock)"
if [ "$FAILED" -ne 0 ]; then
  exit 1
fi

echo "version: $OLD -> $NEW (Cargo.toml, flake.nix, Cargo.lock)"
