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

# 真 Claude Code 2.1.272 在 `-p --input-format stream-json --output-format
# stream-json` 之下被 SIGINT 中斷時的輸出形狀（階段 1 的 real-agent trace
# harness 實測）：它先吐一行 result，`is_error` 為真、`subtype` 是
# `error_during_execution`，而且**沒有** `result` 字串欄位（所以連接器讀到的
# 錯誤摘要就只是 "error_during_execution"），然後才收場。階段 0 的 D1 就是
# 照字面把這一行翻成「失敗」。
emit_interrupted_result() {
  echo '{"type":"result","subtype":"error_during_execution","is_error":true,"duration_ms":1200,"num_turns":1,"session_id":"fake-123"}'
  exit 0
}

TURN=0
while IFS= read -r _line; do
  if [ -n "$INPUT_FILE" ]; then printf '%s\n' "$_line" >> "$INPUT_FILE"; fi
  TURN=$((TURN + 1))
  case "$MODE" in
    # 長任務＋真機形狀的中斷：先回報進度（session 因此進入 working／Active），
    # 再等很久；收到訊號時照真 Claude Code 的樣子先印 result 行才退出。
    # 不改 `hang`：那個分支被 estop／shutdown／「中斷只是請求」等測試當成
    # 「永遠不回話、只能被殺掉」的子程序，加上 trap 會改掉它們的前提。
    long-task)
      trap emit_interrupted_result INT TERM
      echo '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"working on it"}]}}'
      sleep 3600 &
      wait $!
      ;;
    hang)
      sleep 3600
      ;;
    # Multi-turn honesty: the FIRST turn claims completion like a healthy
    # agent; the SECOND turn dies. `claim-then-crash` dies with an observable
    # error (stderr + exit 3), `claim-then-silent` exits 0 having claimed
    # nothing — the first turn's result must not vouch for the second.
    claim-then-crash|claim-then-silent)
      if [ "$TURN" -eq 1 ]; then
        echo '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"working on it"}]}}'
        echo '{"type":"result","subtype":"success","is_error":false,"result":"第一輪完成了（這是聲稱）","total_cost_usd":0.01,"num_turns":1}'
      else
        echo '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"second round"}]}}'
        if [ "$MODE" = "claim-then-crash" ]; then
          echo 'fake-claude: model backend exploded on round two' >&2
          exit 3
        fi
        exit 0
      fi
      ;;
    # Claims completion on the first turn, then the process ends on its own
    # (exit 0): the claim stands, but nothing can ever be delivered again.
    claim-then-exit)
      echo '{"type":"result","subtype":"success","is_error":false,"result":"做完就走了（這是聲稱）","total_cost_usd":0.01,"num_turns":1}'
      exit 0
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
