#!/usr/bin/env bash
# Minimal HTTP host (acceptance scenario I): capabilities → plan → execute →
# SSE → receipt. Requires a running daemon: `interact-ai serve`.
set -euo pipefail
HOME_DIR="${INTERACT_AI_HOME:-$HOME/.adaptive-interaction}"
BASE="${INTERACT_AI_API:-http://127.0.0.1:8787}"
TOKEN="$(cat "$HOME_DIR/state/api-token")"
auth=(-H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json")

echo "== capabilities =="
curl -sf "${auth[@]}" "$BASE/v1/capabilities" | head -c 400; echo

echo "== session =="
curl -sf "${auth[@]}" -X POST "$BASE/v1/session/start" -d '{"label":"http-host"}' >/dev/null

echo "== plan =="
PLAN=$(curl -sf "${auth[@]}" -X POST "$BASE/v1/plans" \
  -d '{"intent":"success","candidates":["conversation"],"minChannels":1,"maxChannels":1,"allowNoAction":false}')
PLAN_ID=$(echo "$PLAN" | python3 -c 'import json,sys; print(json.load(sys.stdin)["planId"])')
echo "plan: $PLAN_ID"

echo "== execute =="
RECEIPTS=$(curl -sf "${auth[@]}" -X POST "$BASE/v1/plans/$PLAN_ID/execute")
ACTION_ID=$(echo "$RECEIPTS" | python3 -c 'import json,sys; print(json.load(sys.stdin)[0]["actionId"])')
echo "action: $ACTION_ID"

echo "== SSE (3s) =="
curl -sf "${auth[@]}" -N --max-time 3 -H "Last-Event-ID: 0" "$BASE/v1/events" | grep "^event:" | sort | uniq -c || true

echo "== receipt =="
curl -sf "${auth[@]}" "$BASE/v1/actions/$ACTION_ID" | python3 -m json.tool | head -20
