#!/usr/bin/env bash
# Fake Claude Code CLI for gateway integration tests.
# Speaks just enough stream-json; honors discover subcommands.
case "$1" in
  --version) echo "fake-claude 1.0.0 (Claude Code)"; exit 0 ;;
  auth) echo '{"loggedIn": true, "authMethod": "fake"}'; exit 0 ;;
esac
# session mode (-p --input-format stream-json ...)
if [ -n "${FAKE_PID_FILE:-}" ]; then echo $$ > "$FAKE_PID_FILE"; fi
if [ -n "${FAKE_ENV_STATUS_FILE:-}" ]; then
  if [ -n "${INTERACT_AI_SESSION_TOKEN:-}" ] && [ -n "${INTERACT_AI_API_URL:-}" ]; then
    printf 'scoped-session-capability-present\n' > "$FAKE_ENV_STATUS_FILE"
  else
    printf 'scoped-session-capability-missing\n' > "$FAKE_ENV_STATUS_FILE"
  fi
fi
echo '{"type":"system","subtype":"init","session_id":"fake-123","model":"fake-model"}'
while IFS= read -r _line; do
  if [ -n "${FAKE_INPUT_FILE:-}" ]; then printf '%s\n' "$_line" >> "$FAKE_INPUT_FILE"; fi
  if [ "${FAKE_MODE:-}" = "hang" ]; then
    sleep 3600
  elif [ "${FAKE_MODE:-}" = "proactive" ]; then
    echo '{"type":"result","subtype":"success","is_error":false,"result":"{\"intent\":\"request_attention\",\"message\":\"有一項低風險建議，想看時再點我。\",\"tone\":\"attentive\",\"behaviorIntent\":\"notice\",\"priority\":\"normal\",\"expiresInSeconds\":60}","total_cost_usd":0.01,"num_turns":1}'
  else
    echo '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"working on it"}]}}'
    echo '{"type":"result","subtype":"success","is_error":false,"result":"完成了（這是聲稱）","total_cost_usd":0.01,"num_turns":1}'
  fi
done
