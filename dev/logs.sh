#!/usr/bin/env bash
# Emits fake JSON logs, one per line, until killed (Ctrl-C).
# Usage: ./genlogs.sh | myRustBinary
#        ./genlogs.sh 0.1 | myRustBinary   # 100ms between lines

DELAY="${1:-1}"

LEVELS=(DEBUG INFO INFO INFO WARN ERROR)
PACKAGES=(auth db http cache worker)
MESSAGES=(
  "request handled"
  "connection established"
  "cache miss"
  "retrying upstream call"
  "user session expired"
  "query took longer than expected"
  "user 69 requested a refund"
)

while true; do
  ts=$(date -u +%Y-%m-%dT%H:%M:%SZ)
  level=${LEVELS[$RANDOM % ${#LEVELS[@]}]}
  pkg=${PACKAGES[$RANDOM % ${#PACKAGES[@]}]}
  msg=${MESSAGES[$RANDOM % ${#MESSAGES[@]}]}

  printf '{"timestamp":"%s","level":"%s","package":"%s","message":"%s"}\n' \
    "$ts" "$level" "$pkg" "$msg"

  sleep "$DELAY"
done
