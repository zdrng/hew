#!/usr/bin/env bash
# Builds the release matrix into dist/ and proves the hardening on every ELF
# artifact it produces.
#
#   scripts/release.sh                              # whole matrix
#   scripts/release.sh x86_64-unknown-linux-musl    # both variants of one triple
#   scripts/release.sh x86_64-unknown-linux-gnu-v3  # one specific row
#   scripts/release.sh --list                       # show the matrix, build nothing
#
# Like verify.sh, a failing target is RECORDED and the run continues. The
# mingw/windows cross is the flakiest row by a wide margin; it must not be able
# to stop the other six artifacts from being produced.
set -euo pipefail

cd "$(dirname "$0")/.."
ROOT="$PWD"

# Same preamble as verify.sh — see the rationale there.
if [ -z "${IN_NIX_SHELL:-}" ] && [ -z "${VERIFY_NO_NIX:-}" ] && command -v nix >/dev/null; then
  exec nix develop -c env VERIFY_NO_NIX=1 "$0" "$@"
fi

BIN_NAME=hew
DIST="$ROOT/dist"

# ---------------------------------------------------------------------------
# The matrix (plan.md §9). One row per artifact: "<triple>|<target-cpu>".
# An empty target-cpu means the baseline microarchitecture for that triple.
#
# x86-64-v3 = AVX2/BMI2, i.e. any CPU from roughly 2015 onwards. It buys the
# faster memchr / serde_json SIMD paths. The baseline rows stay for older
# hardware, which is why both variants of the same triple exist at all.
# ---------------------------------------------------------------------------
MATRIX=(
  "x86_64-unknown-linux-gnu|"
  "x86_64-unknown-linux-gnu|x86-64-v3"
  "x86_64-unknown-linux-musl|"
  "x86_64-unknown-linux-musl|x86-64-v3"
  "aarch64-unknown-linux-gnu|"
  "aarch64-unknown-linux-musl|"
  "x86_64-pc-windows-gnu|"
  "aarch64-apple-darwin|"
  "x86_64-apple-darwin|"
)

# ---------------------------------------------------------------------------
# Hardening flags per triple.
#
# MUST STAY IN SYNC WITH .cargo/config.toml — the lists below are copied from
# plan.md §7, which is the authoritative source for both files. See the long
# comment at rustflags_config() for why they have to be repeated here at all.
#
# -z relro,-z now  : full RELRO (GOT mapped read-only after relocation)
# -z noexecstack   : PT_GNU_STACK without the E bit
# +crt-static      : musl only — links the C runtime in, yielding a static PIE
#
# Both -z options are ELF-only linker directives: they must NOT be handed to the
# mingw (PE/COFF) or Apple (Mach-O) linkers, which is why those triples get an
# empty flag list and no --config argument at all.
# ---------------------------------------------------------------------------
base_flags() { # $1 = triple; prints one rustc argument per line
  case "$1" in
    *-linux-musl)
      printf '%s\n' \
        '-C' 'target-feature=+crt-static' \
        '-C' 'link-arg=-Wl,-z,relro,-z,now' \
        '-C' 'link-arg=-Wl,-z,noexecstack'
      ;;
    *-linux-gnu)
      printf '%s\n' \
        '-C' 'link-arg=-Wl,-z,relro,-z,now' \
        '-C' 'link-arg=-Wl,-z,noexecstack'
      ;;
    *) : ;; # windows-gnu, apple-darwin: nothing ELF-specific applies
  esac
}

# ---------------------------------------------------------------------------
# THE RUSTFLAGS GOTCHA (plan.md §9).
#
# Cargo has four sources of extra rustc flags — build.rustflags,
# target.<triple>.rustflags, target.<cfg>.rustflags and the RUSTFLAGS env var —
# and they are MUTUALLY EXCLUSIVE. Setting RUSTFLAGS does not append to
# target.<triple>.rustflags from .cargo/config.toml, it REPLACES it wholesale.
#
# The v3 rows share a triple with their baseline rows, so they cannot live in
# .cargo/config.toml as a separate section; they need a per-invocation flag. The
# naive `RUSTFLAGS="-C target-cpu=x86-64-v3" cargo build` would therefore
# silently drop RELRO, noexecstack and +crt-static from exactly those artifacts
# — a hardening regression with no error message anywhere.
#
# The fix used here: pass the COMPLETE flag list through
# `--config 'target.<triple>.rustflags=[...]'` on the command line. That is a
# config source rather than an env var, so it composes with .cargo/config.toml
# under the normal config merge rules instead of disabling it. (Worst case the
# arrays are concatenated and a link-arg appears twice, which is a no-op for the
# linker. Best case the CLI value wins. Either way the full set is present.)
#
# The per-artifact assertions further down are the backstop: if this reasoning
# is ever wrong, the build fails instead of shipping a soft binary.
# ---------------------------------------------------------------------------
rustflags_config() { # $1 = triple, $2 = target-cpu (may be empty); prints a --config value or nothing
  local triple="$1" cpu="$2" f toml=""
  local flags=()
  mapfile -t flags < <(base_flags "$triple")
  if [ -n "$cpu" ]; then
    flags+=('-C' "target-cpu=$cpu")
  fi
  if [ ${#flags[@]} -eq 0 ]; then
    return 0
  fi
  for f in "${flags[@]}"; do
    toml="$toml\"$f\","
  done
  printf 'target.%s.rustflags=[%s]' "$triple" "${toml%,}"
}

# ---------------------------------------------------------------------------
# Assertions. Proof instead of assumption — these are the plan.md §12 items 4-6
# checks, modelled on the ones in ../PureKauf/backend/Dockerfile, run on every
# single artifact rather than once on a local debug build.
# ---------------------------------------------------------------------------
assert_elf() { # $1 = binary path, $2 = "static" for the musl extras
  local bin="$1" mode="${2:-}" problems=() n line

  if ! command -v readelf >/dev/null; then
    echo "readelf missing — add binutils to the devshell (plan.md §8)" >&2
    return 1
  fi

  # (a) stripped. `strip = "symbols"` in [profile.release] must have removed both
  # the debug sections and the symbol table. A stray .debug_* section means the
  # profile did not apply to this target.
  n=$(readelf -SW "$bin" | grep -c '\.debug_' || true)
  [ "$n" -eq 0 ] || problems+=("$n .debug_* section(s) present — not stripped")
  if readelf -SW "$bin" | grep -q '\.symtab'; then
    problems+=(".symtab present — not stripped")
  fi

  # (b) full RELRO. -z now sets DF_BIND_NOW in DT_FLAGS and/or NOW in DT_FLAGS_1;
  # readelf prints them as "BIND_NOW" and "Flags: NOW PIE" respectively, and
  # which of the two appears depends on the linker and on static vs dynamic. Both
  # spellings are accepted; neither one appearing means -z now was lost.
  if ! readelf -dW "$bin" 2>/dev/null | grep -Eq 'BIND_NOW|Flags:.*\bNOW\b'; then
    problems+=("no BIND_NOW/NOW in the dynamic section — full RELRO missing")
  fi

  # (c) non-executable stack. -W keeps each program header on ONE line, so the
  # flag field cannot get lost in the wrapped second line. RWE can only ever be
  # the flags column; the address/size columns are hex and contain no R or W.
  line=$(readelf -lW "$bin" | grep -m1 'GNU_STACK' || true)
  if [ -z "$line" ]; then
    problems+=("no GNU_STACK segment — stack permissions are the kernel default")
  else
    case "$line" in
      *RWE*) problems+=("GNU_STACK is RWE — executable stack") ;;
    esac
  fi

  # (d) PIE, i.e. the binary can be loaded at a random base (ASLR).
  readelf -h "$bin" | grep -q 'Type:.*DYN' \
    || problems+=("ELF type is not DYN — not a PIE, no ASLR")

  # (e) musl only: a TRUE static PIE. A PT_INTERP header or any NEEDED entry
  # means the artifact still wants a dynamic loader at runtime — which defeats
  # the entire point of shipping a musl build (run anywhere, no libc version
  # matching). This is the check PureKauf's Dockerfile runs before copying the
  # binary onto `scratch`.
  if [ "$mode" = "static" ]; then
    if readelf -lW "$bin" | grep -qi 'program interpreter'; then
      problems+=("has a program interpreter (PT_INTERP) — not statically linked")
    fi
    if readelf -dW "$bin" 2>/dev/null | grep -q 'NEEDED'; then
      problems+=("has NEEDED entries — links against shared libraries")
    fi
  fi

  if [ ${#problems[@]} -gt 0 ]; then
    printf '\033[31m  hardening assertion FAILED for %s:\033[0m\n' "$bin" >&2
    printf '    - %s\n' "${problems[@]}" >&2
    return 1
  fi
  echo "  hardening OK (stripped, BIND_NOW, GNU_STACK RW, PIE${mode:+, static})"
}

# ---------------------------------------------------------------------------
# Argument handling
# ---------------------------------------------------------------------------
row_id() { # "triple|cpu" -> "triple" or "triple-v3"
  local triple="${1%%|*}" cpu="${1#*|}"
  if [ -n "$cpu" ]; then printf '%s-v3' "$triple"; else printf '%s' "$triple"; fi
}

if [ "${1:-}" = "-h" ] || [ "${1:-}" = "--help" ]; then
  sed -n '2,12p' "$0"
  exit 0
fi
if [ "${1:-}" = "--list" ]; then
  for row in "${MATRIX[@]}"; do printf '%s\n' "$(row_id "$row")"; done
  exit 0
fi

SELECTED=()
if [ "$#" -eq 0 ]; then
  SELECTED=("${MATRIX[@]}")
else
  for arg in "$@"; do
    found=0
    for row in "${MATRIX[@]}"; do
      # A bare triple selects every row of that triple (baseline AND v3); the
      # "<triple>-v3" form selects exactly one row.
      if [ "$arg" = "${row%%|*}" ] || [ "$arg" = "$(row_id "$row")" ]; then
        SELECTED+=("$row")
        found=1
      fi
    done
    if [ "$found" -eq 0 ]; then
      echo "Unknown target: $arg  (try --list)" >&2
      exit 2
    fi
  done
fi

# ---------------------------------------------------------------------------
# Preflight
# ---------------------------------------------------------------------------
# Version comes from Cargo.toml so the artifact names cannot drift from the
# crate. -F'"' + a section guard: the first `version = "..."` under [package],
# not the one under some [dependencies] entry.
VERSION=$(awk -F'"' '/^\[/{s=$1} s ~ /^\[package\]/ && /^[[:space:]]*version[[:space:]]*=/{print $2; exit}' Cargo.toml)
if [ -z "$VERSION" ]; then
  echo "Could not read the package version from Cargo.toml" >&2
  exit 1
fi

# The cross rows depend on `linker = ...` entries from .cargo/config.toml — this
# script deliberately does not duplicate those, so the file has to exist.
if [ ! -f .cargo/config.toml ]; then
  echo "WARNING: .cargo/config.toml is missing — the cross targets have no linker" >&2
  echo "         configured and will almost certainly fail to link." >&2
fi

IS_DARWIN=0
if [ "$(uname -s)" = "Darwin" ]; then IS_DARWIN=1; fi

mkdir -p "$DIST"

BUILT=()
FAILED=()
SKIPPED=()

printf '\n\033[1mhew %s — %d artifact(s) selected\033[0m\n' "$VERSION" "${#SELECTED[@]}"

for row in "${SELECTED[@]}"; do
  triple="${row%%|*}"
  cpu="${row#*|}"
  id="$(row_id "$row")"

  printf '\n\033[1;34m== %s\033[0m\n' "$id"

  # ---- Darwin: not cross-buildable from NixOS -----------------------------
  # rust-overlay happily installs the *-apple-darwin std, but linking needs the
  # Apple SDK and cctools, neither of which can legally or practically be
  # provided by a Linux host. These two rows require a real Mac or a darwin CI
  # runner (see ci.plan.md).
  case "$triple" in
    *-apple-darwin)
      if [ "$IS_DARWIN" -eq 0 ]; then
        printf '\033[33mSKIP %s — Apple targets cannot be cross-linked from a non-darwin host\n' "$id"
        printf '     (needs the Apple SDK + cctools; build it on macOS or a darwin CI runner)\033[0m\n'
        SKIPPED+=("$id")
        continue
      fi
      ;;
  esac

  # Separate target dir per row. The baseline and v3 rows of the same triple
  # differ only in rustflags, so a shared target dir would invalidate the whole
  # dependency graph on every alternation and rebuild it from scratch twice per
  # run. Also keeps target/release/ — which benchmark.sh hardcodes — untouched.
  target_dir="$ROOT/target/release-matrix/$id"

  args=(build --release --locked --target "$triple" --target-dir "$target_dir")
  cfg="$(rustflags_config "$triple" "$cpu")"
  if [ -n "$cfg" ]; then
    args+=(--config "$cfg")
    echo "  rustflags: $cfg"
  fi

  if ! cargo "${args[@]}"; then
    printf '\033[31mFAIL %s — build\033[0m\n' "$id"
    FAILED+=("$id (build)")
    continue
  fi

  # ---- collect ------------------------------------------------------------
  ext=""
  case "$triple" in *-windows-*) ext=".exe" ;; esac
  src="$target_dir/$triple/release/$BIN_NAME$ext"
  if [ ! -f "$src" ]; then
    printf '\033[31mFAIL %s — expected binary not found at %s\033[0m\n' "$id" "$src"
    FAILED+=("$id (missing binary)")
    continue
  fi

  # hew-<version>-<triple>[-v3][.exe]
  suffix=""
  if [ -n "$cpu" ]; then suffix="-v3"; fi
  artifact="$BIN_NAME-$VERSION-$triple$suffix$ext"
  cp "$src" "$DIST/$artifact"
  chmod 0755 "$DIST/$artifact"

  # ---- assert -------------------------------------------------------------
  # PE (windows) and Mach-O (darwin) carry none of the ELF structures these
  # assertions inspect, so they are skipped there rather than faked.
  skip_assert=0
  mode=""
  case "$triple" in
    *-windows-*) skip_assert=1 ;;
    *-apple-darwin) skip_assert=1 ;;
    *-linux-musl) mode="static" ;;
  esac

  if [ "$skip_assert" -eq 1 ]; then
    echo "  (ELF assertions skipped — not an ELF artifact)"
  elif ! assert_elf "$DIST/$artifact" "$mode"; then
    # A soft binary must never reach dist/. Remove it so a partially failed run
    # cannot produce a SHA256SUMS entry for an unhardened artifact.
    rm -f "$DIST/$artifact"
    printf '\033[31mFAIL %s — hardening assertions\033[0m\n' "$id"
    FAILED+=("$id (hardening)")
    continue
  fi

  size=$(wc -c <"$DIST/$artifact")
  printf '\033[32mOK   %s  (%s bytes)\033[0m\n' "$artifact" "$size"
  BUILT+=("$artifact")
done

# ---------------------------------------------------------------------------
# Checksums. Regenerated over everything currently in dist/, not just this run's
# artifacts, so a partial rebuild leaves a consistent, complete SHA256SUMS.
# ---------------------------------------------------------------------------
# Plain file names (no ./ prefix) and glob order, so that `sha256sum -c
# SHA256SUMS` works unchanged from inside the directory a user downloaded into.
SUM_FILES=()
for f in "$DIST"/*; do
  base="${f##*/}"
  if [ -f "$f" ] && [ "$base" != "SHA256SUMS" ]; then
    SUM_FILES+=("$base")
  fi
done
if [ ${#SUM_FILES[@]} -gt 0 ]; then
  SUMS=(sha256sum)
  command -v sha256sum >/dev/null || SUMS=(shasum -a 256) # darwin has no sha256sum
  (cd "$DIST" && "${SUMS[@]}" "${SUM_FILES[@]}" >SHA256SUMS)
  printf '\n\033[1mdist/SHA256SUMS\033[0m\n'
  sed 's/^/  /' "$DIST/SHA256SUMS"
fi

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
echo
printf '\033[1mSummary\033[0m  built: %d   skipped: %d   failed: %d\n' \
  "${#BUILT[@]}" "${#SKIPPED[@]}" "${#FAILED[@]}"
if [ ${#BUILT[@]} -gt 0 ]; then printf '\033[32m  built:   %s\033[0m\n' "${BUILT[*]}"; fi
if [ ${#SKIPPED[@]} -gt 0 ]; then printf '\033[33m  skipped: %s\033[0m\n' "${SKIPPED[*]}"; fi
if [ ${#FAILED[@]} -gt 0 ]; then
  printf '\033[31m  failed:  %s\033[0m\n' "${FAILED[*]}"
  exit 1
fi
exit 0
