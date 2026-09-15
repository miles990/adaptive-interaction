# 階段 1 真 Agent 追蹤驗收（`scripts/tests/phase1/`）

這支腳本驗收「階段 1：互動可追溯性」寫下的紀錄，在**真的**一次互動裡是不是真的出現、順序對不對、
會不會外洩不該外洩的東西。它只走**正式路徑**（`interact-ai serve` 真 daemon＋HTTP API），不注入
狀態、不偽造完成事件；契約與 kind／outcome 的權威定義見 [`docs/aip/interaction-tracing.md`](../../../docs/aip/interaction-tracing.md)，
逐領域覆蓋矩陣見 [`docs/releases/phase-1-coverage-matrix.md`](../../../docs/releases/phase-1-coverage-matrix.md)。

## `trace_e2e.py` 用法

```bash
python3 scripts/tests/phase1/trace_e2e.py --agent claude-code --port 19100 --out /tmp/phase1-claude
python3 scripts/tests/phase1/trace_e2e.py --agent codex --port 19101 --out /tmp/phase1-codex \
  --scenarios normal,cancel,resume,failure,approval,restart
```

| 參數 | 說明 |
|---|---|
| `--agent` | `claude-code` 或 `codex`（必填）。`approval` 情境只在 `codex` 上跑（claude-code 沒有等價的核可流程） |
| `--port` | daemon 監聽埠（必填，避免與其他跑著的 daemon 衝突） |
| `--out` | 輸出目錄（必填）：`home/`（隔離的 `INTERACT_AI_HOME`）、`work-<name>/`（各情境各自的 workdir）、`daemon.log`、`result.json` |
| `--scenarios` | 逗號分隔，預設全跑：`normal,cancel,resume,failure,approval,restart` |
| `--timeout` | 單一等待步驟的秒數上限，預設 240 |

## 六個情境驗什麼

| 情境 | 動作 | 對照本輪的哪個 kind／outcome |
|---|---|---|
| `normal` | 建立 session→送任務→等終態→若 `claimed-completed` 則人工 `verify`→`close` | `agent-session.capability-issued`／`.dispatched`／`.task-delivered`／`.outcome`（`claimed`）／`.verified`／`.closed` 依序出現 |
| `cancel` | 送長任務、等 `active` 三秒後 `POST /interrupt`、等終態、`close` | `agent-session.interrupt-requested`（`accepted`／`interrupt.sent`）＋`agent-session.outcome`（`cancelled`／`outcome.cancelled`） |
| `resume` | 需要 `normal` 先跑過並留下 `providerSessionId`，否則整段 `not-run`。分兩個 `result.json` 鍵：`resume-reject`（放寬 `ttlMinutes` 去 resume，**應該被拒絕**）與 `resume-accept`（用完全相同的授權去 resume，**應該被接受**） | `resume-reject`：`agent-session.resume-checked`（`rejected`），session **不得**真的建立；`resume-accept`：`agent-session.resume-checked`（`accepted`／`resume.ok`），`trace_id` 指回原 session |
| `failure` | claude-code：把 `maxCost` 設到 0.0001 逼出 provider 端預算錯誤；codex：對子程序樹送 `SIGKILL` | `agent-session.outcome`（`failed`／`outcome.connector-error`，或 codex 的 `unknown`／`failed`，誠實視實際終態而定） |
| `approval` | 只在 codex：等 `waiting-for-consent`，人工拒絕核可請求 | `agent.approval`（`rejected`／`approval.human-denied`） |
| `restart` | 送長任務、等 `active`、`SIGKILL` daemon、原地重啟 | `agent-session.outcome`（`unknown`／`runtime.restarted`），重啟後 `GET /v1/agent-sessions/{id}` 回 `expired` |

跑完所有指定情境後，腳本再驗兩件跨情境的事：

- **隔離**：任一 session 的 `GET /v1/trace?sessionId=` 結果裡，`sessionId` 欄位不得出現別的 session id（不串線）。
- **權限**：若 `home/state/api-agent-token` 存在，用它打 `GET /v1/trace` 必須是 `403`（AI token 讀不到追蹤紀錄）。

## 隔離與安全（沿用 `scripts/tests/phase0/README.md` 的規則）

- 一律 `INTERACT_AI_HOME=<out>/home`，`INTERACT_AI_MOBILE_ADVERTISE=0`；**不碰** `~/.adaptive-interaction`。
  埠由 `--port` 指定，config 由腳本寫在 `<out>/home/config/interaction.yaml`。
- 啟動前清掉 `INTERACT_AI_CLAUDE_BIN`／`INTERACT_AI_CODEX_BIN`：一律走真的已登入 `claude`／`codex` 二進位，不用 fixture。
- 任務內容固定兩種（唯讀讀 `NOTES.md`、或約 2500 字的長文寫作），全部在 `<out>/work-<name>/` 內；
  `allowWrite` 一律 `false`（本輪驗的是追蹤紀錄，不需要寫入權限）。
- 有界：預設 `--timeout 240`、`ttlMinutes=20`、`maxMessages=40`。
- 每個情境結束都會呼叫 `close`（`restart` 情境例外——重啟本身就是它要驗的事）；腳本結束時終止自己起的 daemon（`SIGTERM`，等 15 秒後 `SIGKILL`）。
- Codex 在唯讀 sandbox 下即使只是 `cat NOTES.md` 也可能要求核可（`waiting-for-consent`）；`approval` 情境正是利用這一點。

## 結果分類

每個情境的結果寫進 `result.json` 的 `scenarios.<name>`，`verdict` 是下列四種之一：

| verdict | 意思 |
|---|---|
| `passed` | 期望出現的 kind／outcome 都出現了，且沒有敏感字串外洩（`iat-session-`／`Bearer `／`sk-ant`／本機使用者路徑） |
| `product-failed` | 跑完了，但缺紀錄、outcome 不對、或有敏感字串外洩——這是產品缺陷，不是 harness 問題 |
| `not-run` | 情境的前置條件不成立而跳過（例如 `resume` 需要 `normal` 先留下 `providerSessionId`） |
| `blocked` | `/v1/trace` 或 `/v1/agent-sessions/{id}/activity` 回 404（端點不存在），連驗都驗不了 |

`result.json` 也帶 `commit`（`git rev-parse HEAD`）、`worktreeDirty`（`git status --porcelain` 是否非空）、
`binarySha256`（`target/debug/interact-ai` 的雜湊）、`versions`（`interact-ai`／`claude`／`codex`／OS）、
`statusAfter`（收尾時的 `/v1/status` 摘要，含 `traceWriteFailures`／`traceCounts`）與 `events`（時間序的
細粒度事件 log，`mark()` 逐一寫入，標準輸出同步印一份）。

**執行後**：把每個情境的 `verdict` 逐一對照 `docs/releases/phase-1-coverage-matrix.md` §4／§5 對應列，
回填 `docs/releases/phase-1-progress.md` §5／§8 與 `docs/acceptance-evidence.md` 階段 1 節第 10 項
（本文件與 README 撰寫時，`trace_e2e.py` 尚未執行，那幾處留的是「待填：主模型跑完後補」）。
