#!/usr/bin/env bash
# Throughput + stutter benchmark for hew.
#
# Generates a corpus of realistic JSON log lines (hardcoded timestamp — no
# `date` call per line), then feeds it to the binary in a loop for DURATION
# seconds. Lines are counted on the *producer* side, because hew expands
# escaped "\n" in messages into real newlines, so counting its output lines
# would overcount.
#
# Stutter detection: the pipe gives us backpressure — `cat` can only hand
# over a chunk as fast as hew consumes the previous bytes (modulo the ~64KB
# pipe buffer, which is negligible against a multi-MB chunk). So we timestamp
# every chunk hand-off; a chunk that takes much longer than the median is a
# stall in the consumer.
#
# Usage: ./benchmark.sh [duration-seconds]

set -euo pipefail
cd "$(dirname "$0")"

DURATION="${1:-30}"
CHUNK_LINES=2000        # lines per corpus chunk; 1 in 10 carries a stack trace
BIN=target/release/hew
BENCH_DIR=target/bench
CHUNK="$BENCH_DIR/chunk.jsonl"
TIMES="$BENCH_DIR/chunk-times.ns"

mkdir -p "$BENCH_DIR"

if [[ ! -x "$BIN" ]]; then
  echo "building release binary..."
  nix develop -c cargo build --release
fi

# --- corpus generation (once; reused across runs) -------------------------

if [[ ! -f "$CHUNK" ]] || [[ "$(wc -l < "$CHUNK")" -ne "$CHUNK_LINES" ]]; then
  echo "generating corpus chunk ($CHUNK_LINES lines)..."

  # 100-frame Java stack trace as a single JSON string (literal \n and \t
  # escapes — hew's JSON parser turns them back into real newlines/tabs).
  stack='java.lang.RuntimeException: upstream call failed: connection reset by peer'
  for ((i = 1; i <= 100; i++)); do
    stack+='\n\tat com.example.service.OrderPipeline$AsyncStage.lambda$process$'"$i"'(OrderPipeline.java:'"$((100 + i * 7))"')'
  done

  filler='order=7f3a9c21-4b8e-4d2a-b1c9-0e5f6a7b8c9d user=svc-checkout region=eu-central-1 attempt=3 upstream=payments-gateway latency_ms=8412 trace=00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01'
  levels=(DEBUG INFO INFO INFO WARN ERROR)

  : > "$CHUNK"
  for ((i = 0; i < CHUNK_LINES; i++)); do
    level=${levels[i % 6]}
    if ((i % 10 == 0)); then
      msg="request failed, dumping stack $filler $filler\n$stack"
    else
      msg="request handled with retries and partial degradation $filler $filler $filler"
    fi
    printf '{"timestamp":"2026-08-11T12:00:00.000Z","level":"%s","package":"bench","message":"%s"}\n' \
      "$level" "$msg" >> "$CHUNK"
  done
  echo "chunk size: $(du -h "$CHUNK" | cut -f1)"
fi

# --- benchmark run --------------------------------------------------------

echo "running for ${DURATION}s..."
: > "$TIMES"

feed() {
  local end=$((SECONDS + DURATION))
  while ((SECONDS < end)); do
    cat "$CHUNK"
    date +%s%N >> "$TIMES"
  done
}

feed | "$BIN" > /dev/null

# --- report ---------------------------------------------------------------

awk -v chunk_lines="$CHUNK_LINES" -v duration="$DURATION" '
  { t[NR] = $1 }
  END {
    if (NR < 3) { print "not enough chunks completed to report"; exit 1 }
    total_lines = NR * chunk_lines
    wall = (t[NR] - t[1]) / 1e9

    # per-chunk durations (skip the first hand-off, it has no predecessor)
    n = 0
    for (i = 2; i <= NR; i++) d[++n] = (t[i] - t[i-1]) / 1e9
    asort(d, sorted)
    median = sorted[int(n / 2) + 1]
    worst = sorted[n]

    # per-second buckets for min/max throughput (drop the partial last second)
    for (i = 2; i <= NR; i++) bucket[int((t[i] - t[1]) / 1e9)] += chunk_lines
    last = int((t[NR] - t[1]) / 1e9)
    if (last in bucket && length(bucket) > 1) delete bucket[last]
    min_r = -1; max_r = 0
    for (b in bucket) {
      if (bucket[b] > max_r) max_r = bucket[b]
      if (min_r < 0 || bucket[b] < min_r) min_r = bucket[b]
    }

    # stutter: chunks that took > 2x median
    stutters = 0
    for (i = 1; i <= n; i++) if (d[i] > 2 * median) stutters++

    printf "\n"
    printf "lines parsed:      %d in %.1fs\n", total_lines, wall
    printf "throughput:        %d lines/s average\n", total_lines / wall
    printf "per-second range:  %d .. %d lines/s\n", min_r, max_r
    printf "median chunk:      %.1f ms  (%d lines)\n", median * 1000, chunk_lines
    printf "worst chunk:       %.1f ms\n", worst * 1000
    printf "stutters (>2x median chunk time): %d of %d chunks\n", stutters, n
  }
' "$TIMES"
