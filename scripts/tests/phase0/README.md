# 階段 0 基線 runner（`scripts/tests/phase0/`）

這一組腳本是「階段 0：真實狀態恢復」的可重現基線與真 Agent E2E harness。它們只走**正式路徑**
（`interact-ai serve` 真 daemon＋HTTP API＋SSE），不注入狀態、不偽造完成事件；每一步都保留精確命令、
來源 commit、起訖時間、exit code 與 stdout/stderr。結果的分類與解讀寫在
`docs/releases/phase-0-e2e-baseline.md`，這裡只說怎麼跑。

| 腳本 | 做什麼 | 證據層級 |
|---|---|---|
| `run-step.sh` | 單步紀錄器：`PHASE0_OUT=<dir> run-step.sh <name> <workdir> <cmd…>` → 追加一行到 `summary.jsonl` | — |
| `baseline.sh` | 依風險順序跑 fmt／clippy／workspace test／CLI build／fake_iphone build／Tauri test／前端 typecheck·test·build／architecture 各組／CLI E2E／Playwright／iOS 模擬器／drills／ESP32 compile；可只指定步驟名 | unit／contract／integration／browser／simulator／fixture |
| `agent_smoke.py` | 單一真 Agent（`--agent claude-code\|codex`）：建 session→送任務→SSE＋輪詢狀態→（可選）核可／拒絕、interrupt／close、`--cancel-when-active`→記錄 record／mailbox／audit／子程序 | real-agent |
| `multi_session.py` | 同一 daemon 開 N 個 session（同種／異種），各自隔離 workdir＋唯一暗號，驗證不串線、單一取消不影響其他 | real-agent |
| `restart_test.py` | 真 Agent 工作中 SIGKILL／SIGTERM daemon → 重啟同一 home → 記錄 session 誠實狀態、孤兒子程序、mailbox／renew 行為 | real-agent |
| `archive-evidence.py` | 把 scratchpad 原始證據歸檔到 `docs/releases/evidence/<date>-phase-0/`，略過 `home/`、掃描憑證、產 `artifact-manifest.json` | — |

## 隔離與安全（每支腳本都遵守）

- 一律 `INTERACT_AI_HOME=<out>/home`（daemon 的 token、SQLite、config 都在裡面），`INTERACT_AI_MOBILE_ADVERTISE=0`；
  **不碰** `~/.adaptive-interaction`。埠由 `--port` 指定，config 由腳本寫在 `<out>/home/config/interaction.yaml`。
- 真 Agent 時腳本會清掉 `INTERACT_AI_CLAUDE_BIN`／`INTERACT_AI_CODEX_BIN`；要跑 fixture 就明確 export 到
  `crates/interaction-runtime/tests/fixtures/fake_{claude,codex}.sh`，而且結果必須標「fixture」。
- 任務一律低風險、可逆、只在 `<out>/workdir`（或 `work-N`）內；預設唯讀（`--allow-write` 才帶
  `toolScope=["workspace.write"]`＋`consentScope=["agent-session:workspace-write"]`）。
- 有界：`--timeout`（預設 240 s）、`ttlMinutes=20`、`maxMessages=40`；腳本結束會 terminate 自己起的 daemon。
- Codex 在唯讀 sandbox 下連 `cat NOTES.md` 都會要求核可（`waiting-for-consent`）；沒有 `--approve approve` 就會停在那裡
  直到 harness timeout——那是 correctly-blocked，不是失敗。
- 模型與推理設定**無法**經 gateway 指定或回報（能力矩陣 K-01／K-02／K-03）；實際模型只能事後從 provider 端本機
  紀錄（`~/.claude/projects/<workdir>/<providerSessionId>.jsonl`、`~/.codex/sessions/…/rollout-*-<providerSessionId>.jsonl`）
  唯讀取得，記錄時要標「provider-local-log (not via gateway)」。

## 例子

```bash
PHASE0_OUT=/tmp/phase0-baseline scripts/tests/phase0/baseline.sh                 # 全套
PHASE0_OUT=/tmp/phase0-baseline scripts/tests/phase0/baseline.sh rust-fmt fe-test # 只跑兩步
python3 scripts/tests/phase0/agent_smoke.py --agent claude-code --port 19010 --out /tmp/p0/claude-smoke
python3 scripts/tests/phase0/agent_smoke.py --agent codex --port 19011 --out /tmp/p0/codex-smoke --approve approve
python3 scripts/tests/phase0/agent_smoke.py --agent claude-code --port 19012 --out /tmp/p0/claude-cancel \
  --task "請用繁體中文寫一篇約 1500 字的短文…" --cancel-after 3 --cancel-when-active --cancel-mode interrupt --close-after-cancel 5
python3 scripts/tests/phase0/multi_session.py --agents claude-code,codex --port 19013 --out /tmp/p0/multi-E --cancel-index 0 --approve
python3 scripts/tests/phase0/restart_test.py --agent claude-code --port 19014 --out /tmp/p0/restart --signal KILL
```

`baseline.sh` 各步的 passed／failed 數字要從各自的 stdout 讀：Rust／Tauri 看 `test result:` 行的加總、vitest 看
`Tests … passed`、Playwright 看 `N passed`、CLI E2E 看 `RESULT: N passed, M failed`、iOS 看 `scripts/tests/ios-simulator.sh`
輸出的 JSON、architecture 看每組的 PASS／FAIL／SKIP 表（`SKIP` 不算通過）。
