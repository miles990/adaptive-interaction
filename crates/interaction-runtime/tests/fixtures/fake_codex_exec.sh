#!/usr/bin/env bash
# Fake Codex WITHOUT app-server: forces the `codex exec --json` fallback,
# where one message == one bounded subprocess and a second message during a
# running turn is refused with GatewayError::Busy. One turn takes a while
# (sleep) so a test can send a second message while the first is running and
# assert that "not delivered" is never turned into "agent failed".
case "$1" in
  --version) echo "fake-codex-exec 0.1"; exit 0 ;;
  login) echo "Logged in as fake@example.test"; exit 0 ;;
  app-server) exit 1 ;;
esac
if [ "$1" = "exec" ] && [ "$2" = "--help" ]; then exit 0; fi
if [ "$1" != "exec" ]; then exit 1; fi

# cwd == the session workdir (`--cd`), so the pid file stays scoped to one test.
echo $$ > ./fake-pid
echo '{"type":"thread.started","thread_id":"exec-thread-1"}'
echo '{"type":"turn.started"}'
sleep "${FAKE_EXEC_TURN_SECS:-1.5}"
echo '{"type":"item.completed","item":{"type":"agent_message","text":"慢慢做完了（這是聲稱）"}}'
echo '{"type":"turn.completed","usage":{"input_tokens":2,"output_tokens":1}}'
