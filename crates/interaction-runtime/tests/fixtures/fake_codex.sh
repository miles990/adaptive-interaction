#!/usr/bin/env bash
# Fake `codex app-server` for gateway approval tests: enough JSON-RPC to get a
# thread open and then raise ONE approval ServerRequest, so the runtime's
# "nobody decided in time ⇒ deny" watchdog can be exercised end to end.
#
# It records the decision line it actually receives on stdin, so a test can
# assert the deny was really delivered to the agent — not merely logged.
case "$1" in
  --version) echo "fake-codex 0.149.1"; exit 0 ;;
  login) echo "Logged in as fake@example.test"; exit 0 ;;
esac

if [ "$1" = "app-server" ] && [ "$2" = "--help" ]; then
  echo "usage: codex app-server"
  exit 0
fi
# Force the app-server path: the exec fallback must not be picked up here.
if [ "$1" = "exec" ]; then exit 1; fi
if [ "$1" != "app-server" ]; then exit 1; fi

# cwd == the session workdir, so these stay scoped to one test.
# Optional scenario file (workdir-scoped, like fake_claude.sh):
#   deaf-after-approval — after raising the approval request, close stdin and
#   stay alive: every decision write must fail, and the runtime must keep the
#   request pending instead of pretending the deny went through.
#   turns — no approval request; a real turn lifecycle instead: `turn/start`
#   answers with `turn/started`, and `turn/interrupt` ends the turn the way the
#   real app-server does — a `turn/completed` whose `turn.status` is
#   `interrupted` (there is no `turn/failed` method in the protocol).
MODE=""
if [ -f ./fake-mode ]; then MODE=$(cat ./fake-mode); fi
echo $$ > ./fake-pid
DECISION_FILE=./fake-approval-decision
# The raw `thread/start` / `thread/resume` params, so a test can assert the
# permission flags were re-sent on resume instead of being inherited.
START_FILE=./fake-thread-start
RESUME_FILE=./fake-thread-resume

while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id"[[:space:]]*:[[:space:]]*\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *'"method":"initialize"'*)
      printf '{"jsonrpc":"2.0","id":%s,"result":{"serverInfo":{"name":"fake-codex"}}}\n' "$id"
      ;;
    *'"method":"thread/start"'*)
      printf '%s\n' "$line" > "$START_FILE"
      printf '{"jsonrpc":"2.0","id":%s,"result":{"thread":{"id":"fake-thread-1"}}}\n' "$id"
      if [ "$MODE" = "turns" ]; then continue; fi
      # An approval request nobody is going to answer.
      printf '{"jsonrpc":"2.0","id":9001,"method":"item/commandExecution/requestApproval","params":{"command":"rm -rf /tmp/definitely-not"}}\n'
      if [ "$MODE" = "deaf-after-approval" ]; then
        exec 0<&-
        echo closed > ./fake-stdin-closed
        sleep 3600
      fi
      ;;
    *'"method":"thread/resume"'*)
      printf '%s\n' "$line" > "$RESUME_FILE"
      printf '{"jsonrpc":"2.0","id":%s,"result":{"thread":{"id":"fake-thread-resumed"}}}\n' "$id"
      ;;
    *'"method":"turn/start"'*)
      if [ "$MODE" = "turns" ]; then
        printf '{"jsonrpc":"2.0","method":"turn/started","params":{"threadId":"fake-thread-1","turn":{"id":"turn-1","items":[],"status":"inProgress"}}}\n'
        printf '{"jsonrpc":"2.0","method":"item/completed","params":{"threadId":"fake-thread-1","turnId":"turn-1","item":{"type":"agentMessage","id":"m1","text":"working on it"}}}\n'
      fi
      ;;
    *'"method":"turn/interrupt"'*)
      printf '%s\n' "$line" > ./fake-turn-interrupt
      if [ "$MODE" = "turns" ]; then
        printf '{"jsonrpc":"2.0","id":%s,"result":{}}\n' "$id"
        printf '{"jsonrpc":"2.0","method":"turn/completed","params":{"threadId":"fake-thread-1","turn":{"id":"turn-1","items":[],"status":"interrupted"}}}\n'
      fi
      ;;
    *'"decision"'*)
      printf '%s\n' "$line" > "$DECISION_FILE"
      ;;
  esac
done
