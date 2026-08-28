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
      # An approval request nobody is going to answer.
      printf '{"jsonrpc":"2.0","id":9001,"method":"item/commandExecution/requestApproval","params":{"command":"rm -rf /tmp/definitely-not"}}\n'
      ;;
    *'"method":"thread/resume"'*)
      printf '%s\n' "$line" > "$RESUME_FILE"
      printf '{"jsonrpc":"2.0","id":%s,"result":{"thread":{"id":"fake-thread-resumed"}}}\n' "$id"
      ;;
    *'"decision"'*)
      printf '%s\n' "$line" > "$DECISION_FILE"
      ;;
  esac
done
