#!/usr/bin/env bash
# One-command dev launcher for the fork (git-bash / msys on Windows).
#
#   bash scripts/dev-app.sh
#
# What it does:
#   1. starts the Vite dev server on :1420 if nothing is listening there
#      (the debug build loads this devUrl);
#   2. exports TUTORIAL_LLM_API_KEY from secret/key.txt when present
#      (the file is git-ignored; the key never appears on the command
#      line or in logs);
#   3. runs the app: cargo run --bin akagi (--bin is required since
#      tutorial_debug joined the crate).
set -euo pipefail
cd "$(dirname "$0")/.."

if ! curl -s -o /dev/null http://localhost:1420/; then
  echo "[dev-app] starting Vite on :1420 ..."
  (cd frontend && npm run dev >/tmp/akagi-vite.log 2>&1 &
   echo $! > /tmp/akagi-vite.pid)
  for _ in $(seq 1 30); do
    curl -s -o /dev/null http://localhost:1420/ && break
    sleep 1
  done
  curl -s -o /dev/null http://localhost:1420/ || { echo "[dev-app] Vite failed to come up (see /tmp/akagi-vite.log)"; exit 1; }
else
  echo "[dev-app] Vite already running on :1420"
fi

if [[ -f secret/key.txt ]]; then
  export TUTORIAL_LLM_API_KEY="$(sed 's/^GLM_KEY:[[:space:]]*//' secret/key.txt | tr -d '\r\n')"
  echo "[dev-app] TUTORIAL_LLM_API_KEY loaded from secret/key.txt (${#TUTORIAL_LLM_API_KEY} chars)"
else
  echo "[dev-app] note: secret/key.txt absent — LLM features fall back to the config key"
fi

export PATH="$USERPROFILE/.cargo/bin:$PATH"
echo "[dev-app] launching akagi ..."
exec cargo run --bin akagi
