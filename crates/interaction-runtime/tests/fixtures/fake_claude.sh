#!/usr/bin/env bash
# Fake Claude Code CLI for gateway integration tests.
# Speaks just enough stream-json; honors discover subcommands.
case "$1" in
  --version) echo "fake-claude 1.0.0 (Claude Code)"; exit 0 ;;
  auth) echo '{"loggedIn": true, "authMethod": "fake"}'; exit 0 ;;
esac
# session mode (-p --input-format stream-json ...)
if [ -n "${FAKE_PID_FILE:-}" ]; then echo $$ > "$FAKE_PID_FILE"; fi
echo '{"type":"system","subtype":"init","session_id":"fake-123","model":"fake-model"}'
while IFS= read -r _line; do
  if [ "${FAKE_MODE:-}" = "hang" ]; then
    sleep 3600
  else
    echo '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"working on it"}]}}'
    echo '{"type":"result","subtype":"success","is_error":false,"result":"完成了（這是聲稱）","total_cost_usd":0.01,"num_turns":1}'
  fi
done
