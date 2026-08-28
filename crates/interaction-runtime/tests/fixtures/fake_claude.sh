#!/usr/bin/env bash
# Fake Claude Code CLI for gateway integration tests.
# Speaks just enough stream-json; honors discover subcommands.
case "$1" in
  --version) echo "fake-claude 1.0.0 (Claude Code)"; exit 0 ;;
  auth) echo '{"loggedIn": true, "authMethod": "fake"}'; exit 0 ;;
esac

# Scenario selection. A `fake-mode` file inside the session workdir wins over
# the process-global FAKE_* env vars: env vars are shared by every test thread,
# so a workdir-scoped scenario is the only way a test can be sure it is driving
# ITS OWN subprocess. Paths in this branch are workdir-relative (cwd == workdir).
MODE=""
PID_FILE=""
INPUT_FILE=""
ARGV_FILE=""
ENV_STATUS_FILE=""
if [ -f ./fake-mode ]; then
  MODE=$(cat ./fake-mode)
  PID_FILE=./fake-pid
  INPUT_FILE=./fake-input
  ARGV_FILE=./fake-argv
else
  MODE="${FAKE_MODE:-}"
  PID_FILE="${FAKE_PID_FILE:-}"
  INPUT_FILE="${FAKE_INPUT_FILE:-}"
  ENV_STATUS_FILE="${FAKE_ENV_STATUS_FILE:-}"
fi

if [ -n "$ARGV_FILE" ]; then
  printf '%s\n' "$*" > "$ARGV_FILE"
fi
if [ -n "$PID_FILE" ]; then
  echo $$ > "$PID_FILE"
fi
if [ -n "$ENV_STATUS_FILE" ]; then
  if [ -n "${INTERACT_AI_SESSION_TOKEN:-}" ] && [ -n "${INTERACT_AI_API_URL:-}" ]; then
    printf 'scoped-session-capability-present\n' > "$ENV_STATUS_FILE"
  else
    printf 'scoped-session-capability-missing\n' > "$ENV_STATUS_FILE"
  fi
fi

echo '{"type":"system","subtype":"init","session_id":"fake-123","model":"fake-model"}'

case "$MODE" in
  # Exits cleanly, having claimed nothing at all → the outcome is UNKNOWN.
  silent)
    exit 0
    ;;
  # An observable error: non-zero exit + stderr → honestly FAILED.
  crash)
    echo 'fake-claude: model backend exploded' >&2
    exit 3
    ;;
  # Alive but stops reading stdin: writes to its stdin must fail, so nothing
  # may ever be reported as delivered/fetched.
  deaf)
    exec 0<&-
    echo closed > ./fake-stdin-closed
    sleep 3600
    ;;
esac

while IFS= read -r _line; do
  if [ -n "$INPUT_FILE" ]; then printf '%s\n' "$_line" >> "$INPUT_FILE"; fi
  case "$MODE" in
    hang)
      sleep 3600
      ;;
    proactive)
      echo '{"type":"result","subtype":"success","is_error":false,"result":"{\"intent\":\"request_attention\",\"message\":\"有一項低風險建議，想看時再點我。\",\"tone\":\"attentive\",\"behaviorIntent\":\"notice\",\"priority\":\"normal\",\"expiresInSeconds\":60}","total_cost_usd":0.01,"num_turns":1}'
      ;;
    slow)
      # Deliberate gap between "the task reached the agent" (fetched) and "the
      # agent started working" (working), so their ordering is observable.
      sleep 0.4
      echo '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"working on it"}]}}'
      echo '{"type":"result","subtype":"success","is_error":false,"result":"完成了（這是聲稱）","total_cost_usd":0.01,"num_turns":1}'
      ;;
    *)
      echo '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"working on it"}]}}'
      echo '{"type":"result","subtype":"success","is_error":false,"result":"完成了（這是聲稱）","total_cost_usd":0.01,"num_turns":1}'
      ;;
  esac
done
