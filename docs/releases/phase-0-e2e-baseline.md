# 階段 0：雙平台與多 Session 實際情境 E2E 基線

> **這份文件是什麼**：階段 0 用**真的 Claude Code 與真的 Codex**（不是 fixture）、模擬 iPhone fixture、
> 以及 2026-09-06 建置的原生桌面候選 App，跑 11 個使用者情境（E0-01～E0-11）之後的逐案判讀。
> 它同時是「哪些事真的成立」與「哪些事只是看起來成立」的分帳：每一案分成
> Claude Code／Codex／fixture／原生桌面／真 iPhone 五欄各判一次，再給一個 overall。
>
> **怎麼讀**：先看 §2 總表（只有裁決值），需要細節再跳到 §4 的對應小節；要追原始檔就照 §2.2／§4 的證據路徑，
> 路徑一律相對於 `docs/releases/`，逐檔 sha256 在
> [`evidence/2026-09-07-phase-0/artifact-manifest.json`](evidence/2026-09-07-phase-0/artifact-manifest.json)（588 筆）。
> 缺陷編號 D1–D20 的逐條重現步驟在 [已知問題可重現性](phase-0-known-issues-reproducibility.md)（§4），
> 環境阻礙 B1–B6 在同一份文件的 §5；後續階段與下一個實作切片在 [roadmap](phase-0-roadmap.md)。
> 這份文件**不重抄**那兩份的內容，只指過去。
>
> **分類只用六個值**：
>
> | 值 | 意思 |
> |---|---|
> | `completed` | 投影（API／SSE／UI）＋核心狀態（record／status）＋實際效果（檔案／程序／provider 端紀錄）**三層齊全** |
> | `correctly-blocked` | 被正確阻擋，而且阻擋本身有證據（4xx／工具未提供／檔案沒落地） |
> | `product-failed` | 產品行為錯（分類不誠實、wire 值不合法、投影與 record 矛盾），不是工具或環境的錯 |
> | `harness-failed` | 測試設備（AX helper、fixture、driver 上限）先壞掉，產品行為**沒有被測到** |
> | `not-implemented` | 能力不存在（原始碼層級確認） |
> | `needs-environment` | 缺裝置／憑證／額度／並行度才能驗，不是程式問題 |
>
> 另有兩個**狀態標記**（不是裁決值）：`not-run`＝本輪完全沒跑該項；`n/a`＝該平台在這一案沒有對應行為。
>
> **`completed` 的門檻**：agent 說完成、UI 顯示成功、`exit 0` 都**不算**。`claimed-completed` 是**聲稱**不是驗證：
> record 上出現 `humanVerified` 只發生在人類 `POST /verify` 之後（R4 實測），而 `state` 仍維持 `claimed-completed`。
> fixture／模擬器結果一律標 fixture，不頂替真機；沒有跑就標 `not-run`／`needs-environment`／`harness-failed`，
> **「腳本已經修好」不等於「已通過」**。
>
> **模型只能唯讀自 provider 端本機紀錄**：gateway 既不能指定模型也不能回報模型（能力矩陣 K-01／K-02／K-03 = absent），
> 所以 §7 每一列的 `requestedModel` 一律 `not-specifiable-via-gateway`，`actualModel` 一律標
> `provider-local-log (not via gateway)`；那些 provider 紀錄（`~/.claude/projects/…jsonl`、`~/.codex/sessions/…rollout-*.jsonl`）
> 依歸檔規則**未歸檔**，本文件引用它們時一律明寫「未歸檔」。

## 1. 環境與身分

| 項目 | 值 |
|---|---|
| source commit | `78dcda1a3733c97d266ca9b60ad4461c69ca2032`（= `origin/main`；v0.8.0 tag `1fa69b8` 之後只有 4 個 docs／test commit） |
| daemon binary | `target/debug/interact-ai`，`interact-ai 0.8.0`，sha256 `1e066d84d28b9397c6c6843522ec6a0fbaf692d10ffbea59481afb79860c84c7`（2026-09-07 由 HEAD 建置；E2E 期間未重建、未改任何 repo 產品檔） |
| 第二顆 daemon binary（uncertain） | 上一個 session 在 [進度入口 §1](phase-0-progress.md) 與 [Repository 真實狀態 §6](phase-0-repository-state.md) 記下同一棵樹於 2026-09-08 17:29 重建出 `9b013e95…`，但證據目錄裡**沒有任何檔案含這個 hash**（只有 `1e066d84…` 有 `evidence/2026-09-07-phase-0/analysis/commands.json` 的 `binary-sha256` 佐證），而且 H 輪的內嵌時間戳全是 2026-09-07T10:20–10:24Z，早於那次重建。判定：基線、上一輪真 agent 與 R1–R8 用的是 `1e066d84…`（有佐證）；**H 輪所用的 daemon binary 未被記錄，標 uncertain**；「09-08 重建的第二顆」本身也只有敘述、沒有歸檔佐證 |
| 原生 App | `apps/interaction-desktop` release bundle，binary sha256 `1103cda7f8491ece2adbaab478056d14a174da47ab8536a2d652c734a0765f19`（2026-09-06 13:53 建置，`CFBundleShortVersionString=0.7.0`，provenance `appSourceRef=3b375bde` **不是** HEAD 的祖先）→ **所有 native-desktop 結果都綁在這顆舊 App，不代表 HEAD build** |
| AX helper | sha256 `3e1bd72d19bfebc876a8c69b47e72aa5634218fc371111dfe73e50d19e49817b`（走查當時的 `scripts/lib/tauri-ax.applescript`） |
| fixture（模擬 iPhone） | `target/debug/examples/fake_iphone` sha256 `64ed2d995093064ed5db070705e7830448a11ae991f8d1823d6799429709127e`（HEAD build，R6／R7 使用）；H 輪 F-06 補跑用的是**修過但未提交**的版本 sha256 `188c7dfd0e9ad7b96423625e6d333c6031b6dd30b5ab2feaaf2d4b419f72c5d0` |
| agent fixture | `crates/interaction-runtime/tests/fixtures/fake_claude.sh`／`fake_codex.sh`（只在基線與 Playwright 用，不在本文件的真 agent 案例內） |
| 真 agent | Claude Code CLI 2.1.263（claude.ai，max 訂閱）、Codex CLI 0.153.4（ChatGPT 登入） |
| 主機 | macOS 26.2（25C56）arm64、Apple M2 Pro；rustc 1.94.0、node v24.5.0、pnpm 10.27.0、Xcode 26.6（17F113） |
| 隔離 | 每個 daemon 一個 `INTERACT_AI_HOME`（在各自 out 目錄下）＋`INTERACT_AI_MOBILE_ADVERTISE=0`；真 agent 時清掉 `INTERACT_AI_CLAUDE_BIN`／`INTERACT_AI_CODEX_BIN`。所有回合的隔離檢查一致通過（`~/.adaptive-interaction` 的 9 個檔案 mtime／size 與執行前逐項相同；全程未寫入 `~/.claude`、`~/.codex`，只唯讀 provider log） |
| 時間範圍 | 執行：2026-09-07（先前一輪真 agent、R1–R8，以及當日傍晚的 H 輪補跑）；判讀、歸檔與本文件撰寫：2026-09-08。時區 Asia/Taipei（證據檔內的時間戳是 UTC，`Z` 結尾） |
| 環境快照 | [`evidence/2026-09-07-phase-0/analysis/environment-2026-09-07T11-15.json`](evidence/2026-09-07-phase-0/analysis/environment-2026-09-07T11-15.json)；身分與 grep 的逐條命令／stdout 在 [`evidence/2026-09-07-phase-0/analysis/commands.json`](evidence/2026-09-07-phase-0/analysis/commands.json) |
| 使用的 harness | [`evidence/2026-09-07-phase-0/harness-used/`](evidence/2026-09-07-phase-0/harness-used/)（`evidence/2026-09-07-phase-0/harness-used/agent_smoke.py`／`evidence/2026-09-07-phase-0/harness-used/multi_session.py`／`evidence/2026-09-07-phase-0/harness-used/restart_test.py` 當時的副本；與現在提交的 [`scripts/tests/phase0/`](../../scripts/tests/phase0/README.md) 只差一行 ROOT 解析） |

**歸檔路徑對照**（草稿與 JSON 內的寫法 → 本文件使用的實際路徑）：`runs/Rn/…` → `e2e-runs/Rn/…`；
`e2e/<name>/…` → `real-agent-e2e-prior/<name>/…`；`matrix/…` → `static-matrix/…`；
副檔名 `.log` → `.txt`、`.stdout`／`.stderr` → `.stdout.txt`／`.stderr.txt`。
**未歸檔**：任何 `home/` 目錄（token、`*.db`、api-token）、`~/.claude/projects/…`、`~/.codex/sessions/…`、`~/.codex/config.toml`、
`~/Library/Preferences/dev.adaptive.interaction.desktop.plist`——這些是 provider／系統端本機紀錄，repo 內查不到。

### 1.1 H 輪補跑（執行於 2026-09-07 傍晚，台北 18:20–18:25／UTC 10:20–10:25；2026-09-08 歸檔）與它改變了什麼

H 輪三個目錄名稱含 `attempt1-interrupted`，是因為當時的 workflow session 被中斷；**目錄內的執行本身是否跑完要看檔案**：

| 目錄 | 執行本身 | 判定 |
|---|---|---|
| [`e2e-runs/H-evidence-attempt1-interrupted/`](evidence/2026-09-07-phase-0/e2e-runs/H-evidence-attempt1-interrupted/) | **跑完**：`evidence/2026-09-07-phase-0/e2e-runs/H-evidence-attempt1-interrupted/term-claude.stdout.txt`／`evidence/2026-09-07-phase-0/e2e-runs/H-evidence-attempt1-interrupted/term-codex.stdout.txt` 都有 `RESULT {…}` 與 `EXIT=0` | 補上 E0-04 的 **SIGTERM 列 → `completed`**（草稿原為 `harness-failed`） |
| [`e2e-runs/H-fixture-attempt1-interrupted/`](evidence/2026-09-07-phase-0/e2e-runs/H-fixture-attempt1-interrupted/) | **跑完**：`evidence/2026-09-07-phase-0/e2e-runs/H-fixture-attempt1-interrupted/logs/run.txt` 四階段 PASS 到 `=== 完成 ===` | E0-08 的 F-06 **裝置側 → `completed`（fixture 等級，且用的是未提交的修復版 fixture）** |
| [`e2e-runs/H-native-attempt1-interrupted/`](evidence/2026-09-07-phase-0/e2e-runs/H-native-attempt1-interrupted/) | **沒跑完**：只有一個 AX dump 定位探針 `probe/p1`（slowdump 10.166 s exit 0；batchdump exit 1、`evidence/2026-09-07-phase-0/e2e-runs/H-native-attempt1-interrupted/probe/p1/dump-fast.txt` 空），該輪隨即中斷 | E0-03／E0-08／E0-11 的 native-desktop **維持 `harness-failed`**；D18／D19 雖已改腳本但**無重跑證據** |

## 2. 總表

### 2.1 裁決

`n/a` = 這一案在該平台上沒有對應行為（不是「沒跑」）。

| 案 | 情境 | Claude Code | Codex | fixture | 原生桌面 | 真 iPhone | overall |
|---|---|---|---|---|---|---|---|
| E0-01 | 首次設定＋UI 偏好，重啟後保留；native 另加 10 種注入故障後的預設恢復 | n/a | n/a | n/a | completed（10/10 故障案例收斂，合計 113.663 s；App 為 2026-09-06 的 0.7.0 bundle） | n/a | **completed**（API／CLI 三層齊全） |
| E0-02 | 派工給真 agent 並比對 ground truth；同 session 多輪與 `resumeProviderSessionId` 原生續接（K-04） | completed | completed（有核可）／correctly-blocked（無核可） | n/a（native work driver 從未真的啟動 fixture agent） | harness-failed（composer 打字被截斷，未曾送出工作） | n/a | **completed** |
| E0-03 | 取消／關閉／緊急停止：分類誠實度、子程序死亡、重啟不自動恢復 | **product-failed**（interrupt→`failed`，5/5）；close→`closed` completed；estop completed | completed（interrupt→`cancelled` 3/3；close→`closed`） | n/a | harness-failed（兩次都卡在 AX composer 輸入） | n/a | **product-failed** |
| E0-04 | daemon 被 SIGKILL／SIGTERM 後重啟：狀態誠實度、mailbox／lease 邊界、孤兒回收 | completed（KILL＋TERM 各一次） | completed（KILL＋TERM 各一次） | n/a | n/a | n/a | **completed**（SIGTERM 路徑已於 H 輪補齊） |
| E0-05 | 記憶→Context Bundle→真 agent 讀取→矛盾記憶→刪除→舊 transcript 是否仍記得（E-09）→重啟保留→受限 token 降權 | completed（含 E-09 成立：resume 後覆誦已刪記憶） | **not-run**（codex 的 E-09 等價行為完全未測） | n/a | n/a | n/a | **completed**（API／CLI 與 Claude 路徑） |
| E0-06 | 同一 daemon 下兩個同型 session 併發：隔離、取消獨立性、不靜默遺失；併發上限 | completed（claude×2） | completed（codex×2） | n/a | n/a | n/a | **completed**；併發上限（`max_sessions=8`／`max_parallel=4`）**needs-environment** |
| E0-07 | claude-code × codex 跨型並行，取消其中的 claude session | 取消已執行（但終態 `failed`，見 D1） | completed（完全不受影響） | n/a | n/a | n/a | **completed** |
| E0-08 | iPhone：配對／能力協商、雙向狀態、停止結果未知的誠實度、AIP、斷線重連、撤銷、核心離線（F-06） | n/a | n/a | completed（六個 journey）；F-06 daemon 側 completed，裝置側於 H 輪補跑後 **completed（修復版 fixture）** | harness-failed（4/10 與 6/10 journey 後 AX dump 超過 55 s；四個 sensor journey 從未跑到） | **needs-environment**（裝置在、Developer Mode on，但 Xcode 無 Team，`device-build.sh` 卡 3/5） | **completed（fixture 範圍）／needs-environment（真機）** |
| E0-09 | 非同步問答：agent 需要人類補資訊；`waiting-for-input` 是否被自動偵測 | completed（多輪續談） | completed（多輪續談） | n/a | n/a（UI 投影只讀 `workState.ts` 常數表，未跑 Tauri／Playwright） | n/a | **not-implemented**（自動偵測不存在；人工 `POST /report` 路徑 completed） |
| E0-10 | 權限：唯讀擋寫、人類拒絕核可、授權寫入＋verify、撤銷、TTL 過期、AI 不可授權、續開不得放寬 | correctly-blocked（唯讀）／completed（授權寫入＋verify） | completed（阻擋有效、accept 落地）＋**product-failed**（deny 送 `"reject"`；`writable_roots` 併入使用者全域設定） | n/a | n/a | n/a | **correctly-blocked**（撤銷／過期／續開不放寬／agent token 全部確定性阻擋；deny 語意有 product-failed 缺陷） |
| E0-11 | 備份匯出範圍聲明、重啟保留、全新 home 還原、壞資料被擋；native 真下載＋真 NSOpenPanel | n/a | n/a | n/a | harness-failed（匯出真的驗到：404 bytes、sha256 `f4a3171…`；匯入與四個阻擋案例全部未驗證） | n/a | **completed（API／CLI）＋correctly-blocked（壞資料 400）＋harness-failed（native 匯入）** |

### 2.2 每案主要證據（路徑相對於 `docs/releases/`）

| 案 | 證據 |
|---|---|
| E0-01 | `evidence/2026-09-07-phase-0/e2e-runs/R5/E0-01-step1.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R5/E0-01-step2.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R5/E0-01-step3.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R5/E0-11-restart-check.txt`；`evidence/2026-09-07-phase-0/e2e-runs/R7/preset/result.json`、`evidence/2026-09-07-phase-0/e2e-runs/R7/preset.cmd.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R7/preset-homes.txt` |
| E0-02 | `evidence/2026-09-07-phase-0/real-agent-e2e-prior/claude-smoke-1/result.json`＋`evidence/2026-09-07-phase-0/real-agent-e2e-prior/claude-smoke-1/sse.jsonl`；`evidence/2026-09-07-phase-0/real-agent-e2e-prior/codex-smoke-1/result.json`、`evidence/2026-09-07-phase-0/real-agent-e2e-prior/codex-smoke-2/result.json`；`evidence/2026-09-07-phase-0/e2e-runs/R8/claude/run.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R8/claude/raw.json`、`evidence/2026-09-07-phase-0/e2e-runs/R8/claude/sse.jsonl`；`evidence/2026-09-07-phase-0/e2e-runs/R8/argv-claude/run.txt`；`evidence/2026-09-07-phase-0/e2e-runs/R8/codex/run.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R8/codex/raw.json` |
| E0-03 | `evidence/2026-09-07-phase-0/e2e-runs/R1/claude-interrupt/result.json`＋`evidence/2026-09-07-phase-0/e2e-runs/R1/claude-interrupt/sse.jsonl`；`evidence/2026-09-07-phase-0/e2e-runs/R1/codex-interrupt/result.json`＋`evidence/2026-09-07-phase-0/e2e-runs/R1/codex-interrupt/sse.jsonl`；`evidence/2026-09-07-phase-0/e2e-runs/R1/b07-claude/result.json`、`evidence/2026-09-07-phase-0/e2e-runs/R1/b07-codex/result.json`；`evidence/2026-09-07-phase-0/e2e-runs/R1/a07-claude/result.json`；`evidence/2026-09-07-phase-0/e2e-runs/R7/work/failed-ax.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R7/work-diag1/failed-ax.txt` |
| E0-04 | `evidence/2026-09-07-phase-0/real-agent-e2e-prior/restart-claude-kill/result.json`＋`evidence/2026-09-07-phase-0/real-agent-e2e-prior/restart-claude-kill/daemon.txt`、`evidence/2026-09-07-phase-0/real-agent-e2e-prior/restart-codex-kill/result.json`；`evidence/2026-09-07-phase-0/e2e-runs/H-evidence-attempt1-interrupted/term-claude.stdout.txt`、`evidence/2026-09-07-phase-0/e2e-runs/H-evidence-attempt1-interrupted/term-claude/result.json`、`evidence/2026-09-07-phase-0/e2e-runs/H-evidence-attempt1-interrupted/term-codex.stdout.txt`、`evidence/2026-09-07-phase-0/e2e-runs/H-evidence-attempt1-interrupted/term-codex/result.json` |
| E0-05 | `evidence/2026-09-07-phase-0/e2e-runs/R2/step2-bundle-http.json`、`evidence/2026-09-07-phase-0/e2e-runs/R2/step2-bundle-cli.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R2/step4-bundle-after-mem2.json`、`evidence/2026-09-07-phase-0/e2e-runs/R2/step5-bundle-after-delete.json`、`evidence/2026-09-07-phase-0/e2e-runs/R2/step5-resume-final.json`、`evidence/2026-09-07-phase-0/e2e-runs/R2/step6-memory-after-restart.json`、`evidence/2026-09-07-phase-0/e2e-runs/R2/step7-agenttoken-get.json`；`evidence/2026-09-07-phase-0/e2e-runs/R2/E0-05-delivery-claude/result.json` |
| E0-06 | `evidence/2026-09-07-phase-0/real-agent-e2e-prior/multi-C-claude-x2/result.json`＋`evidence/2026-09-07-phase-0/real-agent-e2e-prior/multi-C-claude-x2/sse.jsonl`＋`evidence/2026-09-07-phase-0/real-agent-e2e-prior/multi-C-claude-x2/work-0/NOTES.md`＋`evidence/2026-09-07-phase-0/real-agent-e2e-prior/multi-C-claude-x2/work-1/NOTES.md`；`evidence/2026-09-07-phase-0/real-agent-e2e-prior/multi-D-codex-x2/result.json`＋`evidence/2026-09-07-phase-0/real-agent-e2e-prior/multi-D-codex-x2/sse.jsonl` |
| E0-07 | `evidence/2026-09-07-phase-0/real-agent-e2e-prior/multi-E-claude-codex/result.json`＋`evidence/2026-09-07-phase-0/real-agent-e2e-prior/multi-E-claude-codex/sse.jsonl`＋`evidence/2026-09-07-phase-0/real-agent-e2e-prior/multi-E-claude-codex/work-0/NOTES.md`＋`evidence/2026-09-07-phase-0/real-agent-e2e-prior/multi-E-claude-codex/work-1/NOTES.md` |
| E0-08 | `evidence/2026-09-07-phase-0/e2e-runs/R6/logs/pair-2.json`、`evidence/2026-09-07-phase-0/e2e-runs/R6/logs/phone.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R6/logs/phone2.stderr.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R6/logs/phone3.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R6/logs/daemon-responses.jsonl`、`evidence/2026-09-07-phase-0/e2e-runs/R6/logs/probe-while-dead.json`、`evidence/2026-09-07-phase-0/e2e-runs/R6/logs/probe-after-restart.json`；`evidence/2026-09-07-phase-0/e2e-runs/R7/mobile/result.json`、`evidence/2026-09-07-phase-0/e2e-runs/R7/mobile-diag1/result.json`；`evidence/2026-09-07-phase-0/e2e-runs/H-fixture-attempt1-interrupted/logs/run.txt`＋`evidence/2026-09-07-phase-0/e2e-runs/H-fixture-attempt1-interrupted/logs/phone.txt` |
| E0-09 | `evidence/2026-09-07-phase-0/e2e-runs/R3/claude-followup/result.json`、`evidence/2026-09-07-phase-0/e2e-runs/R3/codex-followup/result.json`、`evidence/2026-09-07-phase-0/e2e-runs/R3/manual-report/after-report.json`、`evidence/2026-09-07-phase-0/e2e-runs/R3/claude-followup-attempt1-race-bug/result.json` |
| E0-10 | `evidence/2026-09-07-phase-0/e2e-runs/R4/a-codex-deny/result.json`、`evidence/2026-09-07-phase-0/e2e-runs/R4/b-claude/result.json`、`evidence/2026-09-07-phase-0/e2e-runs/R4/b-codex/result.json`、`evidence/2026-09-07-phase-0/e2e-runs/R4/c-claude-write/result.json`＋`evidence/2026-09-07-phase-0/e2e-runs/R4/c-claude-write/workdir/hello.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R4/c-codex-write/workdir/hello.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R4/d-revoke/steps.json`、`evidence/2026-09-07-phase-0/e2e-runs/R4/d-resume/steps.json`、`evidence/2026-09-07-phase-0/e2e-runs/R4/e-ttl/ttl-steps.json`、`evidence/2026-09-07-phase-0/e2e-runs/R4/f-agent-token/steps.json`；`evidence/2026-09-07-phase-0/e2e-runs/R8/guard-claude/raw.json`、`evidence/2026-09-07-phase-0/e2e-runs/R8/guard-codex/raw.json`、`evidence/2026-09-07-phase-0/e2e-runs/R8/psleak/raw.json` |
| E0-11 | `evidence/2026-09-07-phase-0/e2e-runs/R5/E0-11-export-api.json`、`evidence/2026-09-07-phase-0/e2e-runs/R5/E0-11-export-cli.json`、`evidence/2026-09-07-phase-0/e2e-runs/R5/E0-11-export-api-vs-cli-diff.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R5/E0-11-restore-fresh-field-diff.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R5/E0-11-bad-backup.json`、`evidence/2026-09-07-phase-0/e2e-runs/R5/E0-11-restore-bad-item.txt`；`evidence/2026-09-07-phase-0/e2e-runs/R7/settings/result.json`＋`evidence/2026-09-07-phase-0/e2e-runs/R7/settings/failed-ax.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R7/probe5/main-window-final.txt` |

## 3. 懷疑者改判（4 筆）

判讀流程是「實跑 agent → 獨立懷疑者用原始 artifact 反駁」。以下 4 筆是懷疑者推翻初判的結果，**本文件以改判後的值為準**：

1. **E0-06 併發上限**：`not-implemented` → **`needs-environment`**。理由：`DelegationLimits`（`crates/interaction-core/src/agent.rs:124/133/146`）
   與 `crates/interaction-runtime/src/agents.rs:490/496` 的強制檢查在原始碼裡確實存在，`not-implemented` 與它自己引用的證據矛盾；
   真正的狀況是本輪最多只跑 2 個並行 session。
2. **E0-04 SIGTERM 路徑**：`needs-environment` → **`harness-failed`**。理由：`restart_test.py --signal TERM` 在同一台機器、
   同一顆 binary 上可用，沒有任何環境缺口，只是當輪沒跑。（**其後 H 輪補跑，現已是 `completed`**，見 §4 E0-04。）
3. **E0-05 supersede**：結論維持 `completed`，但缺陷敘述**收窄**：`crates/interaction-core/src/knowledge.rs` 與
   `crates/interaction-runtime/src/knowledge.rs:1182-1225` 內有完整的 candidate→active→stale→disputed→superseded 狀態機，
   只是**沒有接到 user-memory／preference 這一層**。不能寫成「系統沒有任何衝突解析機制」。
4. **E0-05 已終止 session 仍可收新任務**：medium product defect → **by-design 的觀測性缺口（low，D14）**：
   `crates/interaction-core/src/agent.rs:48-55` 與回歸測試 `claimed_completed_is_open_not_terminal` 明文寫「agent 的聲稱讓 session
   保持 open，驗證與關閉是人類／runtime 的獨立步驟」。
   懷疑者同時主張刪除原文那組 `spentMessages=4`／`spentCost=0.31141175`（理由是「沒有任何被擷取的回應含這個值」），
   **此項改判經覆核後不成立、數字已改回**：`evidence/2026-09-07-phase-0/e2e-runs/R2/step6-memory-before-restart.json` 內有一筆
   `createdBy.kind="runtime"` 的 `task-memory` fact（`memoryId=mem-c26b47b7-a53e-4f6c-874f-5b44457d0f07`，
   `provenance=["agent-session:asession-922c9167-ab5f-4fde-8eaf-f8ef75ad516f"]`，即 E0-05-delivery-claude 那個 session），
   content 為 `{"agentId":"claude-code","closedAt":"2026-09-07T09:09:17.471996Z","costUsd":0.31141175000000004,"goal":"phase0-E0-05-delivery","messages":4,"outcome":"ClaimedCompleted",…}`
   ——這是 runtime 自己產生並落地的核心狀態紀錄，不是「沒有任何擷取」。
   `evidence/2026-09-07-phase-0/e2e-runs/R2/step5-old-session-final.json` 的 `spentMessages=3`／`spentCost=0.135597` 只是
   task-sent（`deliveredAt=2026-09-07T09:06:37.650766Z`）後、agent 回覆抵達前的中繼快照（只有「傳出」那一則把 messages 由 2 推到 3，成本尚未入帳；
   其他案例顯示 claude 回覆約需 8–10 秒），不是最終值。以 `step6` 的 runtime fact 為準：`messages=4`／`costUsd=0.31141175`。
   （注意：此處依據是該 runtime fact，**不是** `result-R2.json` 的敘述文字。）

## 4. 逐案

### E0-01 首次設定與偏好持久化

- **前置條件**：全新隔離 home（API 路徑 port 19050；native 路徑每案 `mktemp` 一個 home 並自選 ephemeral port），onboarding 未完成、UI 偏好為預設。
- **使用者操作**：（API）GET onboarding／status → `POST /v1/onboarding/preview` → `/v1/onboarding/commit` → `PATCH /v1/ui/preferences` →
  SIGTERM daemon → 同 home 重啟 → 重讀。（native）在 App 內完成首次設定 → 切換「安靜」陪伴預設 → 在 loopback proxy 注入 10 種故障
  （連線被拒／回應遺失／回讀失敗／寫入前崩潰／寫入後崩潰／清理失敗／更新的選擇／無關偏好／未知格式標記／重連）→ SIGKILL App → 重啟。
- **預期畫面**：重啟後不再回到首次設定精靈；偏好頁顯示 `locale=ja-JP`／`mode=advanced`；native 的「安靜」勾選在 9/10 故障案例後仍為勾選，
  `newer-choice` 以使用者較新的選擇為準，`future-marker` 顯示「恢復標記」而不是靜默丟棄。
- **核心狀態**：`onboarding.completed=true` 且 `completedAt` 有值；`ui_prefs` meta 落地；native 的
  `state/desktop.json.companionPendingPresetOp` 在恢復後清空（`future-marker` 例外，保留 `{"format":99,"unknownIntent":"keep-me"}`）。
- **可觀察效果**：`GET /v1/status.onboardingCompleted=true`；重啟前後 export 逐欄位相同；native 恢復不會多送一次條件寫入
  （以 proxy 的 `conditionalWrites` 計數斷言）。
- **失敗與恢復行為**：SIGTERM 不走 graceful shutdown（`crates/interaction-cli/src/commands.rs:1608` 只等 `ctrl_c()`，即 D15），
  重啟時印出 `reclaiming stale instance lock pid=…` WARN；本輪未觀察到任何資料遺失。

| 分欄 | 結果 |
|---|---|
| Claude Code / Codex | n/a（本案不涉及 agent session） |
| API／CLI | **completed**（三層齊全） |
| fixture | n/a |
| native-desktop | **completed**：10/10 故障案例收斂，合計 113.663 s；但 App 是 2026-09-06 的舊 bundle，此結果不代表 HEAD build |
| real-iphone | n/a |

**證據**：`evidence/2026-09-07-phase-0/e2e-runs/R5/E0-01-step1.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R5/E0-01-step2.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R5/E0-01-step3.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R5/E0-11-restart-check.txt`；
`evidence/2026-09-07-phase-0/e2e-runs/R7/preset/result.json`（`$.results[*].status` 全 `completed`、逐案 `seconds` 相加 113.663）、
`evidence/2026-09-07-phase-0/e2e-runs/R7/preset.cmd.txt`（EXIT=0）、
`evidence/2026-09-07-phase-0/e2e-runs/R7/preset/refused.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R7/preset/future-marker.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R7/preset/newer-choice.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R7/preset/cleanup-failed.txt`、
`evidence/2026-09-07-phase-0/e2e-runs/R7/preset/crash-before-runtime.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R7/preset/crash-after-runtime.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R7/preset/lost-reply.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R7/preset/readback-failed.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R7/preset/reconnect.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R7/preset/unrelated-prefs.txt`。
重啟時的 stale-lock WARN：R5 的 `homeA/daemon-run2.log` 依歸檔規則**未歸檔**（名稱含 `home` 的目錄一律不入 repo）；
歸檔內的等價證據是 `evidence/2026-09-07-phase-0/e2e-runs/H-evidence-attempt1-interrupted/term-claude/daemon.txt`
（同一行 `reclaiming stale instance lock pid=86307`）。

**清理**：R5 兩個 daemon 已 SIGTERM、19050／19051 已釋放（`evidence/2026-09-07-phase-0/e2e-runs/R5/homeA.pid`、`evidence/2026-09-07-phase-0/e2e-runs/R5/homeB.pid` 為當時 pid 紀錄）；
R7 driver 自行刪除 10 個隔離 home（清單 `evidence/2026-09-07-phase-0/e2e-runs/R7/preset-homes.txt`）。

---

### E0-02 派工給真 agent（含同 session 多輪與原生續接）

- **前置條件**：隔離 daemon＋隔離 workdir（內含唯一暗號或 `NOTES.md`），`allowWrite=false`、`toolScope=[]`、`consentScope=[]`。
- **使用者操作**：`POST /v1/agent-sessions` → `POST …/messages {kind:"task"}` → SSE／輪詢到終態 → 讀 `messages?direction=from-session`
  → 對照 ground truth；延伸：同一 session 再送第二個任務（K-04）、關閉後用 `resumeProviderSessionId` 續開。
- **預期**：SSE 走 `fetched → working → claimed-completed`；record 的 `providerSessionId` 有值、`budget.spentMessages` 增加；
  回答內容與 workdir 內的真實資料一致；`humanVerified` 在未 verify 前不存在。
- **核心狀態／效果**：provider 端本機 log（`~/.claude/projects/<slug>/<psid>.jsonl`、`~/.codex/sessions/…/rollout-*.jsonl`，
  **未歸檔**）必須出現對應的 turn；續接必須續寫**同一個檔案**。
- **失敗與恢復行為**：codex 在 `approval_policy=untrusted`＋`sandbox=read-only` 下，連 `cat NOTES.md` 都會停在 `waiting-for-consent`；
  沒有人核可就永遠不會執行（`spentCost=0.0`）——這是**正確阻擋**，不是失敗。

| 分欄 | 結果 |
|---|---|
| Claude Code | **completed**：答案「階段零煙霧測試」與 `NOTES.md` 第一行逐字相符，`claimed-completed`，`spentCost=0.14471075`；多輪（K-04）第二輪答對隨機暗號 `X55XY4F8`，**子程序 pid 33355 跨兩輪不變**（不重新 spawn）；續接走真 `--resume <psid>`（argv 直接證據），provider log 續寫同一檔，且續接輪 prompt 內**不含**暗號（排除洩題） |
| Codex | **completed（有核可時）**／**correctly-blocked（無核可時）**：`--approve approve` 走完 `waiting-for-consent → claimed-completed`，答案逐字正確；不核可則停在 `waiting-for-consent`、`spentCost=0.0`。多輪／續接同樣成立：pid 35908 跨兩輪不變、thread/resume 續寫同一個 rollout、`turn_context` 仍是 `read-only`／`untrusted`（授權是**重新上鎖**不是繼承） |
| fixture | n/a（native work driver 從未真的啟動 fixture agent） |
| native-desktop | **harness-failed**：composer 打字被截斷（見 E0-03），從未送出任何工作 |
| real-iphone | n/a |

**未觀察到**：300 s approval TTL 自動拒絕（`crates/interaction-runtime/src/gateway.rs:29 APPROVAL_TTL_SECS=300`、`gateway.rs:1034 gateway_sweep`、
`runtime.rs:2419` 每 tick 呼叫）——`codex-smoke-1` 在 deadline 前約 15 s 就被 harness 停掉（`wallSeconds=303.34`），
**只有靜態證據，沒有執行期證據**。

**證據**：`evidence/2026-09-07-phase-0/real-agent-e2e-prior/claude-smoke-1/result.json`＋`evidence/2026-09-07-phase-0/real-agent-e2e-prior/claude-smoke-1/sse.jsonl`（112 行）；
`evidence/2026-09-07-phase-0/real-agent-e2e-prior/codex-smoke-1/result.json`（`final.state=waiting-for-consent`、`wallSeconds=303.34`）；
`evidence/2026-09-07-phase-0/real-agent-e2e-prior/codex-smoke-2/result.json`（3 則 mailbox：approval-request → approval-resolved 220 ms → result）；
`evidence/2026-09-07-phase-0/e2e-runs/R8/claude/run.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R8/claude/raw.json`、`evidence/2026-09-07-phase-0/e2e-runs/R8/claude/sse.jsonl`；
`evidence/2026-09-07-phase-0/e2e-runs/R8/argv-claude/run.txt`（`--resume e63b51f8-…` 的 argv 那一行）；
`evidence/2026-09-07-phase-0/e2e-runs/R8/codex/run.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R8/codex/raw.json`；
`evidence/2026-09-07-phase-0/e2e-runs/R8/claude/workdir/NOTES.md`、`evidence/2026-09-07-phase-0/e2e-runs/R8/claude/workdir-other/NOTES.md`（ground truth）。
provider 端續寫同一檔的證據（`~/.claude/projects/…/e63b51f8-….jsonl`、
`~/.codex/sessions/2026/09/07/rollout-…-01a07b22-….jsonl`）**未歸檔**。

**清理**：所有 session 已 close，daemon 皆自行 SIGTERM，19080–19086 已釋放，無殘留子程序
（`evidence/2026-09-07-phase-0/e2e-runs/R8/result-R8.json`）。

---

### E0-03 取消／中斷／緊急停止

- **前置條件**：隔離 daemon＋長任務（1500 字繁中短文，明示不碰檔案），等 record 進入 `active` 後再取消
  （R1 做到了 `--cancel-when-active`；先前一輪三次 claude 取消都落在 `created`／`fetched`）。
- **使用者操作**：`POST …/interrupt`（或 `POST …/close`）→ 輪詢終態 → 檢查子程序 → 檢查有無遲到的 from-session 訊息；
  另外 `POST /v1/emergency-stop` 與重啟後的 latch 驗證。
- **預期**：人類取消 ⇒ `cancelled`（badge「已取消」）；子程序整組消失；沒有遲到結果；close 後仍保留終局狀態。
- **核心狀態／效果**：`record.state`、SSE `agent.session.state`、角色 `character.system-text` 三者一致；`ps` 確認子程序死亡。
- **失敗與恢復行為**：estop 後重啟不得自動恢復；AI 不得解除 estop。

| 分欄 | 結果 |
|---|---|
| Claude Code | **product-failed**：`interrupt` 一律變成 `failed`（誠實階梯：取消 ≠ 失敗），**5/5 重現**（claude-cancel-1、multi-C i=1、multi-E i=0、R1/claude-interrupt〔active 時取消〕、R1/b07-claude）。`grep -c TaskCancelled crates/interaction-agent-gateway/src/claude.rs == 0`——Claude 連接器在任何路徑都產不出 `cancelled`。機制本身有效（子程序死亡、無遲到訊息），錯的是分類與投影。`close`（從 active）→ `closed`，**completed** |
| Codex | **completed**：`interrupt` → `cancelled`（2/2 單 session＋多 session 各一次，合計 3/3），provider rollout 獨立佐證 `turn_aborted reason="interrupted"`；`close` → `closed`，**completed** |
| 緊急停止（A-07，Claude） | **completed**：live session → `cancelled`＋`detail="cancelled (was Active)"`，子程序 1 s 內消失，`/v1/status.emergencyStop=true`，新 session 403 `policy_blocked`，受限 agent token 解除 403 `token_scope_forbidden`（但仍讀得到 `emergencyStop=true`），SIGTERM 重啟後仍鎖住、人類清除後恢復正常 |
| fixture | n/a |
| native-desktop | **harness-failed**：兩次執行都在第一步 composer 輸入就失敗（AX 讀回是 `Native fixture codex can` 與 `Native fixture code`，兩次截斷點不同、`osascript` 全部 exit 0），沒有任何 session 被建立 |
| real-iphone | n/a |

**額外發現**：`interrupt` 在**兩個 agent 上都是 session 級取消**，不是「停止這一輪」——之後對同一 session 送任務一律 409
`mailbox closed`（`crates/interaction-runtime/src/agents.rs:900-904`），子程序已死也無法就地續跑 →
**not-implemented（沒有 stop-generating 能力，D8）**。Codex 的 provider 端其實只中止了 turn（`turn_aborted`），是 runtime 選擇連程序一起殺掉。

**證據**：`evidence/2026-09-07-phase-0/e2e-runs/R1/claude-interrupt/result.json`＋`evidence/2026-09-07-phase-0/e2e-runs/R1/claude-interrupt/sse.jsonl`（第 109 行 `state=failed`、第 116 行 `closed`、
第 110 行 character system-text intent=failed）；`evidence/2026-09-07-phase-0/e2e-runs/R1/codex-interrupt/result.json`＋`evidence/2026-09-07-phase-0/e2e-runs/R1/codex-interrupt/sse.jsonl`
（第 103／109 行）；`evidence/2026-09-07-phase-0/e2e-runs/R1/b07-claude/result.json`、`evidence/2026-09-07-phase-0/e2e-runs/R1/b07-codex/result.json`（`task2Status=409`）；
`evidence/2026-09-07-phase-0/e2e-runs/R1/a07-claude/result.json`＋`evidence/2026-09-07-phase-0/e2e-runs/R1/a07-claude/sse-d1.jsonl`＋`evidence/2026-09-07-phase-0/e2e-runs/R1/a07-claude/sse-d2.jsonl`（estop 前後兩個 daemon 生命週期）；
`evidence/2026-09-07-phase-0/e2e-runs/R1/claude-close/result.json`、`evidence/2026-09-07-phase-0/e2e-runs/R1/codex-close/result.json`；
`evidence/2026-09-07-phase-0/e2e-runs/R1/close-children-check.txt`；
`evidence/2026-09-07-phase-0/e2e-runs/R1/diag-failreason/result.json`（`detail=null`、from-session 0 筆 → D3）；
另外三次重現 `evidence/2026-09-07-phase-0/real-agent-e2e-prior/claude-cancel-1/result.json`、
`evidence/2026-09-07-phase-0/real-agent-e2e-prior/multi-C-claude-x2/result.json`、`evidence/2026-09-07-phase-0/real-agent-e2e-prior/multi-E-claude-codex/result.json`；
native `evidence/2026-09-07-phase-0/e2e-runs/R7/work/failed-ax.txt`（第 40 行）、`evidence/2026-09-07-phase-0/e2e-runs/R7/work/result.json`、
`evidence/2026-09-07-phase-0/e2e-runs/R7/work-diag1/failed-ax.txt`（第 40 行）、`evidence/2026-09-07-phase-0/e2e-runs/R7/work-diag1/result.json`。
**未歸檔**：`claude-cancel-1/home/state/interaction.db`（observation 內 `inferences.report.error='error_during_execution'`）與
`~/.claude/projects/…jsonl`（`[Request interrupted by user]`）。

**清理**：R1 spawn 的 daemon（91044／91045／5661／5662／21898／21899／41215／51232）全部確認死亡、埠釋放、無孤兒子程序
（`evidence/2026-09-07-phase-0/e2e-runs/R1/all-agent-children.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R1/ps-baseline.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R1/close-children-check.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R1/result-R1.json`）。

---

### E0-04 核心崩潰與重啟恢復

- **前置條件**：隔離 daemon＋進行中的 agent session；`restart_test.py --signal KILL`（SIGKILL）與 `--signal TERM`（SIGTERM）。
- **使用者操作**：殺掉 daemon → 觀察子程序孤兒化 → 同 home 重啟 → GET session／list／messages、POST send／interrupt／renew。
- **預期／核心狀態**：重啟時所有 `is_open()` 的 session 一律標成 `expired`（`crates/interaction-runtime/src/agents.rs:1479-1520`），
  `detail` 誠實說明是否回收了孤兒 pgid；mailbox 409、gateway handle 404、renew 409。
- **可觀察效果**：孤兒 `claude -p` 子程序在重啟時被 `reap_recorded_gateway_pgids`（`agents.rs:1615-1671`）依記錄的 pgid 清掉。

| 分欄 | 結果 |
|---|---|
| Claude Code（SIGKILL） | **completed**：`state=expired`、`detail="runtime restarted; orphan subprocess group reaped (pgid 15721)"`；SIGKILL 後子程序 ppid 變 1（真的孤兒），重啟後 `children-final=[]`；send 409／interrupt 404／renew 409 |
| Codex（SIGKILL） | **completed**：codex 隨 daemon 死亡（stdin 關閉），`+2 s`／`+5 s` 都查不到子程序；rollout 於 daemon 被殺後 5 ms 記下 `turn_aborted reason=interrupted`；`detail="runtime restarted"`（無 pgid，因為沒有孤兒可歸因）——**detail 文字的差異本身就是誠實訊號** |
| SIGTERM 路徑（H 輪補跑） | **completed**（草稿原判 `harness-failed`）：claude 側 `state=expired`、`detail="runtime restarted; orphan subprocess group reaped (pgid 86500)"`，SIGKILL 情境相同地觀察到 `claude -p` 在 daemon 死後 ppid=1 並存活 5 s，重啟後 `children-final=[]`；codex 側 `state=expired`、`detail="runtime restarted"`、子程序在 +2 s 就已消失；兩者 send 409／interrupt 404／renew 409 完全一致，`RESULT`＋`EXIT=0`。附帶確認 D15：兩顆 daemon 的第二次啟動都印 `reclaiming stale instance lock pid=…`（SIGTERM 沒走 `InstanceLock` 的 `Drop`） |
| fixture / native-desktop / real-iphone | n/a |

**已知殘餘風險（非本輪新發現）**：孤兒回收是 best-effort，只在**下一次重啟**才做；若 daemon 從此不再啟動，孤兒 claude 會不受管理地
跑到自然結束（`agents.rs:1618` 註解＋`CHANGELOG.md:1846/1859` v0.4 已記錄）。但 `docs/releases/v0.8.0-known-limitations.md` 沒有重申這條
——文件延續性缺口。

**未解釋現象**：`restart-claude-kill` 的 session 送出任務後 21 s 仍是 `created`（同批 `claude-smoke-1` 只要約 5 s）；provider log 完全
沒有 assistant 紀錄，daemon log 沒有 debug 輸出。發生時間接近台北中午 Claude 額度重置窗口，是**合理但未證實**的假說，
誠實記為 unknown，不歸咎程式碼（環境阻礙 B5）。

**證據**：`evidence/2026-09-07-phase-0/real-agent-e2e-prior/restart-claude-kill/result.json`＋`evidence/2026-09-07-phase-0/real-agent-e2e-prior/restart-claude-kill/daemon.txt`；
`evidence/2026-09-07-phase-0/real-agent-e2e-prior/restart-codex-kill/result.json`＋`evidence/2026-09-07-phase-0/real-agent-e2e-prior/restart-codex-kill/daemon.txt`；
H 輪 SIGTERM：`evidence/2026-09-07-phase-0/e2e-runs/H-evidence-attempt1-interrupted/term-claude.stdout.txt`（逐事件列＋`RESULT`＋`EXIT=0`）、
`evidence/2026-09-07-phase-0/e2e-runs/H-evidence-attempt1-interrupted/term-claude/result.json`（`after.state=expired`、`after.detail`、`after.providerSessionId=ef214755-…`、`wallSeconds=28.01`）、
`evidence/2026-09-07-phase-0/e2e-runs/H-evidence-attempt1-interrupted/term-claude/daemon.txt`（stale-lock WARN）、`evidence/2026-09-07-phase-0/e2e-runs/H-evidence-attempt1-interrupted/term-claude.start.txt`、`evidence/2026-09-07-phase-0/e2e-runs/H-evidence-attempt1-interrupted/term-claude/workdir/NOTES.md`、
`evidence/2026-09-07-phase-0/e2e-runs/H-evidence-attempt1-interrupted/term-codex.stdout.txt`、`evidence/2026-09-07-phase-0/e2e-runs/H-evidence-attempt1-interrupted/term-codex/result.json`（`after.detail="runtime restarted"`、`wallSeconds=6.37`）、`evidence/2026-09-07-phase-0/e2e-runs/H-evidence-attempt1-interrupted/term-codex/daemon.txt`；
harness：`evidence/2026-09-07-phase-0/harness-used/restart_test.py`。
provider rollout（`~/.codex/sessions/2026/09/07/rollout-…-01a07a04-….jsonl`）**未歸檔**。

**清理**：四次執行的 `children-final` 皆為 `[]`（見上列四份 result.json）；daemon 由 harness 自行終止。
H 輪未另外記錄埠（19090／19091）釋放檢查，本文件不補述未記錄的事。

---

### E0-05 記憶、Context Bundle 與刪除的不可收回

- **前置條件**：隔離 daemon（19020），記憶庫為空。
- **使用者操作**：建記憶（明示 `agentVisibility` vs 不設）→ 產 Context Bundle（HTTP 與 CLI 各一份）→ 真 agent 讀 bundle →
  加一筆矛盾記憶 → 刪除 → 新 session 再問 → 對舊 provider session 續接再問 → daemon 重啟 → 受限 agent token 直寫。
- **預期／核心狀態**：預設可見性（`crates/interaction-core/src/memory.rs:166-180`）讓 UserMemory 未明示 `agentVisibility` 時
  **對所有 agent 不可見**；bundle 排序是確定性的（layer_rank → updated_at desc → id）；刪除只影響「之後產生的 bundle」。

| 分欄 | 結果 |
|---|---|
| Claude Code | **completed**：`to-session` 的 `body.contextBundle.includes` 含目標記憶，from-session summary 逐字含「靛青色」與暗號；刪除後 `includes=[]` 且 agent 誠實說「不知道」；**E-09 成立**——用 `resumeProviderSessionId` 續開後，agent 完整覆誦已刪除的機密暗號（`剛才談到的顏色是靛青色，暗號是 phase0-mem-32b415。`），證明刪除**無法**回收已進入 provider transcript 的內容 |
| Codex | **not-run**：codex 的 E-09 等價行為（resume 後已刪記憶是否殘留）本輪完全未測 |
| API／CLI | **completed**：HTTP 與 CLI 的 bundle 除 `generatedAt` 外逐欄位相同；重啟後 6 筆記憶逐欄位不變；受限 agent token 對 `/v1/memory` 的 GET/POST 一律 403 `token_scope_forbidden`（**連 handler 都進不去**），只有「human token ＋ `asAgent`」才會走到降權邏輯（`fact → inference`） |
| fixture / native-desktop / real-iphone | n/a |

**收窄後的缺陷（D13）**：user-memory／preference 層**沒有**接上 supersede／conflict 機制——兩筆同標題、內容矛盾的偏好會同時進入
每一次的 bundle，新舊判斷完全外包給下游 LLM 的文字推論。但 `crates/interaction-core/src/knowledge.rs` 的知識層**有**完整的 superseded
狀態機（`crates/interaction-runtime/src/knowledge.rs:1182-1225`），所以這是「分層未接通」而不是「系統沒有這個能力」。

**降級為 by-design 的項目（D14）**：已 `claimed-completed` 的 session 仍可收新任務並真的執行（`agent.rs:48-55` 明文設計＋回歸測試），
但 `GET /v1/agent-sessions/{id}` 的 `state` 全程不反映這次執行，只能靠輪詢 mailbox 才看得出來 → 觀測性缺口（low）。
(a) 重送這一路的最終值為 `messages=4`／`costUsd=0.31141175`，依據是 `evidence/2026-09-07-phase-0/e2e-runs/R2/step6-memory-before-restart.json`
的 runtime `task-memory` fact（`provenance=agent-session:asession-922c9167-…`）；`evidence/2026-09-07-phase-0/e2e-runs/R2/step5-old-session-final.json`
的 `spentMessages=3`／`spentCost=0.135597` 只是 task-sent 後、回覆抵達前的中繼快照（見 §3 第 4 筆）。

**證據**：`evidence/2026-09-07-phase-0/e2e-runs/R2/step1-create-mem1.json`、`evidence/2026-09-07-phase-0/e2e-runs/R2/step1-probe-default-visibility.json`、
`evidence/2026-09-07-phase-0/e2e-runs/R2/step2-bundle-http.json`、`evidence/2026-09-07-phase-0/e2e-runs/R2/step2-bundle-cli.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R2/step4-create-mem2.json`、`evidence/2026-09-07-phase-0/e2e-runs/R2/step4-bundle-after-mem2.json`、
`evidence/2026-09-07-phase-0/e2e-runs/R2/step5-delete-mem1.json`、`evidence/2026-09-07-phase-0/e2e-runs/R2/step5-delete-mem2.json`、`evidence/2026-09-07-phase-0/e2e-runs/R2/step5-bundle-after-delete.json`、`evidence/2026-09-07-phase-0/e2e-runs/R2/step5-old-session-resend.json`、
`evidence/2026-09-07-phase-0/e2e-runs/R2/step5-old-session-final.json`、`evidence/2026-09-07-phase-0/e2e-runs/R2/step5-resume-create.json`、`evidence/2026-09-07-phase-0/e2e-runs/R2/step5-resume-task-sent.json`、`evidence/2026-09-07-phase-0/e2e-runs/R2/step5-resume-final.json`、
`evidence/2026-09-07-phase-0/e2e-runs/R2/step6-create-mem3.json`、`evidence/2026-09-07-phase-0/e2e-runs/R2/step6-memory-before-restart.json`、`evidence/2026-09-07-phase-0/e2e-runs/R2/step6-memory-after-restart.json`、
`evidence/2026-09-07-phase-0/e2e-runs/R2/step7-agenttoken-get.json`、`evidence/2026-09-07-phase-0/e2e-runs/R2/step7-agenttoken-post-plain.json`、`evidence/2026-09-07-phase-0/e2e-runs/R2/step7-human-asagent-demote.json`；
真 agent 三次派送 `evidence/2026-09-07-phase-0/e2e-runs/R2/E0-05-delivery-claude/result.json`、
`evidence/2026-09-07-phase-0/e2e-runs/R2/E0-05-supersede/result.json`、`evidence/2026-09-07-phase-0/e2e-runs/R2/E0-05-delete-newtask/result.json`（三個目錄各附 sse.jsonl、stdout.txt、workdir/NOTES.md，例如 `evidence/2026-09-07-phase-0/e2e-runs/R2/E0-05-delivery-claude/sse.jsonl`、`evidence/2026-09-07-phase-0/e2e-runs/R2/E0-05-supersede/stdout.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R2/E0-05-delete-newtask/workdir/NOTES.md`）；
`evidence/2026-09-07-phase-0/e2e-runs/R2/final-audit.json`、`evidence/2026-09-07-phase-0/e2e-runs/R2/result-R2.json`。
E-09 的 provider transcript（`~/.claude/projects/…/1bf60429-….jsonl`）**未歸檔**。

**清理**：R2 全程共用一顆 daemon（pid 見 `evidence/2026-09-07-phase-0/e2e-runs/R2/daemon.pid`、log 見同目錄 `evidence/2026-09-07-phase-0/e2e-runs/R2/daemon.txt`）；
原始 artifact 未記錄逐項終止／埠釋放檢查，本文件不把未記錄的事寫成已檢查。

---

### E0-06 同型多 Session 併發

- **前置條件**：一個 daemon 下兩個同型 session，各自隔離 workdir、各自唯一暗號。
- **使用者操作**：同時派工 → 取消其中一個 → 觀察另一個是否受影響 → 檢查記錄保留與 SSE 標記。
- **預期／效果**：結果不得交叉洩漏；SSE 的 session 範圍事件必須帶正確 `sessionId`；取消一個不得干擾另一個；
  兩筆記錄都要留在 list（不得靜默丟工作）。

| 分欄 | 結果 |
|---|---|
| Claude Code ×2 | **completed**：i=0 summary 只含自己的 `TOKEN-0-a1fe3a`，SSE 134 行中 39 行 session 範圍事件、**0 筆混淆**；i=1 被取消後仍以 `failed`（見 D1）留在 list |
| Codex ×2 | **completed**：核可只作用在 i=0（`approved=['0']`，i=1 為空），SSE 151 行、0 筆混淆；i=1 取消 → `cancelled` |
| 併發／保留上限 | **needs-environment**（原判 `not-implemented`，見 §3 第 1 筆）：`max_sessions=8`／`max_parallel=4` 在 `crates/interaction-core/src/agent.rs:124-146` 與 `crates/interaction-runtime/src/agents.rs:490/496` 確實實作，unit／fixture 層隨基線 `cargo test --workspace` 執行（[進度入口 §3／§5](phase-0-progress.md)），但**真 agent 執行期沒有證據**：本輪最多只跑 2 個並行 session |
| fixture / native-desktop / real-iphone | n/a |

**觀察**：`status.agentSessions=1` 而 `list` 有 2 筆——那是「開啟中」計數器，不是資料遺失訊號。

**證據**：`evidence/2026-09-07-phase-0/real-agent-e2e-prior/multi-C-claude-x2/result.json`＋`evidence/2026-09-07-phase-0/real-agent-e2e-prior/multi-C-claude-x2/sse.jsonl`＋`evidence/2026-09-07-phase-0/real-agent-e2e-prior/multi-C-claude-x2/work-0/NOTES.md`＋`evidence/2026-09-07-phase-0/real-agent-e2e-prior/multi-C-claude-x2/work-1/NOTES.md`；
`evidence/2026-09-07-phase-0/real-agent-e2e-prior/multi-D-codex-x2/result.json`＋`evidence/2026-09-07-phase-0/real-agent-e2e-prior/multi-D-codex-x2/sse.jsonl`＋`evidence/2026-09-07-phase-0/real-agent-e2e-prior/multi-D-codex-x2/work-0/NOTES.md`＋`evidence/2026-09-07-phase-0/real-agent-e2e-prior/multi-D-codex-x2/work-1/NOTES.md`；
harness `evidence/2026-09-07-phase-0/harness-used/multi_session.py`。各回合的 `home/state/interaction.db`**未歸檔**。

**清理**：harness 結束時終止自己起的 daemon（見 `evidence/2026-09-07-phase-0/harness-used/multi_session.py` 的隔離規則）；原始 artifact 未記錄逐項殘留檢查。

---

### E0-07 跨型（Claude × Codex）並行

- **前置條件／操作**：同 E0-06，但一個 claude-code、一個 codex，取消 claude 那一個。
- **實際結果**：**completed**。codex 側完全不受 claude 取消影響，照常走完自己的核可循環並回報 `TOKEN-1-cd1568`；
  146 行 SSE 中 50 行 session 範圍事件、0 筆混淆；`providerSessionId` 格式各自正確（claude UUIDv4／codex `01a07a0x-` thread id）；
  兩筆記錄都保留（`failed`／`claimed-completed`）。
- **這一案同時鎖定了 D1 的性質**：claude interrupt→`failed`、codex interrupt→`cancelled` 在同型×2（claude）、同型×2（codex）、
  跨型×1 共 3/3 重現，是**連接器屬性**，不是配對造成的假象。

**證據**：`evidence/2026-09-07-phase-0/real-agent-e2e-prior/multi-E-claude-codex/result.json`＋`evidence/2026-09-07-phase-0/real-agent-e2e-prior/multi-E-claude-codex/sse.jsonl`＋
`evidence/2026-09-07-phase-0/real-agent-e2e-prior/multi-E-claude-codex/work-0/NOTES.md`＋`evidence/2026-09-07-phase-0/real-agent-e2e-prior/multi-E-claude-codex/work-1/NOTES.md`＋`evidence/2026-09-07-phase-0/real-agent-e2e-prior/multi-E-claude-codex/daemon.txt`。provider 端兩份紀錄（`~/.claude/projects/…/276c9883-….jsonl`、
`~/.codex/sessions/…/rollout-…-01a07a03-….jsonl`）**未歸檔**。

**清理**：同 E0-06（harness 自行終止 daemon；未逐項記錄）。

---

### E0-08 iPhone（配對、雙向、停止未知、AIP、撤銷、核心離線）

- **前置條件**：隔離 daemon（19060）＋`fake_iphone`（**模擬器，不是真機**）；真機路徑另做唯讀環境檢查。
- **使用者操作**：`interact-ai mobile pair` → fixture 完成 TOFU pin＋HMAC 配對 → 回報 micLevel／權限 → 啟用受器 →
  `mobile stop-sensors`（不 ack）→ ack／斷線 → AIP capability／touch → 斷線重連 resume → `mobile revoke` → SIGKILL daemon 後重啟。
- **預期／核心狀態**：感測不靜默；停止未回覆一律 `unknown`；撤銷立即斷線且舊 token 不可再認證；重啟後舊 token 仍可用（狀態持久化）。

| 分欄 | 結果 |
|---|---|
| fixture（模擬 iPhone） | **completed**（六個 journey）：配對＋連線；裝置回報的 micLevel 立刻出現在 `mobile status`，但**要到受器被明確 `PATCH /v1/receptors/iphone.mic-level {enabled:true}` 之後才進 `/v1/status.activeSensors`**（bulk onboarding 對 consent-gated 元件**永遠**拒絕，`crates/interaction-runtime/src/human.rs:729-742`，是 by-design 的預設關閉不變量）；`stop-sensors` 無 ack → `outcome:unknown`＋`activeSensors[0].state=stop-unknown`（不靜默消失），ack 後才清空；裝置在未 ack 時斷線 → `/v1/sensors/unresolved` 立刻出現 `confirmedStopped:false`，人類 dismiss 也明寫「no source stop confirmation」；AIP capability→touch→Behavior Intent→`status:applied`，revision 1→2→3；斷線期間 presence=`reconnecting`（不是 offline），resume 只回補缺的兩個 patch；revoke 1.5 s 內斷線、裝置從清單消失、舊 token 再連被拒 |
| 核心離線（F-06） | **completed（daemon 側）＋completed（fixture 裝置側，H 輪補跑）**：daemon 側原本就用一支自寫的最小 WS client 驗到「daemon 死時連線被拒、重啟後同一 token `auth-ok`」；裝置側在草稿當時是 `harness-failed`——**出貨的 fixture** 在第一次連線被拒時就 `process::exit(2)`（`crates/interaction-runtime/examples/fake_iphone.rs:177` 的 `die()`，由 `:298` 的 `reconnect()` 呼叫，即 D17）。H 輪把 `connect()` 改成回傳 `Result`、`reconnect()` 失敗只發 `reconnect-failed` 事件（**修改在工作樹、未提交**，重建後 fixture sha256 `188c7dfd…`），四階段全部 PASS：(A) 配對＋`connected`；(B) `kill -KILL` daemon → HTTP `curl_exit=7 http=000`、fixture 印 `reconnect-failed`（`Connection refused (os error 61)`）且 **process 85633 仍存活**；(C) 同 home 重啟 daemon（mobile 埠仍 18790）→ 同一 process 用同 token 再 `reconnect` → 第 2 次 `connected`；(D) `DELETE /v1/mobile/devices/<id>` → `auth-fail`（`unknown device or bad token (possibly revoked)`）。**這仍然是 fixture 等級，不是 real-iphone** |
| native-desktop | **harness-failed**：兩次跑分別只完成 4/10 與 6/10 journey（312.414 s／430.024 s），之後 AX `dump` 超過 driver 的 55 s 上限（dump 耗時 12.8→46.5 s 單調上升，D19）；**四個 sensor journey（停止結果未知的誠實性）從未跑到**。H 輪的定位探針沒有結論（slowdump 10.166 s exit 0、batchdump exit 1 且 `evidence/2026-09-07-phase-0/e2e-runs/H-native-attempt1-interrupted/probe/p1/dump-fast.txt` 為空，該輪被中斷），D19 的修復**未驗證**，本欄維持 `harness-failed` |
| Claude Code / Codex | n/a |
| real-iphone | **needs-environment**：裝置有（iPhone 11「Alex」available (paired)、iOS 26.3.1、Developer Mode enabled），但 `apps/interaction-ios/scripts/device-build.sh --check-only` 在 step 3/5 卡住——Xcode 的 `IDEProvisioningTeams` 是空的（keychain 內雖有一張 `Apple Development` 憑證，但那不是 `xcodebuild -allowProvisioningUpdates` 讀的那個 store）。整輪**沒有任何真機 E2E 證據**（環境阻礙 B1） |

**證據**：`evidence/2026-09-07-phase-0/e2e-runs/R6/logs/pair-1.json`、`evidence/2026-09-07-phase-0/e2e-runs/R6/logs/pair-2.json`、`evidence/2026-09-07-phase-0/e2e-runs/R6/logs/pair-3.json`、`evidence/2026-09-07-phase-0/e2e-runs/R6/logs/pair-4-orphan.json`、
`evidence/2026-09-07-phase-0/e2e-runs/R6/logs/phone.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R6/logs/phone2.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R6/logs/phone2.stderr.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R6/logs/phone3.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R6/logs/phone.cmds.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R6/logs/daemon-responses.jsonl`、`evidence/2026-09-07-phase-0/e2e-runs/R6/logs/daemon.txt`、
`evidence/2026-09-07-phase-0/e2e-runs/R6/logs/daemon-restart.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R6/logs/daemon-restart2.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R6/logs/mobile-status-0.json`、`evidence/2026-09-07-phase-0/e2e-runs/R6/logs/mobile-status-1.json`、`evidence/2026-09-07-phase-0/e2e-runs/R6/logs/pre-disconnect-rev.txt`、
`evidence/2026-09-07-phase-0/e2e-runs/R6/logs/probe-while-dead.json`、`evidence/2026-09-07-phase-0/e2e-runs/R6/logs/probe-after-restart.json`、`evidence/2026-09-07-phase-0/e2e-runs/R6/logs/ws_auth_probe.py`；`evidence/2026-09-07-phase-0/e2e-runs/R6/result-R6.json`。
native：`evidence/2026-09-07-phase-0/e2e-runs/R7/mobile/result.json`（4 個 completed step、`seconds=312.414`、`error` 為 `dump` timed out after 55 s）、
`evidence/2026-09-07-phase-0/e2e-runs/R7/mobile/mobile-pair-and-negotiate-ax.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R7/mobile/mobile-bidirectional-semantic-ax.txt`、
`evidence/2026-09-07-phase-0/e2e-runs/R7/mobile/mobile-disconnect-ax.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R7/mobile/mobile-reconnect-ax.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R7/mobile/phone-1-events.json`、`evidence/2026-09-07-phase-0/e2e-runs/R7/mobile/processes.txt`；
`evidence/2026-09-07-phase-0/e2e-runs/R7/mobile-diag1/result.json`（6 個 completed step、`seconds=430.024`）＋同目錄六份 ax dump：`evidence/2026-09-07-phase-0/e2e-runs/R7/mobile-diag1/mobile-pair-and-negotiate-ax.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R7/mobile-diag1/mobile-pair-again-ax.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R7/mobile-diag1/mobile-bidirectional-semantic-ax.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R7/mobile-diag1/mobile-disconnect-ax.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R7/mobile-diag1/mobile-reconnect-ax.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R7/mobile-diag1/mobile-remove-old-token-rejected-ax.txt`（另有 `evidence/2026-09-07-phase-0/e2e-runs/R7/mobile-diag1/phone-1-events.json`、`evidence/2026-09-07-phase-0/e2e-runs/R7/mobile-diag1/phone-2-events.json`、`evidence/2026-09-07-phase-0/e2e-runs/R7/mobile-diag1/processes.txt`）。
H 輪 F-06 重跑：`evidence/2026-09-07-phase-0/e2e-runs/H-fixture-attempt1-interrupted/logs/run.txt`（四階段 PASS 到 `=== 完成 ===`）、
同目錄 `evidence/2026-09-07-phase-0/e2e-runs/H-fixture-attempt1-interrupted/logs/phone.txt`（事件序列含 `reconnect-failed`／第二次 `connected`／`auth-fail`）、`evidence/2026-09-07-phase-0/e2e-runs/H-fixture-attempt1-interrupted/logs/phone.stderr.txt`、`evidence/2026-09-07-phase-0/e2e-runs/H-fixture-attempt1-interrupted/logs/phone.cmds.txt`、
`evidence/2026-09-07-phase-0/e2e-runs/H-fixture-attempt1-interrupted/logs/http-while-dead.code`、`evidence/2026-09-07-phase-0/e2e-runs/H-fixture-attempt1-interrupted/logs/revoke.json`、`evidence/2026-09-07-phase-0/e2e-runs/H-fixture-attempt1-interrupted/logs/mobile-status-6.json`、`evidence/2026-09-07-phase-0/e2e-runs/H-fixture-attempt1-interrupted/logs/audit-mobile.txt`（`mobile.device-revoked`＋
`mobile.auth-failed knownDevice=false`）、`logs/fake_iphone.sha256`、`evidence/2026-09-07-phase-0/e2e-runs/H-fixture-attempt1-interrupted/logs/daemon-1.txt`、`evidence/2026-09-07-phase-0/e2e-runs/H-fixture-attempt1-interrupted/logs/daemon-2.txt`、
`evidence/2026-09-07-phase-0/e2e-runs/H-fixture-attempt1-interrupted/run-f06.sh`（完整腳本）、`evidence/2026-09-07-phase-0/e2e-runs/H-fixture-attempt1-interrupted/diffs/fake_iphone.before.rs`、`evidence/2026-09-07-phase-0/e2e-runs/H-fixture-attempt1-interrupted/diffs/fake_iphone.after.rs`、`diffs/fake_iphone.diff`；
修復後的靜態關卡 `evidence/2026-09-07-phase-0/e2e-runs/H-fixture-attempt1-interrupted/logs/cargo-fmt.txt`（exit 0）、`evidence/2026-09-07-phase-0/e2e-runs/H-fixture-attempt1-interrupted/logs/cargo-clippy.txt`（exit 0）、`evidence/2026-09-07-phase-0/e2e-runs/H-fixture-attempt1-interrupted/logs/cargo-build-example.txt`（exit 0）、
`evidence/2026-09-07-phase-0/e2e-runs/H-fixture-attempt1-interrupted/logs/cargo-test-mobile-loop.txt`（`81 passed; 0 failed`）、`evidence/2026-09-07-phase-0/e2e-runs/H-fixture-attempt1-interrupted/logs/drill-lint.txt`。
H 輪 native 定位探針：`evidence/2026-09-07-phase-0/e2e-runs/H-native-attempt1-interrupted/probe/p1/trace.json`、
`evidence/2026-09-07-phase-0/e2e-runs/H-native-attempt1-interrupted/probe/p1/dump-slow.txt`、`evidence/2026-09-07-phase-0/e2e-runs/H-native-attempt1-interrupted/probe/p1/dump-fast.txt`、`evidence/2026-09-07-phase-0/e2e-runs/H-native-attempt1-interrupted/probe/p1/processes.txt`、`evidence/2026-09-07-phase-0/e2e-runs/H-native-attempt1-interrupted/probe/probe.py`、`evidence/2026-09-07-phase-0/e2e-runs/H-native-attempt1-interrupted/probe/ax_probe.applescript`。
真機門檻的靜態依據：`apps/interaction-ios/scripts/device-build.sh:158-190`。

**清理**：H 輪 F-06 的 PHASE E 收工逐項記錄在 `evidence/2026-09-07-phase-0/e2e-runs/H-fixture-attempt1-interrupted/logs/run.txt`：`fake_iphone: dead`、`daemon2 final: dead`，FIFO 已刪除。
R6／R7 的 daemon 與 fixture 由各自 driver 終止（pid 紀錄在 `evidence/2026-09-07-phase-0/e2e-runs/R6/logs/daemon.pid`、`evidence/2026-09-07-phase-0/e2e-runs/R6/logs/phone.pid`、`evidence/2026-09-07-phase-0/e2e-runs/R6/logs/pump.pid`）。

---

### E0-09 非同步問答（waiting-for-input）

- **前置條件**：隔離 daemon（19030／19031），真 agent 各一。
- **使用者操作**：請 agent 先問澄清問題 → 人類 30 s 後回答 → 同一 session 送第二個任務；另外人工
  `POST …/report {event:"waiting-for-input"}`。

| 分欄 | 結果 |
|---|---|
| 自動偵測 | **not-implemented**：`GatewayEvent::TaskWaitingForInput` 只在 `crates/interaction-agent-gateway/src/lib.rs:99` 定義（註解自己寫明沒有 connector 會產生它），唯一消費點 `crates/interaction-runtime/src/gateway.rs:412-417`；UI 有完整的 `NEEDS_INPUT` 投影（`apps/interaction-desktop/src/statusProjection/workState.ts:91-101`）卻永遠不會被觸發（D11） |
| Claude Code | **completed（多輪續談）**：澄清問句被包成 `claimed-completed`，第二個任務被同一 session 收下並延續脈絡；`claimId` 才是判斷「新一輪」的可靠欄位（只看 `state` 會誤判——首次腳本因此踩到 race bug，已保留診斷資料） |
| Codex | **completed（多輪續談）**：同型行為，且拍到 `staleClaim → active → 新 claimId` 的中間態 |
| 人工回報路徑 | **completed**：`POST /report` 無條件把任何 open session 標成 `waiting-for-input`（不驗證「是否真的有人在等」——這是人工補位設計），送真任務後會被連接器事件正常覆蓋 |
| fixture / native-desktop / real-iphone | n/a（UI 投影只讀 `workState.ts` 常數表，未跑 Tauri／Playwright） |

**靜態發現（D7）**：codex 0.153.4 的 v2 schema 確實有等價通知 `item/tool/requestUserInput`
（`ToolRequestUserInputParams`／`Response`，EXPERIMENTAL），但 `crates/interaction-agent-gateway/src/codex.rs:250-263` 把**所有**
帶 id+method 的 ServerRequest 一律當核可請求，`approval_summary`（`codex.rs:432-445`）只找 command／cmd／path／reason、
**問題文字被丟掉**，`resolve_approval`（`codex.rs:623-660`）一律回 `{result:{decision}}`，與 `ToolRequestUserInputResponse` 要求的
`{answers:{…}}` 不符。本輪兩次真 codex turn 都沒自然觸發到，所以後果是**靜態推論**，不是觀察。

**證據**：`evidence/2026-09-07-phase-0/e2e-runs/R3/claude-followup/result.json`＋`evidence/2026-09-07-phase-0/e2e-runs/R3/claude-followup/sse.jsonl`＋`evidence/2026-09-07-phase-0/e2e-runs/R3/claude-followup/daemon.txt`；
`evidence/2026-09-07-phase-0/e2e-runs/R3/codex-followup/result.json`＋`evidence/2026-09-07-phase-0/e2e-runs/R3/codex-followup/sse.jsonl`＋`evidence/2026-09-07-phase-0/e2e-runs/R3/codex-followup/daemon.txt`；
`evidence/2026-09-07-phase-0/e2e-runs/R3/claude-followup-attempt1-race-bug/result.json`（診斷資料，刻意保留）；
人工路徑 `evidence/2026-09-07-phase-0/e2e-runs/R3/manual-report/create.json`、`evidence/2026-09-07-phase-0/e2e-runs/R3/manual-report/report-resp.json`、`evidence/2026-09-07-phase-0/e2e-runs/R3/manual-report/after-report.json`、
`evidence/2026-09-07-phase-0/e2e-runs/R3/manual-report/send-task.json`、`evidence/2026-09-07-phase-0/e2e-runs/R3/manual-report/from-session.json`、`evidence/2026-09-07-phase-0/e2e-runs/R3/manual-report/final-after-clear.json`、`evidence/2026-09-07-phase-0/e2e-runs/R3/manual-report/audit.json`、`evidence/2026-09-07-phase-0/e2e-runs/R3/manual-report/status.json`；
harness `evidence/2026-09-07-phase-0/e2e-runs/R3/followup_test.py`；彙整 `evidence/2026-09-07-phase-0/e2e-runs/R3/result-R3.json`。
codex v2 schema 的比對來源在靜態盤點 `evidence/2026-09-07-phase-0/static-matrix/dim-K.json`（schema 檔本身不在本目錄）。

**清理**：R3 三支 driver 各自起停自己的 daemon（pid 紀錄 `evidence/2026-09-07-phase-0/e2e-runs/R3/manual-report/daemon.pid`）；
原始 artifact 未記錄逐項殘留檢查。

---

### E0-10 權限：拒絕、唯讀、授權寫入、驗證、撤銷、過期、續開不放寬

- **前置條件**：每個子案一個隔離 daemon（19040–19045）＋隔離 workdir。
- **使用者操作**：唯讀 session 收到寫檔任務／人類 deny 核可請求／授權寫入後 verify／close 撤銷／TTL 1 分鐘到期／
  用受限 agent token 打人類層端點／各種「比上次更寬」的續開。

| 分欄 | 結果 |
|---|---|
| Claude Code（唯讀） | **correctly-blocked**：provider 端 `permissionMode=plan`，整輪只用 Glob／Read，**連 Write 工具都沒被提供**；agent 誠實說「沒有 Write 工具」，workdir 沒有 `hello.txt` |
| Claude Code（授權寫入） | **completed**：`permissionMode` 變 `acceptEdits`，`hello.txt` 內容 `hi\n`（`od -c` 驗過），record `allowWrite=true`／`toolScope=['workspace.write']`／**無 `humanVerified` 欄位**；`POST /verify` 後 `humanVerified={at,claimId,note}` 且綁定當次 `claimId`，重複 verify 409 |
| Codex（deny） | **completed（阻擋有效）＋product-failed（wire 值錯，D4）**：deny 之後指令確實沒執行、agent 誠實回報、狀態沒卡住；但 gateway 送的是 `"reject"`（`crates/interaction-agent-gateway/src/codex.rs:644`），而 codex 0.153.4 的 `CommandExecutionApprovalDecision` 只接受 accept／acceptForSession／acceptWithExecpolicyAmendment／applyNetworkPolicyAmendment／decline／cancel。實測 codex 端把它當成**核可管線錯誤**（`Rejected("approval request failed")`）而 fail-closed——安全結果對，但是靠 provider 的失敗處理，不是靠我們送對值；而且人類的「否決」被原文轉述成像系統故障 |
| Codex（授權寫入） | **completed**：`accept` 是合法值，`hello.txt` 落地（`hi`，2 bytes）；但 provider 自報的有效 sandbox 是 `workspace-write` **＋使用者全域 `~/.codex/config.toml` 的三個 `writable_roots`**，超出 session 授權的 `resolvedWorkdir`（D6） |
| 撤銷／過期 | **completed**：close 後 `state=closed`、`consentScope` 清空、`humanVerified` 保留；messages／renew 409、approve／interrupt 404；TTL 到期（懶惰式，讀取時翻轉）→ `expired`，之後 409／404／409；**重啟後所有 open session 一律 expired，授權不因重啟復活** |
| 續開不得放寬 | **correctly-blocked**：以原始 probe 逐筆計，R4 9 筆（`d-resume/steps.json` 4 筆 403＋`d-resume2/steps.json` 5 筆 403）＋R8 15 筆（`guard-claude/raw.json` 10 個 probe 中 7 筆 403、`guard-codex/raw.json` 10 個中 8 筆 4xx，含 codex 的 `maxCost-added` 400）＝24 筆「比上次更寬」的續開全部 4xx，另 5 筆刻意縮小的 probe（claude 的 `maxCost-added`／`workdir-trailing-slash`／`ttl-narrower`、codex 的後兩者）正確 200，訊息逐項指名（可用工具／使用授權／資料範圍／時間上限／訊息上限／工作目錄）；`../` 繞路被 canonicalize 擋下；**省略欄位＝落到 runtime 預設＝放寬，一樣被擋**；縮小（ttl 5／maxMessages 10／結尾斜線／claude 加 maxCost）正確放行；假造／空字串／全空白的 `resumeProviderSessionId` 一律 403 |
| AI 不可授權 | **correctly-blocked**：受限 agent token 對 approve／create／PATCH policy／estop clear／verify／renew／GET session／onboarding commit 全部 403 `token_scope_forbidden`；只有「安全遞減」的 `POST /v1/emergency-stop` 允許（by design） |
| fixture / native-desktop / real-iphone | n/a |

**額外發現（D5）**：codex 連接器沒有等價於 claude 的 MCP／plugin 封鎖——宣告唯讀的 session 一建立就啟動使用者 `~/.codex` 設定裡的
MCP server（chrome-devtools-mcp 瀏覽器控制、codegraph）與 ChatGPT computer-use helper，而且任務全文出現在 world-readable 的 process argv。

**副作用（誠實記錄，D12）**：R4 的 agent-token estop 那一步把當時 `claimed-completed` 的續開 session 轉成 `cancelled`，導致之後 verify
回 409——**是測試操作造成的**，不是重啟造成的；但它也暴露一個設計問題：AI 按下 estop 就能讓一個等待人類驗證的聲稱永遠無法被驗證。

**證據**：`evidence/2026-09-07-phase-0/e2e-runs/R4/a-codex-deny/result.json`＋`evidence/2026-09-07-phase-0/e2e-runs/R4/a-codex-deny/sse.jsonl`＋`evidence/2026-09-07-phase-0/e2e-runs/R4/a-codex-deny/daemon.txt`（daemon.txt 全檔 4 行 → D16）；
`evidence/2026-09-07-phase-0/e2e-runs/R4/b-claude/result.json`、`evidence/2026-09-07-phase-0/e2e-runs/R4/b-codex/result.json`；`evidence/2026-09-07-phase-0/e2e-runs/R4/c-claude-write/result.json`＋`evidence/2026-09-07-phase-0/e2e-runs/R4/c-claude-write/workdir/hello.txt`＋`evidence/2026-09-07-phase-0/e2e-runs/R4/c-claude-write/restart-steps.json`＋
`evidence/2026-09-07-phase-0/e2e-runs/R4/c-claude-write/verify-after-restart.json`＋`evidence/2026-09-07-phase-0/e2e-runs/R4/c-claude-write/after-restart.txt`；`evidence/2026-09-07-phase-0/e2e-runs/R4/c-codex-write/result.json`＋`evidence/2026-09-07-phase-0/e2e-runs/R4/c-codex-write/workdir/hello.txt`；
`evidence/2026-09-07-phase-0/e2e-runs/R4/d-revoke/steps.json`＋`evidence/2026-09-07-phase-0/e2e-runs/R4/d-revoke/step.py`；`evidence/2026-09-07-phase-0/e2e-runs/R4/d-resume/steps.json`＋`evidence/2026-09-07-phase-0/e2e-runs/R4/d-resume/resume.py`；`evidence/2026-09-07-phase-0/e2e-runs/R4/d-resume2/steps.json`＋`evidence/2026-09-07-phase-0/e2e-runs/R4/d-resume2/resume2.py`；
`evidence/2026-09-07-phase-0/e2e-runs/R4/e-ttl/create.json`＋`evidence/2026-09-07-phase-0/e2e-runs/R4/e-ttl/ttl-steps.json`＋`evidence/2026-09-07-phase-0/e2e-runs/R4/e-ttl/restart-steps.json`＋`evidence/2026-09-07-phase-0/e2e-runs/R4/e-ttl/get-pre.txt`＋`evidence/2026-09-07-phase-0/e2e-runs/R4/e-ttl/msg-pre.json`＋`evidence/2026-09-07-phase-0/e2e-runs/R4/e-ttl/renew-pre.json`；
`evidence/2026-09-07-phase-0/e2e-runs/R4/f-agent-token/steps.json`＋`evidence/2026-09-07-phase-0/e2e-runs/R4/f-agent-token/f.py`；彙整 `evidence/2026-09-07-phase-0/e2e-runs/R4/result-R4.json`。
R8 續開守門：`evidence/2026-09-07-phase-0/e2e-runs/R8/guard-claude/run.txt`＋`evidence/2026-09-07-phase-0/e2e-runs/R8/guard-claude/raw.json`、`evidence/2026-09-07-phase-0/e2e-runs/R8/guard-codex/run.txt`＋`evidence/2026-09-07-phase-0/e2e-runs/R8/guard-codex/raw.json`、
`evidence/2026-09-07-phase-0/e2e-runs/R8/guard_probe.py`；D5 的 argv／ps 證據：`evidence/2026-09-07-phase-0/e2e-runs/R8/argv-codex/run.txt`＋`evidence/2026-09-07-phase-0/e2e-runs/R8/argv-codex/raw.json`、
`evidence/2026-09-07-phase-0/e2e-runs/R8/psleak/raw.json`＋`evidence/2026-09-07-phase-0/e2e-runs/R8/psleak/run.txt`＋`evidence/2026-09-07-phase-0/e2e-runs/R8/psleak_probe.py`，對照組
`evidence/2026-09-07-phase-0/e2e-runs/R8/argv-claude/run.txt`（claude argv 有 `--strict-mcp-config`）。
`~/.codex/config.toml:418-419`（`writable_roots`）與 codex rollout **未歸檔**。

**清理**：R4 六個子案各自起停 daemon（pid 紀錄 `evidence/2026-09-07-phase-0/e2e-runs/R4/e-ttl/pid.txt`、
`evidence/2026-09-07-phase-0/e2e-runs/R4/d-resume2/pid.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R4/c-claude-write/pid-restart.txt`）；原始 artifact 未記錄統一的殘留掃描。

---

### E0-11 備份匯出、還原與重啟保留

- **前置條件**：home A 已完成 onboarding／偏好／4 筆跨 layer 記憶；home B 全新。
- **使用者操作**：`GET /v1/memory/export`（HTTP 與 CLI 各一）→ SIGTERM＋重啟 → 逐欄位比對 → 在全新 home 逐條還原
  （欄位集合與 `apps/interaction-desktop/src/pages/BackupSection.tsx:72-83` 相同）→ 再重啟 → 匯入一份中間有壞資料（缺 title）的備份；
  native：真 WebView 下載 ＋ 真 NSOpenPanel。

| 分欄 | 結果 |
|---|---|
| API／CLI | **completed**：export `count=4`／`total=4`／`limitReached=false`，`included=["memory-items"]`、`notIncluded=["knowledge-nodes","assets-and-derivatives","knowledge-receipts","character-interaction-memory"]`（後端硬編與前端 `SCOPE_LABEL` 語意一致——這是**明示的功能邊界**：換機重灌無法完整復原，只能復原記憶分層）；重啟後 6 筆逐欄位不變；全新 home 還原保留 layer／kind／title／content／tags／confidence／agentVisibility／agentDenylist／retention，重新賦予 memoryId／createdAt／updatedAt，且 `createdBy` 一律變成 human（`restoreBackup()` 不送 `asAgent`）——刻意「不信任備份中的身分」，但代價是 agent 出處遺失 |
| 壞資料 | **correctly-blocked**：第 3 筆（缺 title）400 `validation_failed: title 必須為 1..120 字`，第 4 筆完全沒被送出（for-loop 在第一個錯誤 throw），已寫入的 2 筆保留，總數 4→6 |
| native-desktop | **harness-failed**：匯出這一半**真的驗到了**（`~/Downloads` 出現 404 bytes、sha256 `f4a317120e88363c46cbe458760f1f684d698f65595420a04c528a31c8e3f944` 的檔案並在收尾刪除）；但**匯入與四個 correctly-blocked 案例全部未驗證**——AX 的 `choosefile`（`scripts/lib/tauri-ax.applescript:51-63`）從不驗證是否真的導航，Open panel 停在 2026-09-06 的記憶目錄並開了字母序第一個檔（D18）。probe5 用 `kind:"not-a-companion-settings"` 的檔案（解析器必須拒絕）仍看到「已匯入角色設定並套用。」→ 證明 App 拿到的不是我們指定的檔。**這同時代表 2026-09-06 那次綠燈可能是假陽性**。D18 的修復**未驗證**，本欄維持 `harness-failed` |
| Claude Code / Codex / fixture / real-iphone | n/a |

**證據**：`evidence/2026-09-07-phase-0/e2e-runs/R5/E0-11-create.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R5/E0-11-export-api.json`、`evidence/2026-09-07-phase-0/e2e-runs/R5/E0-11-export-cli.json`、
`evidence/2026-09-07-phase-0/e2e-runs/R5/E0-11-export-cli.err`、`evidence/2026-09-07-phase-0/e2e-runs/R5/E0-11-export-api-vs-cli-diff.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R5/E0-11-export-after-restart.json`、
`evidence/2026-09-07-phase-0/e2e-runs/R5/E0-11-export-B-before-restart.json`、`evidence/2026-09-07-phase-0/e2e-runs/R5/E0-11-export-B-after-restart.json`、`evidence/2026-09-07-phase-0/e2e-runs/R5/restore.py`、`evidence/2026-09-07-phase-0/e2e-runs/R5/E0-11-restore-fresh.txt`、
`evidence/2026-09-07-phase-0/e2e-runs/R5/E0-11-restore-fresh-result.json`、`evidence/2026-09-07-phase-0/e2e-runs/R5/E0-11-restore-fresh-field-diff.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R5/E0-11-bad-backup.json`、
`evidence/2026-09-07-phase-0/e2e-runs/R5/E0-11-restore-bad-item.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R5/E0-11-restore-bad-item-result.json`、`evidence/2026-09-07-phase-0/e2e-runs/R5/result-R5.json`。
native：`evidence/2026-09-07-phase-0/e2e-runs/R7/settings/result.json`（`steps[0]` = `export-file` completed、`bytes=404`、
`sha256=f4a3171…`；`steps[1]` = `unfinished` failed、`error="observable state did not arrive"`）、同目錄 `evidence/2026-09-07-phase-0/e2e-runs/R7/settings/failed-ax.txt`、
`evidence/2026-09-07-phase-0/e2e-runs/R7/settings/export-file-ax.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R7/settings/exported-settings.json`、`evidence/2026-09-07-phase-0/e2e-runs/R7/settings/processes.txt`；重跑 `evidence/2026-09-07-phase-0/e2e-runs/R7/settings-diag1/result.json`、
`evidence/2026-09-07-phase-0/e2e-runs/R7/settings-diag2/result.json`；picker 探針 `evidence/2026-09-07-phase-0/e2e-runs/R7/probe1/prefs-final.json`＋`evidence/2026-09-07-phase-0/e2e-runs/R7/probe1/trace.json`＋
`evidence/2026-09-07-phase-0/e2e-runs/R7/probe1/panel-after-choosefile.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R7/probe4/prefs-final.json`、`evidence/2026-09-07-phase-0/e2e-runs/R7/probe5/main-window-final.txt`（第 225 行「已匯入角色設定並套用。」）＋
`evidence/2026-09-07-phase-0/e2e-runs/R7/probe5/trace.json`＋`evidence/2026-09-07-phase-0/e2e-runs/R7/probe5/panel-final.txt`、`evidence/2026-09-07-phase-0/e2e-runs/R7/probe_picker.py`。
`~/Library/Preferences/dev.adaptive.interaction.desktop.plist`（`NSOSPLastRootDirectory` 指向 2026-09-06 的目錄）**未歸檔**。

**清理**：R5 兩個 daemon 已 SIGTERM、埠釋放（見 E0-01）；native 匯出檔在 driver 收尾時刪除（`evidence/2026-09-07-phase-0/e2e-runs/R7/settings/result.json` 的 export 步驟）。

## 5. 雙平台裁決

> **這一節不宣稱「通過」。** 兩個平台各自有 completed 的部分，也各自有 product-failed 與 not-implemented；
> 整體只能說「哪些使用者流程在哪個平台上三層齊全」，不能說「雙平台驗收通過」。

### 5.1 真 Claude Code（CLI 2.1.263，claude.ai max）

- **completed**（三層齊全）：
  1. 派工→接收→`claimed-completed`，回答與 workdir ground truth 逐字相符（E0-02）。
  2. 同一 session 多輪（K-04），子程序不重新 spawn（pid 33355 跨兩輪不變）（E0-02）。
  3. 原生 `--resume <providerSessionId>` 續接（argv 直接證據），provider log 續寫同一檔（E0-02）。
  4. 授權寫入：`permissionMode=acceptEdits`、`hello.txt` 落地、`POST /verify` 後 `humanVerified` 綁當次 `claimId`（E0-10）。
  5. 撤銷（close）、TTL 過期、重啟後一律 `expired`：授權不因重啟復活（E0-10、E0-04）。
  6. SIGKILL 與 SIGTERM 後重啟的誠實狀態＋孤兒 pgid 回收（E0-04）。
  7. 記憶 → Context Bundle → 被讀 → 刪除後新任務不再含；且 E-09 成立：resume 後 agent 完整覆誦已刪除的暗號（E0-05）。
  8. 多 Session 隔離與單獨取消（E0-06、E0-07）；緊急停止的全部不變量（E0-03）。
  9. 非同步多輪續談（E0-09）。
- **correctly-blocked**：唯讀 session 連 Write 工具都沒被提供（`permissionMode=plan`）；24 筆「比上次更寬」的續開 probe（R4 9 筆＋R8 15 筆）全部 4xx，5 筆縮小的正確放行；
  受限 agent token 對人類層端點全部 403（E0-10）。
- **product-failed**：D1 人類 `interrupt` 被記成 `failed` 而不是 `cancelled`（5/5 重現；連接器完全產不出 `cancelled`）；
  D2 close 的 SSE 投影把終局壓成 `closed`；D3 `failed` 在 record 與 mailbox 上都沒有任何原因。
- **not-implemented**：D8「停止這一輪但保留 session」；D11 `waiting-for-input` 自動偵測；K-01／K-02／K-03 模型與推理設定不可指定、不可回報。
- **harness-failed**：原生桌面派工與取消（合成鍵盤掉字，B2）。

### 5.2 真 Codex（CLI 0.153.4，ChatGPT 登入）

- **completed**（三層齊全）：
  1. 有核可時派工→`waiting-for-consent`→`claimed-completed`，答案逐字正確（E0-02）。
  2. 同一 session 多輪（pid 35908 跨兩輪不變）；thread/resume 續寫同一 rollout，且 `turn_context` 仍是 `read-only`／`untrusted`
     ——授權是**重新上鎖**不是繼承（E0-02）。
  3. `interrupt` → `cancelled`（3/3），provider rollout 以 `turn_aborted reason="interrupted"` 獨立佐證（E0-03、E0-07）。
  4. SIGKILL／SIGTERM 後 codex 隨 daemon 死亡、重啟後誠實 `expired`（E0-04）。
  5. 授權寫入 `accept` 路徑檔案落地（E0-10）；多 Session 隔離、核可只作用在發出請求的 session（E0-06、E0-07）。
  6. 非同步多輪續談，並拍到 `staleClaim → active → 新 claimId` 的中間態（E0-09）。
- **correctly-blocked**：唯讀＋`approval_policy=untrusted` 下連 `cat NOTES.md` 都停在 `waiting-for-consent`，沒有人核可就永遠不執行
  （`spentCost=0.0`）；deny 之後指令確實沒執行；續開不放寬與 agent token 阻擋同 Claude（E0-02、E0-10）。
- **product-failed**：D4 deny 的 wire 值 `"reject"` 不在 0.153.4 的合法列舉內（安全結果靠 provider fail-closed 而非我們送對值，
  且人類的否決被轉述成像系統故障）；D5 連接器沒有 MCP／plugin 封鎖——唯讀 session 仍會啟動使用者的 chrome-devtools-mcp／codegraph／
  ChatGPT computer-use helper，並把任務全文暴露在 process argv。
  [進度入口 §5](phase-0-progress.md) 另把 D6（`writable_roots` 併入使用者全域設定，超出 session 授權）也列入 codex 的 product-failed。
- **not-run**：E-09 的 codex 等價行為（resume 後已刪記憶是否殘留）（E0-05）。
- **not-implemented**：與 Claude 共同的 D8、D11、K-01／K-02／K-03；另有 D7（`item/tool/requestUserInput` 未分派、回應 shape 不符），
  但其後果是**靜態推論**，本輪未自然觸發。

## 6. 多 Session 裁決

多 Session 在 runtime 層是乾淨的：同型 claude×2、同型 codex×2、跨型 claude＋codex 三種配對各跑一次，
結果不交叉洩漏（每個 summary 只含自己 workdir 的暗號）、SSE 的 session 範圍事件在 134／151／146 行中
**0 筆 `sessionId` 混淆**、取消其中一個不影響另一個的時間線、核可只作用在發出請求的那個 session、
被取消的記錄也留在 list 不靜默消失（`status.agentSessions` 只是「開啟中」計數）。
唯一沒驗到的是**併發上限本身**：`max_sessions=8`／`max_parallel=4` 在原始碼確實實作
（`crates/interaction-core/src/agent.rs:124-146`、`crates/interaction-runtime/src/agents.rs:490/496`），
但本輪最多只跑 2 個並行 session，因此改判 `needs-environment` 而非 `not-implemented`（見 §3 第 1 筆）。

## 7. 逐次執行的模型／費用／token／耗時

> **讀法**：`requestedModel` 全部是 `not-specifiable-via-gateway`——gateway 沒有可傳入的模型欄位（K-01／K-02／K-03 = absent）；
> `actualModel` 全部標 `provider-local-log (not via gateway)`，也就是事後從 provider 端本機紀錄唯讀取得，那些紀錄**未歸檔**。
> `unknown` 是誠實值：turn 太短、被中斷、或該案沒有讀 provider log。
> connector 欄只寫版本；來源 JSON 記的呼叫形式是 `claude -p --input-format stream-json --output-format stream-json --safe-mode`
> 與 `codex app-server`（JSON-RPC v2），第 25 列另記了顯式 `--permission-mode plan --tools Read,Glob,Grep`。
> 歸檔的 argv 證據（`evidence/2026-09-07-phase-0/e2e-runs/R8/argv-claude/run.txt`、
> `evidence/2026-09-07-phase-0/e2e-runs/H-evidence-attempt1-interrupted/term-claude.stdout.txt`）顯示實際 argv 還含
> `--verbose --strict-mcp-config`；第 35 列（R8/claude S2）的 argv 也是顯式 `--permission-mode plan --tools Read,Glob,Grep`。費用欄的 `spentCost 0.0` 對 codex 是「連接器從不回報成本」，不是「免費」。
> 第 38／39 列是 H 輪 SIGTERM 補跑：模型欄無法從歸檔證據得知（`evidence/2026-09-07-phase-0/e2e-runs/H-evidence-attempt1-interrupted/term-claude/result.json` 與 `evidence/2026-09-07-phase-0/e2e-runs/H-evidence-attempt1-interrupted/term-codex/result.json` 內都沒有 model 欄、provider log 未歸檔），故記 `unknown`。

| # | run | agent | connector | requestedModel | actualModel | 來源 | reasoning／模式 | cost／token | 秒 |
|---|---|---|---|---|---|---|---|---|---|
| 1 | prior/claude-smoke-1 (E0-02) | claude-code | Claude Code CLI 2.1.263 | not-specifiable-via-gateway | claude-fable-5-1 | provider-local-log (not via gateway) | permissionMode=plan | spentCost 0.14471075 USD, spentMessages 2 | 11.2 |
| 2 | prior/codex-smoke-1 (E0-02, 不核可) | codex | Codex CLI 0.153.4 | not-specifiable-via-gateway | gpt-6-astra | provider-local-log (not via gateway) | reasoning_effort=medium; approval_policy=untrusted; sandbox=read-only | spentCost 0.0（指令從未執行） | 303.34 |
| 3 | prior/codex-smoke-2 (E0-02, 核可) | codex | Codex CLI 0.153.4 | not-specifiable-via-gateway | gpt-6-astra | provider-local-log (not via gateway) | reasoning_effort=medium | spentCost 0.0（codex 連接器從不回報成本）, spentMessages 4 | 17.01 |
| 4 | prior/claude-cancel-1 (E0-03) | claude-code | Claude Code CLI 2.1.263 | not-specifiable-via-gateway | unknown（turn 在第一個 assistant 事件前被中斷，provider log 無 model 欄） | provider-local-log (not via gateway) | permissionMode=plan | unknown（spentCost 0.0；失敗分支丟棄 total_cost_usd） | 15.31 |
| 5 | prior/codex-cancel-1 (E0-03) | codex | Codex CLI 0.153.4 | not-specifiable-via-gateway | gpt-6-astra | provider-local-log (not via gateway) | reasoning_effort=null（summary=auto） | unknown（rollout token_count 只有 rate_limits） | 15.46 |
| 6 | prior/multi-C-claude-x2 i=0 (E0-06) | claude-code | Claude Code CLI 2.1.263 | not-specifiable-via-gateway | claude-fable-5-1 | provider-local-log (not via gateway) | permissionMode=plan | costUsd 0.13904425 | 12.47 |
| 7 | prior/multi-C-claude-x2 i=1 (E0-06, 被中斷) | claude-code | Claude Code CLI 2.1.263 | not-specifiable-via-gateway | unknown（中斷於第一個 assistant 事件前） | provider-local-log (not via gateway) | permissionMode=plan | spentCost 0.0 | 12.47 |
| 8 | prior/multi-D-codex-x2 i=0 (E0-06) | codex | Codex CLI 0.153.4 | not-specifiable-via-gateway | gpt-6-astra | provider-local-log (not via gateway) | reasoning_effort=medium | spentCost 0.0, spentMessages 4 | 16.73 |
| 9 | prior/multi-D-codex-x2 i=1 (E0-06, 被中斷→cancelled) | codex | Codex CLI 0.153.4 | not-specifiable-via-gateway | gpt-6-astra | provider-local-log (not via gateway) | reasoning_effort=medium | spentCost 0.0 | 16.73 |
| 10 | prior/multi-E i=0 claude (E0-07, 被中斷) | claude-code | Claude Code CLI 2.1.263 | not-specifiable-via-gateway | unknown（中斷於 assistant 事件前） | provider-local-log (not via gateway) | permissionMode=plan | spentCost 0.0 | 17.53 |
| 11 | prior/multi-E i=1 codex (E0-07) | codex | Codex CLI 0.153.4 | not-specifiable-via-gateway | gpt-6-astra | provider-local-log (not via gateway) | reasoning_effort=medium | spentCost 0.0, spentMessages 4 | 17.53 |
| 12 | prior/restart-claude-kill (E0-04) | claude-code | Claude Code CLI 2.1.263 | not-specifiable-via-gateway | unknown（provider log 無 assistant 紀錄；21 s 內未進 active） | provider-local-log (not via gateway) | unknown | spentCost 0.0, spentMessages 1 | 28.23 |
| 13 | prior/restart-codex-kill (E0-04) | codex | Codex CLI 0.153.4 | not-specifiable-via-gateway | gpt-6-astra（rollout session_meta.base_instructions.provenance.model） | provider-local-log (not via gateway) | unknown（turn 太短，rollout 未落地 reasoning_effort） | spentCost 0.0, spentMessages 1 | 6.31 |
| 14 | R1/claude-interrupt (E0-03, active 時取消) | claude-code | Claude Code CLI 2.1.263 | not-specifiable-via-gateway | claude-fable-5-1 | provider-local-log (not via gateway) | permissionMode=plan（thinking block 為空） | input 2 / cache_creation 5765 / cache_read 2800 / output 2；無 total_cost_usd；spentCost 0.0 | 73.61 |
| 15 | R1/codex-interrupt (E0-03, active 時取消) | codex | Codex CLI 0.153.4 | not-specifiable-via-gateway | gpt-6-astra | provider-local-log (not via gateway) | reasoning_effort=null；approval_policy=untrusted；sandbox=read-only | unknown（rollout 無 token_count） | 13.93 |
| 16 | R1/claude-close (E0-03) | claude-code | Claude Code CLI 2.1.263 | not-specifiable-via-gateway | unknown（kill 先於 provider log flush） | provider-local-log (not via gateway) | permissionMode=plan | unknown（無 usage 紀錄） | 70.57 |
| 17 | R1/codex-close (E0-03) | codex | Codex CLI 0.153.4 | not-specifiable-via-gateway | gpt-6-astra | provider-local-log (not via gateway) | reasoning_effort=null | unknown | 6.83 |
| 18 | R1/b07-claude (stop-vs-cancel) | claude-code | Claude Code CLI 2.1.263 | not-specifiable-via-gateway | claude-fable-5-1 | provider-local-log (not via gateway) | permissionMode=plan | input 2 / cache_creation 5764 / cache_read 2800 / output 2；無 total_cost_usd | 71.11 |
| 19 | R1/b07-codex (stop-vs-cancel) | codex | Codex CLI 0.153.4 | not-specifiable-via-gateway | gpt-6-astra | provider-local-log (not via gateway) | reasoning_effort=null | unknown | 7.46 |
| 20 | R1/a07-claude (emergency stop) | claude-code | Claude Code CLI 2.1.263 | not-specifiable-via-gateway | unknown（estop kill 先於 provider log flush） | provider-local-log (not via gateway) | permissionMode=plan | unknown；spentCost 0.0 | 69.4 |
| 21 | R2/E0-05-delivery-claude | claude-code | Claude Code CLI 2.1.263 | not-specifiable-via-gateway | claude-fable-5-1 | provider-local-log (not via gateway) | permissionMode=plan | costUsd 0.135597 | 9.5 |
| 22 | R2/E0-05-supersede | claude-code | Claude Code CLI 2.1.263 | not-specifiable-via-gateway | claude-fable-5-1 | provider-local-log (not via gateway) | permissionMode=plan | costUsd 0.142408 | 10.05 |
| 23 | R2/E0-05-delete-newtask | claude-code | Claude Code CLI 2.1.263 | not-specifiable-via-gateway | claude-fable-5-1 | provider-local-log (not via gateway) | permissionMode=plan | costUsd 0.128869 | 8.04 |
| 24 | R2/E0-05-old-context-limit（(a) 對已 claimed-completed session 重送 ＋ (b) resume 續開） | claude-code | Claude Code CLI 2.1.263 | not-specifiable-via-gateway | claude-fable-5-1 | provider-local-log (not via gateway) | permissionMode=plan | (b) costUsd 0.0242975（實際擷取）；(a) 最終 messages 4 / costUsd 0.31141175（依據 `step6-memory-before-restart.json` 的 runtime task-memory fact，session asession-922c9167-…），本輪增量≈$0.1958；`step5-old-session-final.json` 的 spentMessages 3 / spentCost 0.135597 是 task-sent 後、回覆抵達前的中繼快照，非最終值 | 85.0 |
| 25 | R3/claude-followup（2 輪，含 30 s 人類延遲） | claude-code | Claude Code CLI 2.1.263 | not-specifiable-via-gateway | claude-fable-5-1 | provider-local-log (not via gateway) | permissionMode=plan | USD 0.18877375（turn1 0.0812 ＋ turn2 0.1076） | 43.94 |
| 26 | R3/codex-followup（2 輪） | codex | Codex CLI 0.153.4 | not-specifiable-via-gateway | gpt-6-astra | provider-local-log (not via gateway) | reasoning_effort=medium | USD 0.0（連接器不回報）；token 數 unknown（未觀察到 tokenUsage 事件） | 42.99 |
| 27 | R3/manual-report（人工 waiting-for-input 後送真任務） | claude-code | Claude Code CLI 2.1.263 | not-specifiable-via-gateway | unknown（本 case 未讀 provider log 取 model，誠實記 unknown） | provider-local-log (not via gateway) | unknown | spentCost 0.141284 | 8.0 |
| 28 | R4/a-codex-deny (E0-10 K-06) | codex | Codex CLI 0.153.4 | not-specifiable-via-gateway | gpt-6-astra | provider-local-log (not via gateway) | unknown（rollout turn_context 無 reasoning_effort）；approval_policy=untrusted；sandbox=read-only | rollout total_token_usage: input 49853 / cached 25856 / output 91 / total 49944；gateway 無 costUsd | 16.32 |
| 29 | R4/b-claude 唯讀收到寫檔任務 | claude-code | Claude Code CLI 2.1.263 | not-specifiable-via-gateway | claude-fable-5-1 | provider-local-log (not via gateway) | permissionMode=plan；thinking_tokens 186 | costUsd 0.19381275；最後一則 usage input 32 / cache_read 8908 / output 673 | 28.0 |
| 30 | R4/b-codex 唯讀＋deny | codex | Codex CLI 0.153.4 | not-specifiable-via-gateway | gpt-6-astra | provider-local-log (not via gateway) | reasoning_output_tokens 24；rollout 無 reasoning_effort 欄 | total_token_usage: input 49869 / cached 37632 / output 129 / total 49998 | 20.43 |
| 31 | R4/c-claude-write 授權寫入 | claude-code | Claude Code CLI 2.1.263 | not-specifiable-via-gateway | claude-fable-5-1 | provider-local-log (not via gateway) | permissionMode=acceptEdits（授權真的下放到工具集） | costUsd 0.1683105 | 10.24 |
| 32 | R4/c-codex-write 授權寫入＋核可 | codex | Codex CLI 0.153.4 | not-specifiable-via-gateway | gpt-6-astra | provider-local-log (not via gateway) | unknown（rollout 無 reasoning_effort）；sandbox=workspace-write（writable_roots 含使用者全域三個目錄） | total_token_usage: input 51384 / cached 38400 / output 69 / total 51453 | 15.09 |
| 33 | R4/d-resume 唯讀續開（同一 provider thread 第二輪） | claude-code | Claude Code CLI 2.1.263 | not-specifiable-via-gateway | claude-fable-5-1 | provider-local-log (not via gateway) | permissionMode 由 acceptEdits 降回 plan；thinking_tokens 145 | record.budget.spentCost 0.33158（累計）；最後一則 usage input 32 / cache_read 11382 / output 495 | 62.0 |
| 34 | R8/claude S1 多輪（K-04，2 輪） | claude-code | Claude Code CLI 2.1.263 | not-specifiable-via-gateway | claude-fable-5-1 | provider-local-log (not via gateway) | permissionMode=plan；turn1 thinking_tokens 259、turn2 0 | USD 0.27653525（turn1 0.129469 ＋ turn2 0.14706625） | 62.8 |
| 35 | R8/claude S2 resume ＋ argv probe（重啟後再續接） | claude-code | Claude Code CLI 2.1.263 | not-specifiable-via-gateway | claude-fable-5-1 | provider-local-log (not via gateway) | permissionMode=plan（argv 顯式 --permission-mode plan、--tools Read,Glob,Grep） | USD 0.04128（S2 0.01802475 ＋ argv probe 0.02325525）；durationSeconds 只涵蓋 S2，argv probe 未計時 | 5.94 |
| 36 | R8/codex S1 多輪（K-04，2 輪） | codex | Codex CLI 0.153.4 | not-specifiable-via-gateway | gpt-6-astra | provider-local-log (not via gateway) | reasoning_effort=medium；approval_policy=untrusted；sandbox=read-only | USD unknown（不回報）；token_count turn1 total 24917、turn2 累計 55195 | 10.66 |
| 37 | R8/codex S2 resume ＋ argv probe | codex | Codex CLI 0.153.4 | not-specifiable-via-gateway | gpt-6-astra | provider-local-log (not via gateway) | reasoning_effort=medium（續接輪 turn_context 仍是 read-only/untrusted） | USD unknown；token_count 續接輪 total 35618、重啟後續接輪 36288 | 8.75 |
| 38 | H-evidence/term-claude（E0-04 SIGTERM） | claude-code | Claude Code CLI 2.1.263 | not-specifiable-via-gateway | unknown（本輪未讀 provider log；provider 端紀錄未歸檔） | not-collected（歸檔證據內無 model 欄） | unknown（未擷取 permissionMode） | spentCost 0.0、spentMessages 1（result.json `after.budget`） | 28.01 |
| 39 | H-evidence/term-codex（E0-04 SIGTERM） | codex | Codex CLI 0.153.4 | not-specifiable-via-gateway | unknown（本輪未讀 provider log；provider 端紀錄未歸檔） | not-collected（歸檔證據內無 model 欄） | unknown（未擷取 reasoning_effort） | spentCost 0.0、spentMessages 1（result.json `after.budget`） | 6.37 |

## 8. 產品缺陷 D1–D20 摘要

> 逐條的位置（`file:line`）、重現命令、完整證據清單與撰稿時的獨立複核在
> [已知問題可重現性 §4](phase-0-known-issues-reproducibility.md)——這裡只放索引，不重抄。
> 「本階段最小修復？」依階段 0 授權（**只允許「阻止驗證的最小修復」**）裁決：只有三條 harness 缺陷符合，
> 其餘一律留給建議階段。（`evidence/2026-09-07-phase-0/analysis/e2e-baseline.json` 對 D1–D4 標 `phase0MinimalFixCandidate=true`，那是**候選**不是裁決；
> 本表與已知問題文件的「本階段最小修復的當下狀態」表一致。）

| id | 標題 | 嚴重度 | 建議階段 | 本階段最小修復？ |
|---|---|---|---|---|
| D1 | 人類 interrupt 真 claude-code session → 終態 failed 而非 cancelled；Claude 連接器在任何路徑都產不出 cancelled | high | 1 | 否——產品行為缺陷／需產品決策，留建議階段 1 |
| D2 | close 的 SSE 投影把已終結的 failed/timed-out/unknown 一律壓成 'closed'，與刻意保留終局狀態的 record 互相矛盾 | high | 1 | 否——產品行為缺陷／需產品決策，留建議階段 1 |
| D3 | failed 的 agent session 在 record 與 mailbox 上都沒有任何原因；唯一線索只存在於 observation（原始 provider token 'error_during_execution'） | medium | 1 | 否——產品行為缺陷／需產品決策，留建議階段 1 |
| D4 | Codex 核可拒絕送出的 wire 值 "reject" 不在 codex 0.153.4 的 CommandExecutionApprovalDecision 列舉內；人類的「拒絕」在 agent 端變成核可機制錯誤而非語意上的拒絕 | medium | 1 | 否——產品行為缺陷／需產品決策，留建議階段 1 |
| D5 | codex 連接器沒有等價於 claude 的 MCP／plugin 封鎖：宣告唯讀的 session 一建立就啟動使用者 ~/.codex 設定裡的 MCP server（chrome-devtools-mcp 瀏覽器控制、codegraph）與 ChatGPT computer-use helper，且任務全文出現在 world-readable 的 process argv | high | 1 | 否——產品行為缺陷／需產品決策，留建議階段 1 |
| D6 | allowWrite 的 codex session 只送 sandbox:"workspace-write" 字串、不送 writable_roots，實際生效範圍併入使用者全域 ~/.codex/config.toml，超出 session 授權的 resolvedWorkdir | medium | 2 | 否——產品行為缺陷／需產品決策，留建議階段 2 |
| D7 | codex.rs 把所有帶 id+method 的 ServerRequest 一律當核可請求，不分辨 item/tool/requestUserInput；resolve_approval 一律回 {result:{decision}}，與 ToolRequestUserInputResponse 的 {answers:{…}} 合約不符，問題文字也被丟棄 | medium | 2 | 否——產品行為缺陷／需產品決策，留建議階段 2 |
| D8 | 沒有『停止這一輪、保留 session』的能力：POST /interrupt 在兩個 agent 上都是 session 級取消，之後送任務一律 409 mailbox closed，子程序也已被殺 | medium | 3 | 否——產品行為缺陷／需產品決策，留建議階段 3 |
| D9 | 續開授權檢查（check_resume_not_wider／check_resume_same_workdir）無論拒絕或接受都不留稽核紀錄；被接受的續開也沒有記下接續了哪一個 provider session | medium | 1 | 否——產品行為缺陷／需產品決策，留建議階段 1 |
| D10 | agent.session.state 的 SSE 投影混用 gateway 階段名（fetched／working）與真實 record 狀態，導致同一時刻 GET 說 created、SSE 說 fetched | low | 2 | 否——產品行為缺陷／需產品決策，留建議階段 2 |
| D11 | waiting-for-input 沒有任何連接器會自動產生：GatewayEvent::TaskWaitingForInput 只有定義與單一消費點，UI 的 NEEDS_INPUT 投影永遠不會被觸發，只能靠人工 POST /report | medium | 2 | 否——產品行為缺陷／需產品決策，留建議階段 2 |
| D12 | 受限 agent token 按下 emergency stop 會把 claimed-completed 的 session 轉成 cancelled，人類之後再也無法對那個聲稱做 verify（409） | low | 3 | 否——產品行為缺陷／需產品決策，留建議階段 3 |
| D13 | user-memory／preference 層沒有接上 supersede／conflict 機制：矛盾的偏好會同時進入每一次 Context Bundle，新舊判斷完全外包給下游 LLM 的文字推論（知識層已有完整狀態機，只是未接通） | low | 4 | 否——產品行為缺陷／需產品決策，留建議階段 4 |
| D14 | 已 claimed-completed 的 session 仍可收新任務並真的執行（by-design），但 GET /v1/agent-sessions/{id} 的 state 全程不反映該次執行——觀測性缺口，不是行為缺陷 | low | 3 | 否——產品行為缺陷／需產品決策，留建議階段 3 |
| D15 | daemon 只處理 SIGINT 的優雅關閉；SIGTERM（一般 kill／process manager stop 的預設訊號）會跳過 InstanceLock 的 Drop，下次啟動印出 stale-lock WARN | low | 2 | 否——產品行為缺陷／需產品決策，留建議階段 2 |
| D16 | codex 子程序 stderr 被完全丟棄，JSON-RPC 協定層錯誤（例如 D4 的無效 decision 值）在 daemon.log 留不下任何痕跡 | low | 1 | 否——產品行為缺陷／需產品決策，留建議階段 1 |
| D17 | 測試 fixture fake_iphone 的 connect() 在任何連線失敗（含『核心暫時離線』）時 die() → process::exit(2)，導致 F-06 的裝置側恢復路徑無法用出貨 fixture 走完 | medium | 1 | **是**（`crates/interaction-runtime/examples/fake_iphone.rs`，未提交）——已修，**已驗證**（H-fixture F-06 四階段 PASS） |
| D18 | AX 測試輔助的 choosefile 無條件回報成功、不驗證 Open panel 是否真的導航，會產生假陽性（panel 停在記憶中的舊目錄並開字母序第一個檔） | medium | 1 | **是**（`scripts/lib/tauri-ax.applescript` 等，未提交）——已修，**未驗證**（無修復後 native 匯入重跑） |
| D19 | AX dump 走 System Events 的 entire contents 遍歷整個視窗，隨 session 累積而單調變慢，超過 driver 的 55 s 上限，導致 E0-08 native 的四個 sensor journey 從未跑到 | medium | 1 | **是**（timeout 55→150 s＋走訪改一次 `properties`，未提交）——已修，**未驗證**（無 10/10 journey 結果） |
| D20 | claude 續開的 session 在 create 回應裡 providerSessionId 仍是 null，且 record 從不保存 resumeProviderSessionId；codex 在 attach 當下就有值，兩個連接器投影時機不一致 | low | 2 | 否——產品行為缺陷／需產品決策，留建議階段 2 |

三條 harness 缺陷的當下狀態（與已知問題文件同步）：**D17 已修（fixture，H-fixture 驗證，修改未提交）**；
**D18 已修、未驗證**；**D19 已修、未驗證**——因此 E0-03／E0-08／E0-11 的 `native-desktop` 欄位仍然是 `harness-failed`，
不得因為「腳本已經改好」就改寫成通過。

## 9. 環境阻礙（代號與逐條詳情見 [已知問題可重現性 §5](phase-0-known-issues-reproducibility.md)）

| 代號 | 一句話 |
|---|---|
| B1 | 真 iPhone 簽章：裝置在、Developer Mode 開，但 Xcode 無 Team，`device-build.sh --check-only` 卡 step 3/5 → 本輪 real-iphone = 0 筆。 |
| B2 | macOS 合成鍵盤事件非決定性掉字，AX 打字進 Tauri WebView 被截斷（`osascript` 仍 exit 0）→ E0-03／E0-02 native。 |
| B3 | NSOpenPanel 記憶舊目錄＋`choosefile` 無條件回報成功（＝D18）→ E0-11 native 匯入無法驗證，且 2026-09-06 綠燈有假陽性之虞。 |
| B4 | 磁碟空間不足，本輪禁止重建，原生 App 綁在 2026-09-06 的 0.7.0 bundle（非 HEAD）。 |
| B5 | Claude 額度／冷啟動：task-sent→active 約 61–65 s；另有一次 21 s 仍 `created` 且 provider log 無 assistant 紀錄（未證實）。 |
| B6 | 共用機器有其他 agent 的 daemon／子程序同時在跑，所有 `ps` 斷言都必須依 ppid／pgid 鏈過濾；不得 `pkill -f interact-ai`。 |

已知問題文件另有 **B2′／B3′**：B2／B3 對應的四支腳本已修改未提交，但**修復後的 native 重跑結果尚未出現在證據目錄**。

## 10. 本階段「沒有驗到」的清單（不得寫成已驗收）

1. **真 iPhone 的任何 E2E**（簽章卡住，B1）；**ESP32 真板**驗收依舊為零。
2. **原生桌面**三項：工作派送與取消（E0-03）、設定匯入與四個阻擋案例（E0-11）、四個 sensor journey（E0-08）。
3. **codex 的 300 s approval TTL 自動拒絕**在執行期的行為（只有靜態證據）。
4. **codex 的 `item/tool/requestUserInput` 真實協定往返**（D7 的後果是靜態推論）。
5. **併發上限 `max_sessions=8`／`max_parallel=4` 的真 agent 執行期驗證**（本輪最多 2 個並行 session）。
6. **codex 的 E-09**（resume 後已刪記憶是否殘留）。
7. **codex `writable_roots` 是否真的可寫入 workdir 之外**（只有 provider 自報的策略，沒有實際寫入證據——刻意不做）。
8. **原生 App 從 HEAD 重建**（磁碟不足，B4）：所有 native 結果綁在 2026-09-06 的 0.7.0 bundle。

已從這份清單移除的兩項（本輪確實補到了證據）：

- `restart_test.py --signal TERM`（乾淨關閉路徑）——H 輪對 claude-code 與 codex 各跑一次，`EXIT=0`（§4 E0-04）。
- E0-08 F-06 的**裝置側**「核心離線→重啟→同一 process 重連」——H 輪以修過（未提交）的 `fake_iphone` 四階段 PASS（§4 E0-08）。
  仍是 **fixture 等級**，不是真機；而且用的不是 HEAD 出貨的 fixture。

## 11. 下一動作

不在這裡重列。階段 1 的第一個切片（D1＋D2＋D3「取消與失敗的誠實終態」）的紅燈測試、實作位置、
同步要修的 D2／D3 與真 agent 迴歸命令，見 [roadmap §1.1](phase-0-roadmap.md#11-下一個實作切片最小完整可驗收)；
之後的排序（D4＋D16、K-01／02／03、bundle 上限矛盾、D5／D6 產品決策、D9 稽核）見 [進度入口 §6](phase-0-progress.md)。
本文件判讀所產生的 17 條逐項待辦（含每一條的檔案、行號與要新增的測試名稱）原文保存在
[`evidence/2026-09-07-phase-0/analysis/e2e-baseline.json`](evidence/2026-09-07-phase-0/analysis/e2e-baseline.json) 的 `nextTasks`
鍵，與 roadmap §1.1 逐項對應。
