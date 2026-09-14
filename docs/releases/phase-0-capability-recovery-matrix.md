# 階段 0：能力恢復矩陣（canonical）

> **這份文件是什麼**：階段 0（2026-09-07／08）對「這個產品現在到底有哪些能力真的能用」的逐列盤點結果，
> 是後續階段的唯一現況依據。它取代 `docs/capability-completion-matrix.md`、`docs/v05-capability-gap-matrix.md`、
> `docs/v05-recovery-matrix.md`、`docs/releases/v0.6.0-recovery-matrix.md` 這四份舊清單（那四份保留為當時的歷史紀錄）。
>
> **來源**：(1) 八個維度的靜態盤點 → 獨立懷疑者逐列反駁 → completeness critic 審查，原始 JSON 已歸檔在
> [`evidence/2026-09-07-phase-0/static-matrix/dim-{A,B,C,D,E,F,G,K}.json`](evidence/2026-09-07-phase-0/static-matrix/)；
> (2) 同一輪的雙平台／多 Session 真 agent E2E 基線，原始資料在
> [`evidence/2026-09-07-phase-0/e2e-runs/`](evidence/2026-09-07-phase-0/e2e-runs/) 與
> [`evidence/2026-09-07-phase-0/real-agent-e2e-prior/`](evidence/2026-09-07-phase-0/real-agent-e2e-prior/)，
> 彙整表在 [`evidence/2026-09-07-phase-0/analysis/e2e-baseline.json`](evidence/2026-09-07-phase-0/analysis/e2e-baseline.json)；
> (3) repo 事實見 [phase-0-repository-state.md](phase-0-repository-state.md)。
> 維度代號與標題（A 電腦與核心／B 人類與 Agent 溝通／C 多 Agent／D 策略與上下文／E 記憶與知識／F iPhone／
> G 視覺化與一般模式／K 模型選擇與 connector 相容）取自本輪盤點的工作定義，來源 JSON 的 `dimension` 欄只有代號。
>
> **怎麼再產生**：靜態盤點是逐維度讀 repo（Read／Grep，不建置）後由獨立 agent 反駁；本文件把三份輸出
> （`found.rows`／`verdicts`／`critic`）整合成一張表。執行期欄位要重跑的話，用 repo 內的
> `scripts/tests/phase0/{agent_smoke.py,restart_test.py,multi_session.py}`（本輪實際使用的副本已歸檔在
> [`evidence/2026-09-07-phase-0/harness-used/`](evidence/2026-09-07-phase-0/harness-used/)）。每列的
> 「限制與下一動作」都可獨立驗證，命令候選見 §5.2。
>
> **證據 commit**：全部靜態列一律以 `78dcda1`（= origin/main，v0.8.0 tag `1fa69b8` 之後只有 docs 與測試 commit）為準。
> 執行期證據另附檔案路徑。

---

## 0. 說明與圖例

### 0.1 狀態（只用這六個值）

| 狀態 | 意思 |
|---|---|
| `implemented-and-connected` | 從正式入口（HTTP／CLI／Tauri／UI）可達，關鍵環都在，且 repo 內有斷言覆蓋 |
| `implemented-partial` | 已接線但缺關鍵環（少一個入口、少一層投影、或關鍵行為只有靜態推論） |
| `defined-only` | 型別／欄位／enum／trait 存在，但沒有任何生產路徑會走到 |
| `absent` | 需求對應的機制在 repo 內不存在 |
| `confirmed-defect` | 能指到具體錯誤路徑（file:line）的缺陷，不是「還沒做」 |
| `needs-investigation` | 從現有證據無法判定；本文件不以猜測填空 |

**型別、欄位、enum variant、port trait 存在不等於 production 功能**——這是 `defined-only` 與
`implemented-and-connected` 的分界，本矩陣一律照此判。

### 0.2 證據層級（只用這十二個值）

`static-inspection`（讀碼／讀文件）、`unit`（crate 內 `#[test]`／vitest／XCTest 純函式）、
`contract`（golden schema／跨語言 conformance fixture）、`fixture`（fake_claude.sh／fake_codex.sh／
fake_iphone 等替身子程序）、`simulator`（iOS 模擬器）、`integration`（起真 daemon 的 crates/*/tests 與 CLI E2E）、
`browser`（Playwright＋Chromium＋真 daemon）、`native-desktop`（真 Tauri App＋AX 走查）、
`real-agent`（真 Claude Code／Codex 二進位）、`real-iphone`（真 iPhone 裝置）、
`real-hardware`（真 ESP32／Serial／BLE 硬體）、`not-run`（本輪未執行）。

**本階段的分層事實**：

| 層級 | 本階段（HEAD `78dcda1`）狀況 |
|---|---|
| `real-agent` | **有**：本輪以 Claude Code 2.1.263 與 Codex 0.153.4 實跑 E0-02…E0-11，證據在 `e2e-runs/`。但 repo 內的自動化測試仍是 0 筆真 agent（`E2E_REAL_AGENTS=1` 時 Playwright session 測試 skip），所以靜態列的 evidenceLevel 一律不標 `real-agent`，只在「本次結果」欄引用 E2E 案例 |
| `real-iphone` | **本階段 0 筆**。F-01／F-03／F-05 的 `real-iphone` 標記來自 [v0.5.0-iphone-device-evidence.md](v0.5.0-iphone-device-evidence.md)（2026-09-03、v0.5.0 建置），不是 HEAD。本輪 E0-08 真機列為 needs-environment：iPhone 11 已連線且 Developer Mode 已開，但 Xcode 未選 Team，`apps/interaction-ios/scripts/device-build.sh` 停在 3/5 |
| `real-hardware` | **本階段 0 筆**，ESP32／Serial／BLE 真板驗收累計仍為零 |
| `native-desktop` | 引用的原生證據全部綁在 **2026-09-06 建置的 App**（Info.plist 版本字串 0.7.0、source `87b0d031`），不是由 HEAD 重建（磁碟只剩約 12 GB，本輪禁止 cargo／pnpm build）。本輪自己的四次原生走查有三次 harness-failed，見 §4 與 §1.3 |

### 0.3 每列的固定欄位

`使用者需求`／`實作 owner`／`正式入口及呼叫路徑`／`資料與協定契約`／`狀態與儲存 owner`／`現有測試`／
`證據 commit`／`本次結果`（狀態＋證據層級＋對應的 E0-xx 執行期結果）／`限制與下一動作`。

「現有測試」列的是 **repo 內既有的測試檔與測試名**，不代表本輪跑過；本輪實跑了哪些見
`evidence/2026-09-07-phase-0/baseline/` 與 `e2e-runs/`。

---

## 1. 總表

### 1.1 狀態分布（數 `found.rows`，並套用 §3 的改判）

| 維度 | 列數 | implemented-and-connected | implemented-partial | defined-only | absent | confirmed-defect | needs-investigation |
|---|---|---|---|---|---|---|---|
| A 電腦與核心 | 12 | 9 | 1 | 2 | 0 | 0 | 0 |
| B 人類與 Agent 溝通 | 10 | 3 | 5 | 1 | 0 | 1 | 0 |
| C 多 Agent | 10 | 4 | 6 | 0 | 0 | 0 | 0 |
| D 策略與上下文 | 6 | 1 | 4 | 0 | 0 | 1 | 0 |
| E 記憶與知識 | 9 | 2 | 5 | 0 | 0 | 2 | 0 |
| F iPhone | 6 | 3 | 2 | 0 | 1 | 0 | 0 |
| G 視覺化與一般模式 | 6 | 3 | 2 | 0 | 0 | 1 | 0 |
| K 模型選擇與 connector 相容 | 8 | 2 | 2 | 0 | 3 | 1 | 0 |
| **合計** | **67** | **27** | **27** | **3** | **4** | **6** | **0** |

六個 `confirmed-defect`：B-07（真 Claude interrupt 產不出 cancelled）、D-06 與 E-04（同一個
bundle 48 KiB vs mailbox 16 KiB 的位元組矛盾，兩個維度各記一列）、E-09（刪除記憶後已派送的副本不可收回且無標示）、
G-06（動作收據 `completed` 在桌面被畫成綠色成功）、K-06（codex 送出的 `"reject"` 不在 0.153.4 的合法列舉內）。

### 1.2 執行期基線（E0-01…E0-11）

來源 [`analysis/e2e-baseline.json`](evidence/2026-09-07-phase-0/analysis/e2e-baseline.json) 的 `table`。
`overall` 照抄該檔，未自行改寫。

| 案例 | 情境 | overall | 對應矩陣列 |
|---|---|---|---|
| E0-01 | 首次設定＋偏好重啟保留；native 端 10 種注入故障後的預設恢復 | completed | A-02、A-03、A-08a |
| E0-02 | 派工給真 agent 並比對 ground truth；多輪與 `resumeProviderSessionId` 原生續接 | completed | A-05、B-01、B-08、C-01、C-04、D-03、K-04 |
| E0-03 | 取消／關閉／緊急停止的分類誠實度與子程序死亡 | **product-failed** | B-07、A-07、A-07a、C-04、C-08 |
| E0-04 | daemon 被 SIGKILL 後重啟：狀態誠實度、mailbox／lease 邊界、孤兒回收 | completed | A-08a、B-05、D-06 |
| E0-05 | 記憶→Context Bundle→真 agent 讀取→刪除→resume 後仍覆誦（E-09） | completed | D-03、D-04、E-01、E-03、E-09 |
| E0-06 | 同型兩 session 併發：隔離、取消獨立性 | completed（併發上限 needs-environment） | C-02、C-05 |
| E0-07 | claude＋codex 跨型並行，取消其中的 claude | completed | C-03、C-08 |
| E0-08 | iPhone：配對、雙向、停止結果未知、AIP、重連、撤銷、核心離線 | completed（fixture 範圍）／needs-environment（真機） | F-01、F-02、F-05、F-06 |
| E0-09 | agent 需要人類補充資訊時的非同步問答；waiting-for-input 自動偵測 | **not-implemented**（自動偵測不存在；人工 POST /report 路徑 completed） | A-05a、B-04、G-01 |
| E0-10 | 權限：唯讀阻擋、拒絕核可、授權寫入＋human verify、撤銷、TTL、續開不放寬 | correctly-blocked（deny 語意有 product-failed 缺陷） | A-06、C-06、D-05、K-05、K-06 |
| E0-11 | 備份匯出範圍、重啟保留、全新 home 還原、壞資料被擋；native 端真下載＋NSOpenPanel | completed（API／CLI）＋correctly-blocked（壞資料 400）＋harness-failed（native 匯入） | A-08 |

雙平台判讀（照抄來源）：Claude Code 走完了派工／多輪／原生 `--resume` 續接／唯讀與授權寫入／human verify／
撤銷／過期／重啟恢復／記憶與 E-09，唯一的 product-failed 是人類 interrupt 被記成 `failed`；Codex 走完同一組流程
且 interrupt→cancelled 正確，但有兩個 product-failed 級缺陷（`"reject"` 不在合法列舉內；無 MCP／plugin 封鎖）。
兩個平台共同 not-implemented 的是「停止這一輪但保留 session」與 waiting-for-input 的自動偵測。

多 Session 判讀（照抄來源）：同型 claude×2、同型 codex×2、跨型 claude＋codex 各跑一次，結果不交叉洩漏、
SSE 的 session 範圍事件 0 筆混淆、取消其一不影響另一個、核可只作用在發出請求的 session；唯一沒驗到的是
併發上限本身（`max_sessions=8`／`max_parallel=4` 在原始碼確實實作，本輪最多只跑 2 個並行 session）。

### 1.3 執行期缺陷 D1–D20 → 矩陣列索引

`severity` 與 `location` 照抄 `analysis/e2e-baseline.json` 的 `productDefects`。

| 缺陷 | 嚴重度 | 一句話 | 主要落點（file:line） | 對應列 |
|---|---|---|---|---|
| D1 | high | 人類 interrupt 真 claude session → 終態 `failed` 而非 `cancelled`；claude 連接器任何路徑都產不出 cancelled | `crates/interaction-agent-gateway/src/claude.rs:354-357`、`:442-455`、`:213-231` → `gateway.rs:579-584` → `agents.rs:1185` | B-07、A-07、C-04 |
| D2 | high | close 的 SSE 投影把已終結的 failed／timed-out／unknown 一律壓成 `closed`，與刻意保留終局狀態的 record 相矛盾 | `crates/interaction-runtime/src/agents.rs:1371-1379` vs `:1296-1310` | A-07、B-07、G-06 |
| D3 | medium | `failed` 的 session 在 record 與 mailbox 都沒有原因，唯一線索只在 observation | `agents.rs:1209-1218`、`:1254-1257` | B-07、K-08 |
| D4 | medium | Codex 拒絕送出的 wire 值 `"reject"` 不在 0.153.4 的列舉內；人類的「拒絕」在 agent 端變成核可機制錯誤 | `crates/interaction-agent-gateway/src/codex.rs:643-644` | K-06、K-05、A-06 |
| D5 | high | codex 連接器沒有等價於 claude 的 MCP／plugin 封鎖：唯讀 session 一建立就啟動使用者 `~/.codex` 的 MCP server，任務全文出現在 process argv | `codex.rs:166-169` 對照 `claude.rs:82-84` | K-05、D-05、C-04 |
| D6 | medium | allowWrite 的 codex session 只送 `sandbox:"workspace-write"`，實際生效範圍併入使用者全域設定 | `codex.rs:315-340` | K-05、C-06 |
| D7 | medium | codex.rs 把所有帶 id＋method 的 ServerRequest 一律當核可請求，回應形狀與 `requestUserInput` 合約不符 | `codex.rs:250-263`、`:432-445`、`:623-660` | K-06、C-04 |
| D8 | medium | 沒有「停止這一輪、保留 session」的能力：interrupt 在兩個 agent 上都是 session 級取消 | `agents.rs:900-904`、`gateway.rs:579-588` | B-07、C-04 |
| D9 | medium | 續開授權檢查（拒絕或接受）都不留稽核紀錄，接受的續開也沒記下接續了哪一個 provider session | `agents.rs:453-468`、`:105`、`:204` | B-08、C-10、K-04 |
| D10 | low | `agent.session.state` 的 SSE 投影混用 gateway 階段名（fetched／working）與真實 record 狀態 | `gateway.rs:798`、`agents.rs:1060` | B-02、G-02 |
| D11 | medium | waiting-for-input 沒有任何連接器會產生，UI 的 NEEDS_INPUT 投影永遠不會被觸發 | `crates/interaction-agent-gateway/src/lib.rs:99`、`gateway.rs:412-417`、`agents.rs:1182,1248` | A-05a、B-04、G-01 |
| D12 | low | 受限 agent token 按下 emergency stop 會把 claimed-completed 轉成 cancelled，人類之後再也無法 verify | `crates/interaction-api/src/lib.rs:454-461`、`agents.rs:1120-1150` | A-07a、B-10 |
| D13 | low | user-memory／preference 層沒接上 supersede／conflict，矛盾偏好會同時進每一次 bundle | `memory.rs:279-353` 對照 `knowledge.rs:1182-1225` | E-07、D-04 |
| D14 | low | 已 claimed-completed 的 session 仍可收新任務並真的執行（by-design），但 GET 的 state 全程不反映——觀測性缺口 | `crates/interaction-core/src/agent.rs:48-55`、`agents.rs:899-905` | B-09、G-06 |
| D15 | low | daemon 只處理 SIGINT 的優雅關閉；SIGTERM 會跳過 InstanceLock 的 Drop | `crates/interaction-cli/src/commands.rs:1608`、`crates/interaction-runtime/src/lock.rs:84-94` | A-02、A-08a |
| D16 | low | codex 子程序 stderr 被完全丟棄，協定層錯誤在 daemon.log 留不下痕跡 | `codex.rs:176-181` | K-06 |
| D17 | medium | 出貨 fixture `fake_iphone` 在任何連線失敗時 `process::exit(2)`，F-06 的裝置側恢復路徑走不完 | `crates/interaction-runtime/examples/fake_iphone.rs:112-115`、`:158-177`、`:296-298` | F-06 |
| D18 | medium | AX 測試輔助的 choosefile 無條件回報成功、不驗證 Open panel 是否真的導航（假陽性） | `scripts/lib/tauri-ax.applescript:51-63` | A-08（原生證據可信度） |
| D19 | medium | AX dump 走 System Events 遍歷整個視窗，隨 session 累積變慢並超過 driver 的 55 s 上限 | `scripts/lib/tauri-ax.applescript`＋`scripts/tests/tauri-mobile-sensor-walkthrough.py:225` | F-01／F-05 的原生證據 |
| D20 | low | claude 續開的 session 在 create 回應裡 `providerSessionId` 仍是 null，且 record 從不保存 `resumeProviderSessionId` | `agents.rs:547`、`:619-621`、`:277` | K-02、B-08、C-01 |

D1、D2、D3、D8、D11 五項的共同後果是：**使用者按下「中斷」之後，畫面上得到的是「動作失敗」而不是「已取消」，
而且沒有任何地方說得出原因**。這是本階段執行期唯一橫跨兩個平台的高嚴重度組合。

---

## 2. 逐維度逐列

### 2.A 維度 A — 電腦與核心（12 列）

#### A-01 安裝

| 欄位 | 內容 |
|---|---|
| 使用者需求 | 安裝：`get.sh`（Release 附 `install.sh`）／`interact-ai self install-skill`、`install-desktop`、`update`、`uninstall`／DMG 桌面安裝包／從原始碼 cargo 建置 |
| 實作 owner | `scripts/get.sh` ＋ `crates/interaction-cli/src/selfmgmt.rs` ＋ `.github/workflows/release.yml` |
| 正式入口及呼叫路徑 | `scripts/get.sh:64-176`（平台偵測→下載→sha256 fail-closed→install→呼叫 `self install-skill`／`install-desktop`）→ `commands.rs:433-445`（SelfCmd 分派）→ `selfmgmt.rs:441`／`:547-582`／`:251-316`／`:318`；`release.yml:85-131`（CLI matrix）、`:136-179`（desktop 包） |
| 資料與協定契約 | `docs/INSTALL.md`（平台覆蓋表、sha256-only、未簽章／未公證、Linux aarch64 無預編譯）；`scripts/tests/docs-claims.sh:127-139` 把 INSTALL.md 的誠實宣稱與 selfmgmt 逃生門綁在一起 |
| 狀態與儲存 owner | 檔案系統：`$BIN_DIR/interact-ai`、`~/.local/share/interact-ai/completions`、各 agent home 的 `skills/`；資料目錄由 `INTERACT_AI_HOME` 或 `~/.adaptive-interaction` 決定（`config.rs:64-73`） |
| 現有測試 | `selfmgmt.rs:588-655`（unit）；`scripts/tests/release-scripts.sh:265-321`（真的執行 get.sh、stub 掉 curl／gh，斷言缺 .sha256 時 fail-closed 且不落地二進位）；`scripts/tests/docs-claims.sh:127-139` |
| 證據 commit | `78dcda1` |
| 本次結果 | `implemented-and-connected`；證據層級 `static-inspection`＋`unit`＋`integration`＋`not-run`（端到端安裝流程本輪未跑）。無對應 E0-xx |
| 限制與下一動作 | `get.sh:64-73` 只認 Darwin/arm64、Darwin/x86_64、Linux/x86_64，**沒有 Windows 分支**，而 `docs/INSTALL.md:79-90` 把 Windows x64 CLI 標為可用——見 §4 的 A-X4 裁定（文件缺陷）。DMG／exe／msi 未簽章未公證。下一動作：修 INSTALL.md 的 Windows 列或給 get.sh 加分支；把 `install.sh --cli-only` smoke 寫成可重跑腳本 |

#### A-02 啟動與單一 Runtime

| 欄位 | 內容 |
|---|---|
| 使用者需求 | `interact-ai serve` 前景 daemon；Tauri 控制中心啟動時是否自己 spawn daemon、如何發現既有 daemon、token 如何取得；單一 Runtime 保證 |
| 實作 owner | `crates/interaction-cli/src/commands.rs`（serve）＋ `apps/interaction-desktop/src-tauri/src/{lib.rs,supervisor.rs}` ＋ `crates/interaction-runtime/src/{runtime.rs,lock.rs,config.rs}` |
| 正式入口及呼叫路徑 | CLI `commands.rs:430-432` → `:1572-1620` serve() → `Runtime::start`（`runtime.rs:266-267` `InstanceLock::acquire`）→ `config.rs:337-366` token → `interaction_api::serve`。Tauri `lib.rs:3510-3575` setup → `start_supervised`（`:3336-3508`）：先打 `GET /ready`（`supervisor.rs:392-449`）命中則 External＋讀 `state/api-token`；沒有 daemon 則同進程 `Runtime::start`＋`interaction_api::serve`（`:3403-3460`），bind 失敗即 Degraded。前端 `desktop.ts:176-194` 輪詢 `supervisor_info`，External 且有 token 時 `transport.configureHttp` 改走 `/v1/*` |
| 資料與協定契約 | `supervisor.rs:1-11`（永不雙 Runtime；完全結束不殺外部 daemon；關窗只隱藏）；`docs/INSTALL.md` §4；`lock.rs:1-7`（O_EXCL＋stale PID 回收） |
| 狀態與儲存 owner | `state/runtime.lock`、`state/api-token`／`api-agent-token`、`config/interaction.yaml`、Tauri `AppState.supervisor`、`state/desktop.json` |
| 現有測試 | `supervisor.rs:475-590`（unit）；`apps/interaction-desktop/e2e/offline.spec.ts:6`（browser）；`scripts/v03-cli-e2e.sh:15-30`（integration）；`scripts/tests/tauri-preset-recovery.py`、`scripts/tauri-ax-walkthrough.sh`（native-desktop，皆不在 CI） |
| 證據 commit | `78dcda1` |
| 本次結果 | `implemented-and-connected`；`static-inspection`＋`unit`＋`integration`＋`browser`＋`native-desktop`（原生證據為 2026-09-06 bundle）。E0-01＝completed（`e2e-runs/R5/E0-01-step{1,2,3}.txt`、`e2e-runs/R7/preset/result.json` 10/10 故障案例收斂、`e2e-runs/R7/preset.cmd.txt` EXIT=0） |
| 限制與下一動作 | 外部模式 token 讀不到只會 Degraded，沒有 UI 讓使用者貼 token。**D15**：daemon 只處理 SIGINT，SIGTERM 會跳過 `InstanceLock` 的 Drop（`commands.rs:1608`、`lock.rs:84-94`），下次啟動印 stale-lock WARN；E0-04 的 SIGTERM 乾淨關閉路徑本輪未跑（改判 harness-failed，見 §4）。下一動作：處理 SIGTERM；補外部模式 token 復原入口 |

#### A-03 首次設定 onboarding

| 欄位 | 內容 |
|---|---|
| 使用者需求 | `/v1/onboarding`（GET）、`/draft`（PUT）、`/preview`（POST 乾跑）、`/commit`（POST）；`Onboarding.tsx` 三步精靈；`FirstSuccess.tsx`；重新執行入口 |
| 實作 owner | `crates/interaction-runtime/src/human.rs` ＋ `apps/interaction-desktop/src/pages/{Onboarding.tsx,FirstSuccess.tsx}` |
| 正式入口及呼叫路徑 | HTTP `lib.rs:55-61` → `routes.rs:1095-1120` → `human.rs:641`／`:667`／`:683`／`:850`（commit 兩段式：檔案原子寫＋備份 → SQLite 交易；失敗 `restore_file_backups`＋`onboarding_partial` 稽核）。Tauri `lib.rs:982-1010`。CLI `commands.rs:1159` 只 GET。閘門 `App.tsx:288-289` 讀 `status.onboardingCompleted`（`runtime.rs:636`） |
| 資料與協定契約 | `docs/aip/general-mode-ux.md`（五入口）；`human.rs:679-682`／`:850` 註解（preview 與 commit 同驗證同 diff、preview 零副作用）；`docs/DESKTOP-GUIDE.md` |
| 狀態與儲存 owner | SQLite meta：`onboarding`、`onboarding-draft`、`ui-preferences`（`human.rs:559-573`）；`config/policies/policy.yaml`、`config/recipes/*`；`state/desktop.json`（角色可見度／表現程度） |
| 現有測試 | `crates/interaction-runtime/tests/human_layer.rs:305`、`:747`、`:799`、`:850`、`:899`、`:1006`、`:1085-1260`（commit 失敗回滾路徑）；`crates/interaction-api/tests/api_e2e.rs:894`；`src/test/onboarding.test.tsx:237-590`、`firstSuccess.test.tsx:81-189`；`e2e/app.spec.ts:16`、`general-mode-tasks.spec.ts:574`、`evidence.spec.ts:658`；`scripts/tauri-ax-walkthrough.sh` |
| 證據 commit | `78dcda1` |
| 本次結果 | `implemented-and-connected`；`static-inspection`＋`unit`＋`integration`＋`browser`＋`native-desktop`。E0-01＝completed |
| 限制與下一動作 | commit 之外的三個副作用（`proactiveDialoguePatch`、`desktop.prefsPatch` ×2）不在 commit 交易內；瀏覽器模式無法寫桌面偏好（誠實跳過）。onboarding 狀態綁 `INTERACT_AI_HOME`。下一動作：維持 |

#### A-04 角色顯示／切換／互動

| 欄位 | 內容 |
|---|---|
| 使用者需求 | 角色視窗、內建 adapter（shu／sprite／text／shape）切換、外部 adapter（`/v1/character/ws`）、Character Session（權威狀態、桌面觸摸事件） |
| 實作 owner | `crates/interaction-runtime/src/{character.rs,character_session.rs}` ＋ `crates/interaction-api/src/{routes.rs,character_ws.rs}` ＋ `src-tauri/src/{lib.rs,character_bridge.rs,character_store.rs,host_safety.rs}` ＋ `apps/interaction-desktop/src/{character/,companion/,pages/CompanionPage.tsx}` |
| 正式入口及呼叫路徑 | 顯示：`lib.rs:3552-3561` → `ensure_companion_window`（`:2410-2450`）→ `CompanionApp.tsx:731 api.characterHello` → `character.rs:947 character_hello`（manifest 驗證、拒 external-process／remote-device、reduced-motion 協商）→ 投影 `character.rs:467-539` → 桌面 in-process gateway → adapter。切換：`CompanionPage.tsx:391-397` → `desktop_prefs_patch`（`lib.rs:2147`）→ `supervisor.rs:411-470` 原子寫 `state/desktop.json`。外部 adapter：`/v1/character/adapters`＋`/v1/character/ws`（`lib.rs:107-118`、`:331`）→ `character.rs:1236`／`:1289`／`:1423`。Character Session：`routes.rs:1907-1975`（human token）與 Tauri `lib.rs:1832-1868` → `character_session.rs:1244`／`:1365`／`:1416` |
| 資料與協定契約 | `docs/character-protocol/README.md`（CPP 1.0 唯一契約；golden `schemas/character-protocol.schema.json`）；`docs/aip/character-session.md`；不變量「角色呈現層沒有權限主權」（`character.rs:1371` 拒安全 intent；`api lib.rs:425-428` adapter token 只能 POST receipts／events） |
| 狀態與儲存 owner | CharacterHub（記憶體；adapter token sha256 存 SQLite `character_adapters`）；`state/desktop.json`；`state/characters/<id>/`；`state/character-session.json`＋`.epoch` |
| 現有測試 | `character_loop.rs:204-2389`；`character_session_loop.rs:454-1952`；`api_e2e.rs:1577`、`:1879`、`:2026`、`:2705`；vitest `character-gateway`／`character-adapters`／`character-shu-adapter` 等；`e2e/character.spec.ts:35-87`、`character-session.spec.ts:263-351`、`app.spec.ts:139`；`scripts/v03-cli-e2e.sh:456-519`、`:596-615`；`scripts/tauri-ax-walkthrough.sh` |
| 證據 commit | `78dcda1` |
| 本次結果 | `implemented-and-connected`；`static-inspection`＋`unit`＋`contract`＋`fixture`＋`integration`＋`browser`＋`native-desktop`（原生證據為 2026-09-06 bundle）。無直接 E0-xx（E0-08 的 fixture iPhone 觸摸間接覆蓋 Character Session） |
| 限制與下一動作 | 真 Tauri 視窗的換角色／顯示隱藏證據只來自不在 CI 的 AX 走查；外部 adapter 只有 fixture（`examples/character-adapters/text-adapter.mjs`）。下一動作：維持；重跑 AX 走查時附 JSON 與 binary digest |

#### A-05 工作建立／狀態／成果

| 欄位 | 內容 |
|---|---|
| 使用者需求 | `WorkPage`／`TaskComposer` 交代 → `/v1/agent-sessions` 建立 → 子程序（Claude `-p stream-json`／Codex app-server）→ 狀態機各值 → 成果（mailbox result／artifact／人工驗證）呈現 |
| 實作 owner | `crates/interaction-runtime/src/{agents.rs,gateway.rs}` ＋ `crates/interaction-agent-gateway` ＋ `crates/interaction-core/src/agent.rs` ＋ `apps/interaction-desktop/src/pages/{WorkPage.tsx,work/TaskComposer.tsx,AiPage.tsx}` ＋ `statusProjection/workState.ts` |
| 正式入口及呼叫路徑 | 建立：`TaskComposer.tsx:469-478` → `routes.rs:1389`／Tauri `lib.rs:1573` → `agents.rs:387 create_agent_session`（`agent_create_lock` 序列化；estop 拒；allow_write 需 gateway agent＋workdir＋兩個 scope；resume 不得放寬；TTL clamp 1..1440）→ `gateway_attach`（`gateway.rs:307-408`：Codex maxCost／intent-only 拒絕、`resolve_gateway_workdir` 排除 state 目錄、注入 session token 與 API base）→ `spawn_gateway_pump`（`:412-700`）。送任務：`agentSessionSend("task")` → `agents.rs:852 mailbox_send` → `gateway_deliver`（`gateway.rs:704-826`）。狀態機：`report_agent_session`（`agents.rs:1174-1263`）；enum `agent.rs:17-45`；UI 投影 `workState.ts:116-190`。成果：`AiPage.tsx:528 agentSessionMessages('from-session')`；驗證 `agents.rs:1111 verify_agent_session`（human-only、綁 claim_id） |
| 資料與協定契約 | `docs/aip/semantic-state.md`／`statusProjection`；CLAUDE.md 誠實階梯（claimed≠verified）；`agent.rs:1-8`（session 是租約不是身分；report 是 claim） |
| 狀態與儲存 owner | `Runtime.agent_sessions`（記憶體 map＋mailbox VecDeque）＋ SQLite `agent_sessions`；meta `gateway_pgids`；子程序 process group |
| 現有測試 | `agents_loop.rs:49-1336`；`gateway_loop.rs:138-2757`（fixture 子程序）；`api_e2e.rs:1238`、`:1336`、`:2361`；vitest `workPage.test.tsx`、`aiPageResume.test.tsx`、`statusProjection.test.tsx`；`e2e/work-delegate.spec.ts:110-327`、`general-mode-tasks.spec.ts:1155`、`home-state.spec.ts:76-203`、`evidence.spec.ts:320`；`scripts/v03-cli-e2e.sh:200-280`；`scripts/tests/tauri-work-cancel.py` |
| 證據 commit | `78dcda1` |
| 本次結果 | `implemented-and-connected`；`static-inspection`＋`unit`＋`fixture`＋`integration`＋`browser`＋`native-desktop`（**§4 A-X1 裁定新增**：`evidence/2026-09-06-convergence/final/native-final-checkpoint/runs.json` 記錄 `work-clean` exit 0／52.936 s，產物含 `codex-turn-cancelled-ax.txt`、`claude-long-work-closed-and-processes-exited-ax.txt`；2026-09-06 bundle、fixture agent）。**E0-02＝completed**（真 Claude Code：答案逐字正確、claimed-completed、spentCost 0.14471075、多輪不重新 spawn；真 Codex：`--approve approve` 走完，答案逐字正確；證據 `e2e-runs/R8/claude/{run.txt,raw.json,sse.jsonl}`、`real-agent-e2e-prior/claude-smoke-1/`、`real-agent-e2e-prior/codex-smoke-{1,2}/result.json`）。本輪 native 派工＝harness-failed（AX composer 打字被截斷） |
| 限制與下一動作 | 成果呈現只有 mailbox 文字與 artifact 路徑字串，沒有檔案清單／diff；Claude 路徑無法回報 USD 以外的預算。下一動作：把 `ArtifactProduced`／`fileChange` 寫進信箱並在卡片列出；修 AX 輸入（見 §4 環境阻礙） |

#### A-05a 工作狀態 waiting-for-input

| 欄位 | 內容 |
|---|---|
| 使用者需求 | agent 卡在等人類回答時 UI 應顯示「等你回答」 |
| 實作 owner | `crates/interaction-core/src/agent.rs` ＋ `crates/interaction-agent-gateway/src/lib.rs` ＋ `crates/interaction-runtime/src/gateway.rs` |
| 正式入口及呼叫路徑 | 型別 `agent.rs:24-30 AgentSessionState::WaitingForInput`（註解自陳 HONEST GAP）；`gateway lib.rs:95-99 GatewayEvent::TaskWaitingForInput`；泵有處理分支 `gateway.rs:437-443` → `agents.rs:1181`；唯一可達路徑是 human token／Tauri 呼叫 `POST /v1/agent-sessions/{id}/report event=waiting-for-input`（`routes.rs:1414`）；UI `workState.ts:126-128` |
| 資料與協定契約 | `docs/DESKTOP-GUIDE.md:181`、`:289` 把「等你回答」列為使用者會看到的狀態；semantic-state 契約含 waiting-input |
| 狀態與儲存 owner | `AgentSessionRecord.state`（SQLite `agent_sessions`） |
| 現有測試 | `agents_loop.rs:681`（經人工 report 路徑觸發，不是 connector 推論）；`src/test/statusProjection.test.tsx`（投影文案） |
| 證據 commit | `78dcda1` |
| 本次結果 | `defined-only`；`static-inspection`＋`unit`。**E0-09＝not-implemented**（自動偵測不存在；人工 POST /report 路徑 completed，證據 `e2e-runs/R3/manual-report/final-after-clear.json`（`state":"claimed-completed"`））。真 agent 的提問會以 turn 結束到達，被記成 claimed-completed（`e2e-runs/R3/{claude-followup,codex-followup}/result.json`）。對應缺陷 **D11** |
| 限制與下一動作 | gateway agent 永遠不會進入「等你回答」，文件卻把它列為一般狀態。下一動作：在 DESKTOP-GUIDE／FEATURES 標明只對自報型 agent 成立，或移除該狀態；若 Claude Code 2.1.x stream-json 已有對應事件則在 `claude.rs` 補接 |

#### A-05b 工作建立時指定模型

| 欄位 | 內容 |
|---|---|
| 使用者需求 | `SessionSpec.model` → `claude --model` |
| 實作 owner | `crates/interaction-agent-gateway/src/{lib.rs,claude.rs}` ＋ `crates/interaction-runtime/src/{agents.rs,gateway.rs}` |
| 正式入口及呼叫路徑 | `gateway lib.rs:166 SessionSpec.model`（預設 None）；`claude.rs:103-104` 有值才加 `--model`；`gateway.rs:355-370` 組 SessionSpec 時**從未設定 model**；`CreateAgentSession`（`agents.rs:247-278`）無 model 欄位；CLI／TaskComposer／AiPage 均無 |
| 資料與協定契約 | 無（API／CLI／UI 都沒有承諾可選模型） |
| 狀態與儲存 owner | n/a |
| 現有測試 | 無（`gateway_loop.rs` 全檔 grep `--model` 0 命中） |
| 證據 commit | `78dcda1` |
| 本次結果 | `defined-only`；`static-inspection`。與 K-01 同一缺口；E0-02 的 `e2e-runs/R8/argv-claude/run.txt` 記錄了真 claude 的 argv（含 `--resume`），可作為「argv 內沒有 `--model`」的對照 |
| 限制與下一動作 | create payload 傳不進 model，連線用各 CLI 自身預設。下一動作見 K-01 |

#### A-06 核准／拒絕／撤銷

| 欄位 | 內容 |
|---|---|
| 使用者需求 | agent approval（`/approve`）、session consent scope（grant／revoke／stop）、撤銷 provider／裝置，且 agent token 不得授權 |
| 實作 owner | `gateway.rs`（approval）＋ `runtime.rs`（consent）＋ `crates/interaction-api/src/{lib.rs,routes.rs}`（principal 邊界）＋ `pages/{AiPage,SafetyPage,HomePage}.tsx` |
| 正式入口及呼叫路徑 | approval：`AiPage.tsx:834/846` → `routes.rs:2023`／Tauri `lib.rs:1297`／CLI `agents approve` → `gateway_resolve_approval`（`gateway.rs:828`）→ `resolve_approval_as`（`:843-975`：in_flight 防重入、送達失敗保留登記、一律回寫 mailbox、audit）；來源 `TaskWaitingForConsent`（`codex.rs` ServerRequest → `gateway.rs:445-480`）；逾時自動拒絕 `gateway_sweep`（`:1034-1130`）。consent：`SafetyPage.tsx:495`／`:322`、`HomePage.tsx:341` → `routes.rs:469-491` → `runtime.rs:1360`／`:1415`／`:1461`。邊界：`api lib.rs:363-420`、`:488-497` |
| 資料與協定契約 | CLAUDE.md 不變量（AI 不可授予 consent／不可提高上限）；`docs/aip/threat-model.md`；`gateway.rs:1034-1040`（逾時＝拒絕，絕不代為同意） |
| 狀態與儲存 owner | `ManagedSession.approvals`（記憶體，隨子程序消失）；mailbox `approval-request`／`approval-resolved`（記憶體）；`Session.consents`（SQLite `sessions`）；audit 表 |
| 現有測試 | `gateway_loop.rs:903`、`:957`、`:1101`、`:1133`、`:1651`；`consent_one_shot_loop.rs:212-531`；`runtime_loop.rs:179`、`:293`；`consent_e2e.rs:139-202`；`api_e2e.rs:167`、`:2229`；`e2e/work-delegate.spec.ts:327`、`app.spec.ts:97`；`scripts/v03-cli-e2e.sh:316-321` |
| 證據 commit | `78dcda1` |
| 本次結果 | `implemented-and-connected`；`static-inspection`＋`unit`＋`fixture`＋`integration`＋`browser`。**E0-10＝correctly-blocked**（唯讀阻擋、撤銷、TTL 過期、續開不放寬、agent token 全部確定性阻擋；證據 `e2e-runs/R4/{a-codex-deny,b-claude,b-codex,d-revoke,d-resume,d-resume2,e-ttl,f-agent-token}/`），但**拒絕語意帶 product-failed**：`ApprovalDecision::Deny => "reject"`（`codex.rs:643-644`）不在 0.153.4 列舉內（**D4**） |
| 限制與下一動作 | 待核可請求不持久化（重啟即失）；Claude Code session 沒有核可流程（設計如此）。下一動作：把 `"reject"` 改為 `"decline"`（K-06）；評估待核可請求持久化 |

#### A-07 取消：interrupt vs close vs 程序樹終止

| 欄位 | 內容 |
|---|---|
| 使用者需求 | interrupt（中斷目前 turn）vs close（關閉 session）；process group SIGTERM→SIGKILL；孤兒子程序在重啟後回收 |
| 實作 owner | `crates/interaction-runtime/src/{gateway.rs,agents.rs}` ＋ `crates/interaction-agent-gateway/src/{process.rs,claude.rs,codex.rs,codex_exec.rs}` |
| 正式入口及呼叫路徑 | interrupt：`AiPage.tsx:664` → `routes.rs:2040-2049`（`interrupt_principal_allowed`）／Tauri `lib.rs:1311`／CLI → `gateway_interrupt`（`gateway.rs:995-1013`）→ `codex.rs:662-681` `turn/interrupt`（寫不進去退回 SIGINT）；`claude.rs:354-357` 對 pgid 送 SIGINT；`codex_exec.rs:253-259` SIGINT＋cancel 旗標。close：`AiPage.tsx:680` → `routes.rs:1548` → `close_agent_session`（`agents.rs:1265-1394`）→ `gateway_spawn_kill`（`gateway.rs:1016-1032`：鎖外 `terminate(2000)` 再有界取鎖 kill）。孤兒：`agents.rs:1569 record_gateway_pgid` → `:1620-1670 reap_recorded_gateway_pgids`（restore 與 shutdown 各一次，以 `pid==pgid`＋cmd 驗證） |
| 資料與協定契約 | CLAUDE.md（長時工作必須有 TTL／cancel；子程序不得跨重啟存活）；`process.rs:1-2`；`AiPage.tsx:681-683` UI 只敢說「已要求終止子程序」 |
| 狀態與儲存 owner | `ManagedSession`（記憶體）、`ProcessGroup` pgid、meta `gateway_pgids`、`AgentSessionRecord.state`／`closed_at` |
| 現有測試 | `gateway_loop.rs:1772`、`:1850`、`:624`、`:1286`、`:2095`；`agents_loop.rs:524`、`:862`、`:491`；`process.rs` unit；`api_e2e.rs:2229`；`e2e/work-delegate.spec.ts:159/201/226`；`scripts/v03-cli-e2e.sh:277-279`；`scripts/tests/tauri-work-cancel.py` |
| 證據 commit | `78dcda1` |
| 本次結果 | `implemented-and-connected`（close／估停／程序樹終止面）；`static-inspection`＋`unit`＋`fixture`＋`integration`＋`browser`＋`native-desktop`（**§4 A-X1 裁定新增**，同 A-05）。**E0-03＝product-failed**：真 claude interrupt 5/5 重現為 `failed`（`e2e-runs/R1/claude-interrupt/{result.json,sse.jsonl}`、`R1/b07-claude/result.json`），close(from active)→closed＝completed，estop 全部不變量成立；真 codex interrupt→cancelled 3/3（`e2e-runs/R1/codex-interrupt/sse.jsonl`）。缺陷 **D1／D2／D3／D8**。interrupt 的分類正確性見 B-07（已改判 confirmed-defect） |
| 限制與下一動作 | Windows 無程序樹終止／孤兒回收；pgid 記錄 best-effort；close 不等待終止完成（UI 文案誠實）。下一動作：比照 `codex_exec.rs:318-329` 讓 claude 記 `interrupt_requested` 並發 `TaskCancelled`；close 投影不得把終局狀態壓成 closed（D2）；record 補 failure reason（D3） |

#### A-07a Emergency Stop

| 欄位 | 內容 |
|---|---|
| 使用者需求 | 任一入口（HTTP／CLI／Tauri／tray／估停檔）→ 停感測、停動器、取消 receipts、殺 agent session、撤銷 consent；latch 跨重啟不自動恢復；只有人類可解除 |
| 實作 owner | `runtime.rs`（emergency_stop／clear）＋ `agents.rs`（estop_agent_sessions）＋ `crates/interaction-api` ＋ `src-tauri/src/{lib.rs,tray.rs,host_safety.rs}` |
| 正式入口及呼叫路徑 | `routes.rs:995-1008`（human 與 legacy agent 可按，adapter token 403）／`:1010`（clear，human-only）；tool-call `routes.rs:755`；CLI；Tauri `lib.rs:923-946`；tray `tray.rs:39/103/138`（直達 Backend，不經 WebView）；估停檔 `state/emergency-stop.requested`（`runtime.rs:2431-2436` 每 tick 讀）。實作 `runtime.rs:1116-1251`（並行停感測＋停動器 2 s 有界＋未確認者列 unconfirmed＋`estop_agent_sessions` `agents.rs:1394-1428`＋撤銷全部 consent）。重啟：`runtime.rs:298`／`:431`／`:496-503` 只讀不清 |
| 資料與協定契約 | CLAUDE.md 不變量（AI 不可解除；重啟不自動恢復；dispatched≠confirmed）；`docs/aip/threat-model.md`；`tray.rs:1-3` |
| 狀態與儲存 owner | `Runtime.estop` AtomicBool＋SQLite meta `estop_engaged`；receipts 表；`sessions.consents`；`state/emergency-stop.requested`；Character Session truth |
| 現有測試 | `estop_parallel.rs:53-366`；`agents_loop.rs:428`、`:1068`；`gateway_loop.rs:1932`、`:2095`；`runtime_loop.rs:293`、`:857`；`character_session_loop.rs:957`、`:1858`；`character_loop.rs:1957`；`api_e2e.rs:686`、`:167`；`tray.rs:265-295`；`e2e/estop.spec.ts:37-192` 等；`scripts/v03-cli-e2e.sh:674-684`；`scripts/tauri-ax-walkthrough.sh` |
| 證據 commit | `78dcda1` |
| 本次結果 | `implemented-and-connected`；`static-inspection`＋`unit`＋`fixture`＋`simulator`＋`integration`＋`browser`＋`native-desktop`。**E0-03 的 estop 段＝completed**（全部不變量成立，`e2e-runs/R1/a07-claude/result.json`） |
| 限制與下一動作 | 真硬體 estop 只有 serial pty 模擬器；動器未回 ack 只能記 unconfirmed；原生 tray／人類解除的真人操作為 needs-environment。**D12**：受限 agent token 按 estop 會把 claimed-completed 轉成 cancelled，之後人類再也無法 verify（`api lib.rs:454-461`＋`agents.rs:1120-1150`）。下一動作：裁定 estop 是否應保留可驗證的 claim |

#### A-08 備份／還原

| 欄位 | 內容 |
|---|---|
| 使用者需求 | 記憶匯出（`/v1/memory/export`）、還原（`BackupSection`）、角色設定匯出／匯入、是否有整體狀態備份 |
| 實作 owner | `crates/interaction-runtime/src/memory.rs` ＋ `pages/BackupSection.tsx` ＋ `companion/settingsTransfer.ts` ＋ `src-tauri/supervisor.rs` |
| 正式入口及呼叫路徑 | 匯出：`BackupSection.tsx:118-140` → `routes.rs:2135`／Tauri `lib.rs:1361` → `memory.rs:192-239`（只 `memory_items`、`EXPORT_MAX_ITEMS`、included／notIncluded 明列）；CLI `memory export`。**還原沒有後端入口**：`api lib.rs:191-204` 沒有 `/v1/memory/import`，CLI 也沒有 `MemoryAction::Import`；`BackupSection.tsx:56-100` 在前端解析 JSON（≤5 MiB、≤1000 筆）逐筆呼叫 `memoryCreate`（新 ID、不信任檔內身分／時間／狀態），非原子。角色設定：`settingsTransfer.ts:57`／`:110` → `desktop.prefsPatch` |
| 資料與協定契約 | `docs/DESKTOP-GUIDE.md:362`（明說不是完整備份）；`BackupSection.tsx:1-16` 誠實原則註解；`memory.rs:216-233` scope 宣告 |
| 狀態與儲存 owner | SQLite `memory_items`；`state/desktop.json` |
| 現有測試 | `memory_loop.rs:467`、`:506`、`:526`；`src/test/backupSection.test.tsx:38-120`（7 個 `it()`）；`e2e/app.spec.ts:230`；`scripts/tests/tauri-settings-walkthrough.py:2-151`（native，不在 CI）；`scripts/v03-cli-e2e.sh:282-295` |
| 證據 commit | `78dcda1` |
| 本次結果 | `implemented-partial`；`static-inspection`＋`unit`＋`browser`＋`native-desktop`（懷疑者維持原判與層級）。**E0-11＝completed（API／CLI 範圍）＋correctly-blocked（壞資料 400）＋harness-failed（native 匯入）**：匯出驗到（`~/Downloads` 404 bytes、sha256 `f4a3171…`），匯入與四個阻擋案例全部未驗證，因為 AX `choosefile` 從不驗證導航（**D18**），連帶使 2026-09-06 那次綠燈可能是假陽性。證據 `e2e-runs/R5/E0-11-*`、`e2e-runs/R7/settings/{result.json,failed-ax.txt}` |
| 限制與下一動作 | 無整體備份／還原（SQLite、config YAML、tokens、character-session.json 都不在任何匯出內）；匯出不落檔；還原非原子且只有記憶層。下一動作：加 Runtime 端 export/import bundle＋Tauri save dialog，或把 UI 文案改成「匯出／還原記憶」；先修 `scripts/lib/tauri-ax.applescript` 的 choosefile 斷言 |

#### A-08a 持久化與重啟恢復

| 欄位 | 內容 |
|---|---|
| 使用者需求 | SQLite（WAL）、config YAML、desktop.json、character-session.json；重啟後 open receipts→uncertain、open agent session→expired＋unknown 事件、estop 保留、pause／consent／recipe 狀態保留 |
| 實作 owner | `crates/interaction-storage` ＋ `crates/interaction-runtime/src/{runtime.rs,agents.rs,config.rs,sensor_journal.rs,character_session.rs}` ＋ `src-tauri/src/supervisor.rs` |
| 正式入口及呼叫路徑 | `Runtime::start`（`runtime.rs:266-520`）：InstanceLock → `Store::open`（WAL）→ clean_shutdown 判斷 → open receipts 標 Uncertain（`:280-296`）→ estop meta → estop 補投 Character Session（`:496-503`）→ `restore_agent_sessions`（`agents.rs:1479-1538`：reap pgids；open→Expired＋`detail "runtime restarted"`；發 taxonomy `unknown`）。shutdown（`runtime.rs:536-581`）依序停感測→persist character session→open receipts→Cancelled→actuator estop→reap→journal flush→`clean_shutdown=true`→釋放 lock。桌面 `supervisor.rs:395-470`（tmp+fsync+rename，保留 0600） |
| 資料與協定契約 | `docs/aip/character-session.md`（快照格式／遷移／隔離）；`runtime.rs:282-292`（crash vs restart）；`agents.rs:1490-1492`（Open sessions do NOT survive a restart） |
| 狀態與儲存 owner | `state/interaction.db`（receipts／plans／sessions／observations／audit／meta／providers／agent_sessions／memory_items／assets／knowledge_*／character_adapters）；`config/*`；`state/{desktop.json,character-session.json,api-token,runtime.lock}` |
| 現有測試 | `runtime_loop.rs:422`、`:772`；`human_layer.rs:123`、`:162`；`consent_one_shot_loop.rs:246`；`proactive_loop.rs:303`；`providers_loop.rs:490`／`:730`；`agents_loop.rs:491`、`:812`（斷言 `record.state==Expired` 且事件序列＝`["unknown"]`）；`character_session_loop.rs:1331`、`:1394`、`:1858`；`supervisor.rs:475-590`；`crates/interaction-storage/src/lib.rs` unit |
| 證據 commit | `78dcda1` |
| 本次結果 | `implemented-and-connected`；`static-inspection`＋`unit`＋`fixture`＋`integration`＋`native-desktop`。**懷疑者 refuted=true**：原列 evidence 寫「SQLite 無 schema 版本遷移框架跡象」是事實錯誤——`crates/interaction-storage/src/lib.rs:18` `CURRENT_SCHEMA=8`、`:76-282` 以 `PRAGMA user_version` 逐階段 migrate（本文件已現場複核 grep 命中）。狀態不變，見 §3。**E0-04＝completed**（`expired`＋detail 記錄回收 pgid、SIGKILL 後子程序 ppid=1、重啟後 children=[]、send 409／interrupt 404／renew 409；`real-agent-e2e-prior/restart-claude-kill/`、`restart-codex-kill/result.json`）。E0-01 的重啟保留亦 completed |
| 限制與下一動作 | 重啟後語意在 record（expired）與事件（unknown）之間不一致；**mailbox 不持久化**，重啟後 claimed-completed 的工作沒有任何成果可看卻仍可按「標記為已驗證」（§4 A-X6 裁定）；config YAML 目錄沒有搬遷工具（SQLite 側已有版本化 migration）。下一動作：統一重啟後的對外狀態；把 from-session 信箱至少 result／approval-* 持久化 |

### 2.B 維度 B — 人類與 Agent 溝通（10 列）

#### B-01 新任務

| 欄位 | 內容 |
|---|---|
| 使用者需求 | 人類建立 Agent 工作階段並把任務文字送進真實 agent（create payload 欄位、task 如何變成 prompt、model／domains 是否可指定） |
| 實作 owner | `interaction-runtime::agents`（create_agent_session／mailbox_send）＋ `::gateway`（gateway_attach／gateway_deliver）＋ connectors；`pages/work/TaskComposer.tsx` |
| 正式入口及呼叫路徑 | `POST /v1/agent-sessions`（`api lib.rs:217` → `routes.rs:1389`）→ `POST …/messages kind=task`（`lib.rs:227-230` → `routes.rs:1500`）；Tauri `lib.rs:1573`＋`:1597`；CLI `commands.rs:915-940`＋`:1003`；桌面 `TaskComposer.start`（`:460-495`）→ `buildSessionCreateInput`（`:166-178`） |
| 資料與協定契約 | `CreateAgentSession`（`agents.rs:247-278`）：providerId?／agentId／label?／ttlMinutes?／dataScope[]／toolScope[]／consentScope[]／allowWrite／maxCost?／maxMessages?／delegation?／workdir?／resumeProviderSessionId?——**沒有 task、沒有 model**。任務文字走 `mailbox_send(kind="task")`；kind==task 時 runtime 強制附 contextBundle（`agents.rs:852-886`）；`gateway_deliver` 組固定 JSON prompt `{task, contextBundle, runtimeRules[]}`（`gateway.rs:726-736`） |
| 狀態與儲存 owner | `AgentSessionEntry`（記憶體）＋ SQLite `agent_sessions`；`GatewayManager.sessions`（子程序 handle）；contextBundle 收據存在 record（`agent.rs:243`） |
| 現有測試 | `gateway_loop.rs:138`（fixture：stdin 收到 contextBundle）；`agents_loop.rs:309`；`workPage.test.tsx:230/271/628`；`e2e/work-delegate.spec.ts:110`；`scripts/v03-cli-e2e.sh:263-277`；`scripts/tests/tauri-work-cancel.py` |
| 證據 commit | `78dcda1` |
| 本次結果 | `implemented-and-connected`；`unit`＋`fixture`＋`integration`＋`browser`＋`native-desktop`。**E0-02＝completed**（真 agent 派工並比對 ground truth，兩平台都逐字正確） |
| 限制與下一動作 | model 無法由任何正式入口指定（型別存在、未接線）；task 與 create 分兩步，第二步失敗會留下一個沒有任務的 `created` session；`domains` 只有 HTTP 直呼帶得進（`TaskComposer.tsx:175` 只送 `workspace:<dir>`，CLI 無旗標）。§4 B-X6 記錄了「同樣缺兩個關鍵環卻判 connected」的尺度爭議。下一動作：決定是否開放 model；桌面／CLI 補 domain 選擇（與 E-10／D-09 同一決策） |

#### B-02 文字串流

| 欄位 | 內容 |
|---|---|
| 使用者需求 | agent 產生的文字如何一路到 SSE 與桌面（逐 token／逐訊息？事件名？Last-Event-ID？） |
| 實作 owner | connector parser ＋ `gateway.rs spawn_gateway_pump` ＋ `interaction-api::sse` ＋ 桌面 `transport.ts`／`AiPage.tsx` |
| 正式入口及呼叫路徑 | connector stdout → `GatewayEvent::TaskProgress{text}`（`gateway lib.rs:85-88`）→ `gateway.rs:380-411` → `report_agent_session("progress")`（`agents.rs:1174-1250`）→ `events.emit(agent.session.state)`（`agents.rs:1098-1105`）→ SSE `GET /v1/events`（`sse.rs:56-103`）→ `transport.ts runStream`（`:415-491`）；卡片另以 5 s 輪詢 from-session 信箱（`AiPage.tsx:153,528-537`） |
| 資料與協定契約 | 逐**訊息**、非逐 token：claude 只取 assistant text 區塊（`claude.rs:410-421`，2000 字上限，啟動參數無 `--include-partial-messages`）；codex `item/agentMessage/delta` 明確丟棄（`codex.rs:579`）。SSE `agent.session.state` payload 只有 `{agentSessionId, agentId, state}`（`agents.rs:1098-1105`），**不含文字**；文字只落 `inferences.report.text` 的觀察儲存 |
| 狀態與儲存 owner | 文字：觀察儲存（`agent.session` receptor，confidence 0.5）；狀態：`AgentSessionRecord.state`；事件：EventBus ring（1024）；桌面 `transport.ts stream.buffer`（500） |
| 現有測試 | `claude.rs` unit（parser）；`codex.rs:753`（delta 靜默）；`gateway_loop.rs:703`；`sse.rs:112-191`；`crates/interaction-events/src/lib.rs:112`；`e2e/home-state.spec.ts:94`、`work-delegate.spec.ts:110`（只驗「處理中」標籤，不驗文字） |
| 證據 commit | `78dcda1` |
| 本次結果 | `implemented-partial`；`unit`＋`fixture`＋`integration`＋`browser`。E0-02 的 `sse.jsonl`（`e2e-runs/R8/claude/sse.jsonl`）確認事件流可用但不帶文字。缺陷 **D10**（SSE 混用 gateway 階段名 fetched／working 與 record 狀態，同一時刻 GET 說 created、SSE 說 fetched） |
| 限制與下一動作 | 串流文字在**桌面與 SSE** 上沒有可見面（§4 B-X4 裁定：`POST /v1/observations/query` 與 CLI `observe` 是正式入口，看得到；措辭需限定為「桌面與 SSE 看不到」）。SSE Lagged 只 warn 並丟棄（`sse.rs:90-93`），客戶端不知情。下一動作：把 TaskProgress 寫進 from-session 信箱（`kind=progress` 標籤與 `messageSummary` 已存在），或在事件加 bounded text；SSE 客戶端加跳號偵測 |

#### B-03 工作進度（工具／產物／token）

| 欄位 | 內容 |
|---|---|
| 使用者需求 | 工具開始／完成、產物、token 用量如何投影給人類 |
| 實作 owner | connectors ＋ `gateway.rs` pump ＋ `statusProjection/workState.ts` |
| 正式入口及呼叫路徑 | `claude.rs:424-432 tool_use → ToolStarted`；`codex.rs:464-508 item/started、item/completed`；`codex_exec.rs:440-461 ArtifactProduced` → pump `gateway.rs:456-484` progress → `report_agent_session` → 觀察儲存；`TokenUsage` → `gateway.rs:486-509`（本輪已結束則只 ingest）；桌面 `workState.ts:85-90,116-117` 一律顯示「處理中」 |
| 資料與協定契約 | `GatewayEvent::ToolStarted`／`ToolCompleted{name}`／`ArtifactProduced{path?}`／`TokenUsage{total,last}`（`gateway lib.rs:104-130`）。claude 只有 ToolStarted、無 ToolCompleted（`claude.rs:468` 忽略 tool_result）；codex app-server 的 `fileChange` 不帶 path，只有 exec fallback 帶第一個 path |
| 狀態與儲存 owner | 觀察儲存（`inferences.report.tool`／`artifact`／`tokenUsage`）；`record.budget.spent_cost` 只有 claude 有值（`gateway.rs:518-520`） |
| 現有測試 | `claude.rs` unit（tool_use→ToolStarted）；`codex.rs` unit（通知正規化、token usage 不冒充 USD）；`codex_exec.rs:513`；**fixture 不輸出工具事件**，所以 runtime／browser 從未走過工具進度 |
| 證據 commit | `78dcda1` |
| 本次結果 | `implemented-partial`；`unit`＋`fixture`＋`not-run`（§4 B-X9 記錄了「同列同時標 fixture 與 not-run」的語意衝突：投影層在 fixture 與 browser 皆未被觸發，故 not-run 指的是投影路徑）。無專屬 E0-xx |
| 限制與下一動作 | 工具／產物／token 進度在 UI 完全不可見；claude 無工具完成事件；codex app-server 無產物路徑。下一動作：把 tool／artifact／tokenUsage 加進 from-session 信箱或事件 payload，並在 `fake_claude.sh` 加 tool_use 行讓 fixture 覆蓋 |

#### B-04 Agent 提問／人類回答

| 欄位 | 內容 |
|---|---|
| 使用者需求 | agent 阻塞等人類回話（waiting-for-input）由誰產生、人類如何回答、UI 有無待回答入口 |
| 實作 owner | `interaction-core::AgentSessionState` ＋ `GatewayEvent` ＋ `agents.rs` report 路徑 ＋ 桌面 `workState`／inbox |
| 正式入口及呼叫路徑 | 唯一能進入 `WaitingForInput` 的路徑是 `POST /v1/agent-sessions/{id}/report {event:"waiting-for-input"}`（`routes.rs:1414` → `agents.rs:1182`）。HTTP scope：legacy agent token 非 GET 白名單不含 `/report` → 403（`api lib.rs:488-535`）；session capability token 亦 403（`:430-470`）。回答管道只有通用 `mailbox_send(kind=task)` → 新一輪 turn（`AiPage.tsx:715-737`「再交代一句」） |
| 資料與協定契約 | `gateway lib.rs:89-99` 註解明言沒有 connector 產生；`agent.rs:21-30` HONEST GAP 註解；CLI help `main.rs:434` 列出 `kind question`，runtime 無對應處理 |
| 狀態與儲存 owner | `AgentSessionRecord.state` |
| 現有測試 | `agents_loop.rs:681-760`（人工 report 觸發）；`character_loop.rs:371`（waiting-input → ask intent）；`statusProjection.test.tsx:46/103/131`（文案）。**沒有任何測試走真實「agent 問→人答」迴路** |
| 證據 commit | `78dcda1` |
| 本次結果 | `defined-only`；`unit`＋`fixture`。**E0-09＝not-implemented**（自動偵測）。真 agent 的提問會以 turn 結束到達，記成 claimed-completed，人類看到「對方說已完成」而不是「等你回答」（`e2e-runs/R3/{claude-followup,codex-followup}/result.json`；同一輪也拍到 codex 的 staleClaim→active→新 claimId 中間態）。缺陷 **D11** |
| 限制與下一動作 | §4 B-X1 裁定：本列的結論只對 **waiting-for-input** 成立；`waiting-for-consent` 那一支有真實 connector 產生者與 UI（見 A-06／§5.1 的 B-M01）。§4 B-X2 裁定：文件與 scope 相斥的出處是 `crates/interaction-cli/src/main.rs:446` 與 `skills/…/references/api.md:104`，不是 `references/cli.md:127`。下一動作：產品決策——移除 waiting-for-input（含文案與 docs），或實作 connector 級偵測；同步修正 skill 文件 |

#### B-05 非同步接續

| 欄位 | 內容 |
|---|---|
| 使用者需求 | 人類離開再回來（切頁／重載／重連／daemon 重啟）能否找回 agent 狀態、結果與待裁決 |
| 實作 owner | `agents.rs`（persist／restore）＋ events ring ＋ `sse` ＋ 桌面 `transport.ts`／`AiPage.tsx`／activity inbox |
| 正式入口及呼叫路徑 | `GET /v1/agent-sessions`（`routes.rs:1385`）＋`GET /{id}/messages`（`routes.rs:1455`；Tauri `lib.rs:1582` 用 `mailbox_peek`）＋`GET /v1/activity/inbox`（`activity.rs:93-135`）；SSE 重連 `transport.ts runStream`（`:415-491`）以 `/v1/status.startedAt`＋`eventSequence` 決定 Last-Event-ID（`:396-412`）；Tauri 內嵌只轉送即時事件（`src-tauri/lib.rs:3412-3430`） |
| 資料與協定契約 | record 每次變更持久化；**信箱不持久化**（`agents.rs:584`、`:1519` 皆 `VecDeque::new()`）；重啟後 open → Expired＋taxonomy unknown；SSE 伺服端 ring 1024 重放；桌面首連從目前序號開始、cursor 只活在該視窗記憶體（`transport.ts:417`），重載頁面等同首連 |
| 狀態與儲存 owner | SQLite `agent_sessions`；記憶體 mailbox；EventBus ring；桌面 buffer 500 |
| 現有測試 | `agents_loop.rs:491`、`:812`；`gateway_loop.rs:138` §6b；`crates/interaction-events/src/lib.rs:112`；`e2e/work-delegate.spec.ts:201` |
| 證據 commit | `78dcda1` |
| 本次結果 | `implemented-partial`；`unit`＋`fixture`＋`integration`＋`browser`（懷疑者維持）。**E0-04＝completed**：SIGKILL 重啟後 session 為 `expired`、send 409／interrupt 404／renew 409，孤兒被回收 |
| 限制與下一動作 | daemon 存活時可找回狀態＋信箱，**但 close 之後信箱已被清空**（§4 B-X3／B-X7 裁定：`agents.rs:1321` 只保留有 `delivered_at` 的信件，而 from-session 訊息永遠沒有戳記）；daemon 重啟後只剩狀態；SSE 缺漏無告警；沒有「你離開時完成了」的主動通知。下一動作：持久化 from-session 信箱；SSE 跳號偵測；桌面 close 允許輸入 handoff |

#### B-06 執行中補充（turn 進行中送第二則訊息）

| 欄位 | 內容 |
|---|---|
| 使用者需求 | turn 進行中 POST messages 的行為（claude stdin／codex `turn/start`／codex exec Busy）與桌面回饋 |
| 實作 owner | `gateway.rs gateway_deliver` ＋ connectors `send_user_message` ＋ 桌面 `work/delivery.ts` |
| 正式入口及呼叫路徑 | `routes.rs:1500`／Tauri `lib.rs:1597`／CLI `commands.rs:1003`／`AiPage.tsx:715-737` → `mailbox_send`（`agents.rs:852-968`）→ `gateway_deliver`（`gateway.rs:669-800`） |
| 資料與協定契約 | runtime 對 claude／codex app-server **沒有 turn 進行中的閘門**：直接寫 stdin（`claude.rs:323-341`）／送 `turn/start`（`codex.rs:604-622`）；codex exec 一則訊息一個子程序，忙碌回 `GatewayError::Busy`（`codex_exec.rs:131-143`）→ 409「未送達」（`gateway.rs:759-763`）；stdin 5 s 逾時 → Unavailable；stdin 已關 → session 記 failed。桌面六態投影 `work/delivery.ts:1-60` |
| 狀態與儲存 owner | mailbox（記憶體）；`ManagedSession.turn_settled`；`record.human_verified`；桌面 delivery notice |
| 現有測試 | `gateway_loop.rs:1518`、`:1286`、`:782`；`agents_loop.rs:278`；`aiPageResume.test.tsx:141-200`、`workPage.test.tsx:368`；`e2e/work-delegate.spec.ts:110`（首送；無 browser 測試按「再交代一句」） |
| 證據 commit | `78dcda1` |
| 本次結果 | `implemented-partial`；`unit`＋`fixture`＋`browser`（懷疑者維持）。E0-02 與 E0-09 的多輪續談確認同一 session 第二輪可用且 pid 不變（claude pid 33355、codex pid 35908 不變），但那是**前一輪已結束**之後送出，不是 turn 進行中 |
| 限制與下一動作 | 真 `claude -p` 在 turn 中收到第二則 user message 是排隊、合併還是忽略，以及真 codex app-server 對進行中 thread 再送 `turn/start` 的回應，仍未驗。§4 B-X8 記錄了 UI 缺口：`AiPage.tsx:722-741` 的送出鈕沒有 in-flight 保護，連按兩下會產生兩則訊息、兩次 deliver、兩格預算。下一動作：實錄 turn 中第二則訊息的行為；UI 加 in-flight 保護 |

#### B-07 停止生成 vs 取消執行（**confirmed-defect**）

| 欄位 | 內容 |
|---|---|
| 使用者需求 | interrupt（中斷本輪）與 close（終止子程序樹）的實際行為 |
| 實作 owner | `gateway.rs`（gateway_interrupt／gateway_spawn_kill／gateway_sweep）＋ connectors ＋ `process.rs` ＋ `AiPage.tsx` |
| 正式入口及呼叫路徑 | interrupt：`routes.rs:2039`／Tauri `lib.rs:1311`／CLI `commands.rs:903`／`AiPage.tsx:664-671` → `gateway_interrupt`（`gateway.rs:995-1011`）。close：`routes.rs:1548`／`lib.rs:1620`／`commands.rs:1038`／`AiPage.tsx:680-683` → `close_agent_session`（`agents.rs:1265-1330`）→ `gateway_spawn_kill`（`gateway.rs:1013-1030`） |
| 資料與協定契約 | trait 說 interrupt＝「中斷目前 turn（不殺 session）」（`gateway lib.rs:245-246`）、CLI help 同意此說（`crates/interaction-cli/src/main.rs:371-372`）。實作：codex → `turn/interrupt` → `interrupted` → `TaskCancelled`（`codex.rs:529`）→ state `Cancelled`（**終局、非 open**，`agent.rs:48-57`）；**claude → 對整個 process group 送 SIGINT**（`claude.rs:354-356`），`grep -c TaskCancelled claude.rs` ＝ **0**（本文件已現場複核） |
| 狀態與儲存 owner | `AgentSessionRecord.state`；`GatewayManager.sessions`；OS process group（meta `gateway_pgids`） |
| 現有測試 | `gateway_loop.rs:1772`、`:1850`、`:1932`、`:2095`；`process.rs:281/301/324`；`e2e/work-delegate.spec.ts:159/201/226`；`scripts/tests/tauri-work-cancel.py`。**沒有任何測試對 claude session 呼叫 interrupt**（`gateway_interrupt` 在 `gateway_loop.rs` 只出現在 `:1805`、`:1900`，兩處都是 codex） |
| 證據 commit | `78dcda1` |
| 本次結果 | **`confirmed-defect`；原判 `implemented-partial` → 改判 `confirmed-defect`**（理由：懷疑者 `correctedStatus` 給 confirmed-defect，逐點反駁後仍成立——claude.rs 沒有 cancel 旗標、`TaskCancelled` 零命中、`human.rs:139` 讓 programming 任務預設路由 codex 因此所有既有通過測試都沒踩到這條路；且本輪 E0-03 執行期 5/5 重現）。證據層級 `unit`＋`fixture`＋`browser`＋`native-desktop`。**E0-03＝product-failed**：`e2e-runs/R1/claude-interrupt/{result.json,sse.jsonl}`（cancel-issued → state `failed`）、`R1/b07-claude/result.json`（獨立第二次）、`real-agent-e2e-prior/{claude-cancel-1,multi-C-claude-x2,multi-E-claude-codex}/result.json`（另外三次）。缺陷 **D1**（含 D2／D3／D8） |
| 限制與下一動作 | 靜態盤點預測終態為 `unknown`，執行期實際是 `failed`——**以執行期為準**（§4 裁定 X-新1）。codex 路徑雖正確產出 cancelled，但 record 也隨之終局化，「中斷後再交代」不可能（**D8**）。下一動作：在 `ClaudeHandle` 記 `interrupt_requested`，`SessionClosed` 時若已請求中斷則發 `TaskCancelled`（比照 `codex_exec.rs:318-329`）；補 claude hang 模式的 interrupt 測試；close 投影不得覆寫終局狀態（D2）；record 補失敗原因（D3） |

#### B-08 Session 恢復（resume）

| 欄位 | 內容 |
|---|---|
| 使用者需求 | 以 providerSessionId 續開（claude `--resume`／codex `thread/resume`／`exec resume`）、寫入權不繼承、runtime 重啟後續開 |
| 實作 owner | `agents.rs`（check_resume_not_wider／check_resume_same_workdir／resumed_session_record）＋ connectors ＋ CLI `agents resume` ＋ `buildResumeInput` |
| 正式入口及呼叫路徑 | `POST /v1/agent-sessions {resumeProviderSessionId}`（`routes.rs:1389`；`agents.rs:277`、`:454-470`）；CLI `commands.rs:943-1002`（allowWrite:false、上限沿用、workdir 沿用）；桌面「接續上次（唯讀）」（`AiPage.tsx:693-708` → `:118-136`）→ `gateway_attach spec.resume_provider_session`（`gateway.rs:337`）→ `claude.rs:107 --resume`／`codex.rs:315-340 thread/resume` 重送 cwd＋approvalPolicy＋sandbox／`codex_exec.rs:69-90 exec resume` 重鎖 sandbox |
| 資料與協定契約 | 續開不得放寬：資料／工具／同意 scope、intent-only 工具集、ttl、maxCost、maxMessages、同一 resolved_workdir（`agents.rs:105-245`）；找不到先前紀錄 → PolicyBlocked（`:454-470`） |
| 狀態與儲存 owner | `AgentSessionRecord.provider_session_id`／`resolved_workdir`（SQLite）；provider 端 thread 由 claude／codex 自行保存 |
| 現有測試 | `gateway_loop.rs:842`、`:1007`、`:2157`、`:2295`、`:2505`、`:2694`；`agents.rs` `resume_workdir_tests`；`codex.rs` connector unit；`aiPageResume.test.tsx:50-140`。CLI E2E 與 Playwright **無** resume 案例 |
| 證據 commit | `78dcda1` |
| 本次結果 | `implemented-and-connected`；`unit`＋`fixture`（懷疑者維持；repo 內無真 agent 證據）。**E0-02 的續接段＝completed（real-agent）**：`e2e-runs/R8/argv-claude/run.txt` 直接拍到 argv 含 `--resume <psid>`，provider log 續寫同一檔且 prompt 不含暗號；codex `thread/resume` 續寫同一 rollout 且 read-only／untrusted 重新上鎖（`e2e-runs/R8/codex/raw.json`）。缺陷 **D9**（續開授權檢查無稽核）、**D20**（claude 續開的 create 回應 providerSessionId 仍是 null） |
| 限制與下一動作 | 續開一律唯讀，要寫入需重新建立；桌面續開沒有 e2e。下一動作：補 Playwright 續開案例；為 resume guard 加 audit（D9）；record 保存 `resumeProviderSessionId`（D20） |

#### B-09 重複與遲到訊息

| 欄位 | 內容 |
|---|---|
| 使用者需求 | 重送／重複 POST 的 dedupe、close 後遲到事件、approval 重複裁決、SSE 重放去重 |
| 實作 owner | `agents.rs`（mailbox ids／budget）＋ gateway pump／approvals ＋ `sse` ＋ connectors |
| 正式入口及呼叫路徑 | `POST /messages`／`/agent-sessions` **無 idempotency key**（`routes.rs:1389`、`:1500`）；SSE 重放與 live 以 sequence 去重（`sse.rs:67-89`）；pump 對 close 後事件用 `let _ =` 吞掉（`gateway.rs:380-636`）；approval 裁決 in_flight 互斥（`gateway.rs:847-905`） |
| 資料與協定契約 | `message_id = msg-{sid}-{n}` 單調遞增（`agents.rs:917`）；`MAILBOX_CAP` 200 FIFO 淘汰；每則佔訊息預算；`delivered_at` 首次戳記為準；report 對非 open session 回 Conflict；每個 claim 新 claim_id 讓舊驗證失效 |
| 狀態與儲存 owner | mailbox（記憶體）；PendingApproval map；EventBus sequence；桌面 cursor |
| 現有測試 | `gateway_loop.rs:1651`、`:903`、`:957`、`:1286`；`codex.rs` unit（失敗的 approval 寫入保留請求）；`agents_loop.rs:940`、`:49`；events／sse unit。**無測試**：重複 POST 同一任務、close 後遲到的 claimed／failed、SSE lag 後的客戶端行為 |
| 證據 commit | `78dcda1` |
| 本次結果 | `implemented-partial`；`unit`＋`fixture`。缺陷 **D14**（已 claimed-completed 的 session 仍可收新任務並真的執行，但 GET 的 state 全程不反映——E0-09 的 codex 案例拍到 staleClaim→active→新 claimId 的中間態，`e2e-runs/R3/codex-followup/result.json`） |
| 限制與下一動作 | 沒有請求層 idempotency；close 後遲到的 agent 結果無痕跡；SSE 缺漏無告警；信箱 200 則 FIFO 可能丟掉舊 result。下一動作：`POST /messages` 加可選 `clientMessageId`；close 後遲到事件寫一筆 audit；SSE 客戶端跳號偵測 |

#### B-10 claimed／verified 與成果呈現

| 欄位 | 內容 |
|---|---|
| 使用者需求 | agent 聲稱完成 → 只有人類能驗證；UI 文案；成果檔案／summary 如何看 |
| 實作 owner | `agents.rs`（report／verify_agent_session）＋ gateway pump ＋ api scope ＋ 桌面 `AiPage`／`workState` ＋ iOS `CharacterView` |
| 正式入口及呼叫路徑 | claim：connector `TaskClaimedCompleted` → pump 寫 from-session `result{summary,costUsd}`＋report claimed-completed（`gateway.rs:511-545`）→ state ClaimedCompleted（open）＋claim_id（`agents.rs:1214`）＋清 human_verified。verify：`POST /v1/agent-sessions/{id}/verify`（human token 專屬，`routes.rs:1435`；agent／session token 403）／Tauri `lib.rs:1689`／CLI `commands.rs:1057`／`AiPage.tsx:605-616` → `agents.rs:1111-1172` → taxonomy verified ＋ `mobile_present_verified` |
| 資料與協定契約 | claimed ≠ verified：只有 `verify_agent_session` 能寫 `human_verified`，且綁當下 claim_id（`agent.rs:232-257`）；任何新回報／新任務送達都清掉驗證。summary 來源：claude result 文字 ≤4000；codex 最後一則 agentMessage；**exec fallback 無 summary** |
| 狀態與儲存 owner | `AgentSessionRecord.state`／`claim_id`／`human_verified`（SQLite）；from-session mailbox（記憶體）；記憶層 `AgentHandoff`（30 天） |
| 現有測試 | `agents_loop.rs:608`、`:940`、`:1143`；`api_e2e.rs:1238`（真 HTTP）；`gateway_loop.rs:138`；`mobile_loop.rs`（simulator）；`e2e/work-delegate.spec.ts:252`、`:298`、`home-state.spec.ts:94`、`evidence.spec.ts:320`；`workPage.test.tsx:434`；`scripts/v03-cli-e2e.sh:192-205`；iOS `CharacterView.swift:8`、`AIPEnvelope.swift:154-167` |
| 證據 commit | `78dcda1` |
| 本次結果 | `implemented-and-connected`；`unit`＋`fixture`＋`integration`＋`browser`＋`simulator`。**E0-10 的 human verify 段＝completed**：授權寫入後 `hello.txt` 落地、verify 後 humanVerified 綁 claimId、重複 verify 409（`e2e-runs/R4/{c-claude-write,c-codex-write}/`） |
| 限制與下一動作 | 驗證不變量完整且多層測試，但「成果呈現」只有一段文字 summary：無檔案清單、無 diff、codex exec 連 summary 都沒有、產物路徑只進觀察。**D12**：受限 agent token 觸發 estop 會讓 claim 無法再被 verify。下一動作：把 artifact／fileChange 寫進信箱並列出；exec fallback 用最後 agentMessage 當 summary |

### 2.C 維度 C — 多 Agent（10 列）

#### C-01 Provider／Profile／Session 的關係

| 欄位 | 內容 |
|---|---|
| 使用者需求 | `provider.ai-agent.*`、AgentProfile、AgentSessionRecord 與儲存表的關係 |
| 實作 owner | `crates/interaction-agent-gateway` ＋ `crates/interaction-runtime/src/{agents,gateway}.rs` |
| 正式入口及呼叫路徑 | `POST/GET /v1/agent-sessions`（`api lib.rs:216-218` → `routes.rs:1385-1403`）；`GET /v1/agents`、`POST /v1/agents/refresh`（`routes.rs:2007-2013`）；CLI `commands.rs:876-884`、`:911-940`；Tauri `lib.rs:1567-1579`；UI `AiPage.tsx:989-996`、`TaskComposer.tsx:167-177` → `agents.rs:387-661` → `gateway_attach`（`gateway.rs:285-377`） |
| 資料與協定契約 | `AgentKind::provider_id()` 固定 `provider.ai-agent.codex` 與 `provider.ai-agent.claude-code`（`gateway lib.rs:20-37`）；`AgentSessionRecord`（`agent.rs:183-244`）；每個 session 另登記為 `provider.ai-session.<id>`（`agents.rs:555-577`） |
| 狀態與儲存 owner | `Runtime.agent_sessions`（`runtime.rs:105`）＋ `GatewayManager.sessions`（`gateway.rs:107-108`）；SQLite `agent_sessions`／`providers`／`meta.gateway_pgids` |
| 現有測試 | `agents_loop.rs:49`、`:491`；`gateway_loop.rs:138`；`api_e2e.rs:2229`；`scripts/v03-cli-e2e.sh:181-196`、`:261-281`；`e2e/evidence.spec.ts:321-372` |
| 證據 commit | `78dcda1` |
| 本次結果 | `implemented-partial`；`static-inspection`＋`unit`＋`integration`＋`browser`。**E0-02＝completed**（真 agent 建立與派工可用）。缺陷 **D20**（claude 的 providerSessionId 在 create 回應為 null，兩個 connector 投影時機不一致） |
| 限制與下一動作 | `docs/ARCHITECTURE.md:114` 說的三層（Provider → Agent Profile → Agent Session）在程式碼中只有兩層：`grep AgentProfile` 只命中模組註解與該文件，沒有型別、欄位或表；`agents.rs:503-509` 對 `providerId` 完全不驗證；非 gateway 的 agentId 建立出的 session 沒有任何 provider 實體。下一動作：改寫文件或補型別；create 時驗 providerId 與 agentId 一致 |

#### C-02 同種多 Session（claude×2）

| 欄位 | 內容 |
|---|---|
| 使用者需求 | GatewayManager 是否以 session id 隔離子程序、事件、mailbox、workdir |
| 實作 owner | `gateway.rs`（GatewayManager）＋ `crates/interaction-agent-gateway/src/{claude,process}.rs` |
| 正式入口及呼叫路徑 | 同 C-01 的建立入口重複呼叫 → `create_agent_session`（受 `agent_create_lock` 序列化，`agents.rs:394`）→ `gateway_attach` 每次新建 connector 並 `start_session`（`gateway.rs:315-347`）→ 以 runtime session id 插入 `GatewayManager.sessions`（`:362-366`） |
| 資料與協定契約 | `AgentConnector::start_session` 每次回傳獨立 handle（`gateway lib.rs:256-265`）；handle 自帶 process group 與 events receiver |
| 狀態與儲存 owner | `GatewayManager.sessions: HashMap<session id, ManagedSession>`；每個 `AgentSessionEntry` 各自的 mailbox；每 session 一把記憶體 capability token（`agents.rs:284-319`） |
| 現有測試 | `gateway_loop.rs:2095-2155`（兩個 fake_claude `hang` session 並存、各自 pid、estop 並行終止）；`e2e/evidence.spec.ts:336-372`（同一 daemon 同時開 2 codex＋2 claude fixture session）；`api_e2e.rs:2229-2360`（token 擁有權互不可跨） |
| 證據 commit | `78dcda1` |
| 本次結果 | `implemented-and-connected`；`static-inspection`＋`integration`＋`browser`。**E0-06＝completed（real-agent）**：claude×2 併發，0 筆 SSE sessionId 混淆、無暗號交叉、取消不干擾另一個（`real-agent-e2e-prior/multi-C-claude-x2/{result.json,sse.jsonl}`）——本輪執行期補上了原本只有靜態推論的「投遞隔離」關鍵環 |
| 限制與下一動作 | §4 C-X1／C-X2 記錄了與 C-07 的判準不一致（未指定 workdir 的多個 session 共用 `home/agent-workspaces/no-folder`）；repo 內仍沒有直接斷言「送給 A 的任務不會進 B 的 stdin」的測試。下一動作：把本輪的隔離結果固化成一支 `gateway_loop` 測試；為 no-folder session 建 per-session 子目錄 |

#### C-03 異種多 Session（claude＋codex）

| 欄位 | 內容 |
|---|---|
| 使用者需求 | claude 與 codex 並存 |
| 實作 owner | `gateway.rs` ＋ `{claude,codex,codex_exec}.rs` |
| 正式入口及呼叫路徑 | `create_agent_session` 以 agentId 選 kind（`agents.rs:597`）→ `gateway_attach` 以 kind 從 `connectors()` 挑（`gateway.rs:74-81`、`:315-318`）；UI `AiPage.tsx:887-890`、`TaskComposer.tsx:43,79`（programming 預設 codex、conversation 預設 claude-code，`human.rs:137`）；路由建議 `routes.rs:2049-2060` → `gateway.rs:1144-1194`（確定性、不強制） |
| 資料與協定契約 | `default_connectors()`／`connectors()` 同時提供兩個 connector（`gateway lib.rs:268-273`；可用 `INTERACT_AI_CLAUDE_BIN`／`INTERACT_AI_CODEX_BIN` 覆寫） |
| 狀態與儲存 owner | 同 C-02；GatewayManager 不分 kind，同一張 map |
| 現有測試 | `e2e/evidence.spec.ts:336-372`；`gateway_loop.rs:2650-2690`（序列，非並行）；`e2e/work-delegate.spec.ts:170`、`:263` |
| 證據 commit | `78dcda1` |
| 本次結果 | `implemented-and-connected`；`static-inspection`＋`integration`＋`browser`。**E0-07＝completed（real-agent）**：跨型並行，取消 claude 不影響 codex，codex 照常走完核可循環並回報自己的暗號（`real-agent-e2e-prior/multi-E-claude-codex/{result.json,sse.jsonl}`） |
| 限制與下一動作 | runtime 層仍沒有任何整合測試同時讓 codex 與 claude fixture 並行。§4 C-X9 記錄了可感知的行為：建立被 `agent_create_lock` 序列化且每次 attach 都重新 discover（每個 connector 上限 6 s），使用者感受到的是序列建立＋延遲累加，本輪 E0-02 的真 claude 冷啟動 61–65 s 也與此相關（見 §4 環境阻礙）。下一動作：補並行 fixture 測試；量測 discover 對建立延遲的貢獻 |

#### C-04 Connector 真實支援範圍

| 欄位 | 內容 |
|---|---|
| 使用者需求 | claude stream-json 的 send／approval／interrupt／resume；codex app-server 的 thread/start、turn/start、turn/interrupt、approval、thread/resume；codex exec fallback 限制 |
| 實作 owner | `crates/interaction-agent-gateway/src/{claude,codex,codex_exec,process}.rs` |
| 正式入口及呼叫路徑 | `/messages`（`routes.rs:1500-1516` → `mailbox_send` → `gateway_deliver` → `handle.send_user_message`）；`/approve`（`routes.rs:2023-2032` → `gateway_resolve_approval`）；`/interrupt`（`routes.rs:2039-2048` → `gateway_interrupt`）；resume 走 create；CLI `commands.rs:893-909`、`:941-1002`；Tauri `lib.rs:1297-1314`、`:1597-1618` |
| 資料與協定契約 | `AgentSessionHandle` trait（`gateway lib.rs:234-254`）；`GatewayEvent`（`:77-152`）；`SessionSpec`（`:156-207`） |
| 狀態與儲存 owner | 各 handle 內部狀態；runtime 端 `PendingApproval`（`gateway.rs:83-93`） |
| 現有測試 | `claude.rs` unit 479-639；`codex.rs` unit 715-988；`codex_exec.rs` unit 475-601；`gateway_loop.rs:842`、`:1007`、`:1133`、`:1772`；`e2e/work-delegate.spec.ts:159-224`、`:327` |
| 證據 commit | `78dcda1` |
| 本次結果 | `implemented-partial`；`static-inspection`＋`unit`＋`integration`＋`browser`。**E0-02／E0-03／E0-10 提供了真 connector 證據**：claude send／resume 正確；codex thread/start、turn/start、turn/interrupt、thread/resume 正確；**claude interrupt 是缺陷（D1）**、**codex deny 值不合法（D4）**、**codex 無 MCP/plugin 封鎖（D5）**、**codex writable_roots 併入使用者全域設定（D6）**、**ServerRequest 一律當核可（D7）** |
| 限制與下一動作 | claude 無 approval 管道（設計如此）、model／max-turns 未接線；codex schema 註解仍鎖 0.149.1 而本機是 0.153.4。下一動作：見 K-06 的四項處置；為 claude interrupt 補 fixture 測試並定義預期狀態 |

#### C-05 開啟 Session 與實際並行

| 欄位 | 內容 |
|---|---|
| 使用者需求 | 並行上限、`open_agent_sessions` 用途、policy 限制、同時 turn 的排程 |
| 實作 owner | `agents.rs`（上限）＋ `crates/interaction-core/src/agent.rs`（DelegationLimits） |
| 正式入口及呼叫路徑 | `create_agent_session`（`agents.rs:469-501`）讀 `policy().delegation`；`PATCH /v1/policy`（`api lib.rs:298-299` → `runtime.rs:701-716` merge-patch）可改數值；`GET /v1/status` 的 `agentSessions` 計數（`runtime.rs:631`） |
| 資料與協定契約 | `DelegationLimits` 預設 `max_depth 3`／`max_sessions 8`／`max_messages_per_session 200`／`max_parallel 4`（`agent.rs:121-143`）；`PolicyConfig.delegation`（`policy.rs:101-103`） |
| 狀態與儲存 owner | `Runtime.policy_config`（持久化 `config_service.save_policy`）；open 計數即時由 map 計算（`agents.rs:375-383`） |
| 現有測試 | `agents_loop.rs:228-276`（第 9 個 open session 被拒）、`:581-606`；`gateway_loop.rs:2095`；`agent.rs` unit 388-410 |
| 證據 commit | `78dcda1` |
| 本次結果 | `implemented-partial`；`static-inspection`＋`unit`＋`integration`。**E0-06 的併發上限段＝needs-environment**（原判 not-implemented → 改判 needs-environment，理由：`agent.rs:124-146` 與 `agents.rs:490/496` 確實實作，只是本輪最多跑 2 個並行 session） |
| 限制與下一動作 | 「並行」只有 open-session 計數，沒有執行中 turn 的並行上限、沒有 per-provider 上限、沒有排程；`PATCH /v1/policy` 對 delegation 數值不驗證（可設 0 或任意大）。§4 C-X4 裁定：`check_delegation`（`agent.rs:171-176`）仍會強制 `max_sessions`，C-05 evidence 的括號說法為準。下一動作：為 delegation 上限加 PATCH 驗證；定義同 session 第二個 turn 的語意 |

#### C-06 委派、深度、allowlist、訊息與預算限制

| 欄位 | 內容 |
|---|---|
| 使用者需求 | delegation depth、tool allowlist、max messages、lease TTL、cost ceiling 在哪確定性強制 |
| 實作 owner | `agents.rs`／`gateway.rs` ＋ `agent.rs`（check_delegation） |
| 正式入口及呼叫路徑 | `create_agent_session`（`agents.rs:412-438` write 前提、`:469-501` delegation、`:504` TTL、`:526-541` budget）；`mailbox_send`（`:906-912` 訊息預算）；`gateway_deliver`（`gateway.rs:699-714` 成本預算）；`renew`（`agents.rs:826-848`）；watchdog `gateway_sweep`（`gateway.rs:1034-1122`）；`DelegateActuator`（`agents.rs:1746-1889`） |
| 資料與協定契約 | `DelegationLimits`／`DelegationEnvelope`／`check_delegation`（`agent.rs:103-178`）；`SessionBudget`（`:84-101`）；`CapabilityLease`（`:60-81`） |
| 狀態與儲存 owner | `AgentSessionRecord.budget`／`lease`（記憶體＋SQLite）；`policy_config.delegation` |
| 現有測試 | `agent.rs:388-410`；`agents_loop.rs:228`、`:278`、`:562`、`:390`；`gateway_loop.rs:2295`、`:2157`、`:2694`、`:2416`；`scripts/v03-cli-e2e.sh:182-190` |
| 證據 commit | `78dcda1` |
| 本次結果 | `implemented-partial`；`static-inspection`＋`unit`＋`integration`。**E0-10＝correctly-blocked**（TTL 過期、續開不放寬、agent token 阻擋皆確定性成立） |
| 限制與下一動作 | `DelegationLimits.provider_allowlist`（`agent.rs:129-130`）在整個 crates 沒有任何讀取點 → **defined-only**；深度檢查只在 payload 帶 envelope 時才做，直接呼叫 HTTP 的呼叫端可自填 `hop_count:0`；非 gateway session 的 maxCost 只存不扣；`docs/MAINTAINERS-MAP.md:100` 說「限界由 `crates/interaction-policy` 確定性強制」與程式不符（policy crate 無 agent／delegation 程式碼）。缺陷 **D6**（codex allowWrite 的實際範圍超出 session 授權）。下一動作：修 MAINTAINERS-MAP；實作或移除 provider_allowlist；codex `thread/start` 送 `writable_roots` |

#### C-07 工作目錄隔離

| 欄位 | 內容 |
|---|---|
| 使用者需求 | workdir 驗證規則（拒絕 `INTERACT_AI_HOME`、需存在、是否要求各 session 不同目錄） |
| 實作 owner | `gateway.rs`（resolve_gateway_workdir）＋ `agents.rs`（check_resume_same_workdir） |
| 正式入口及呼叫路徑 | `create_agent_session` → `gateway_attach` → `resolve_gateway_workdir`（`gateway.rs:240-281`）；resume 比對 `agents.rs:204-236`；UI `AiPage.tsx:868-901`、`TaskComposer.tsx:158-177`；CLI `--workdir`（`main.rs:391,428`） |
| 資料與協定契約 | `AgentSessionRecord.resolved_workdir`（`agent.rs:219-227`）：實際掛載的正規化絕對路徑，續開唯一可比對的事實 |
| 狀態與儲存 owner | `resolved_workdir`（`agents.rs:624-626`）；檔案系統本身 |
| 現有測試 | `gateway_loop.rs:2258`、`:2592`、`:2620`、`:2505`；`agents.rs` `resume_workdir_tests`；`e2e/evidence.spec.ts:331-336`（每 session 各自 tmp 目錄＝測試衛生，非 runtime 強制） |
| 證據 commit | `78dcda1` |
| 本次結果 | `implemented-partial`；`static-inspection`＋`unit`＋`integration`＋`browser`。**E0-06／E0-07 的暗號隔離間接支持**（每個 summary 只含自己 workdir 的暗號） |
| 限制與下一動作 | 無 per-session 唯一目錄、無 worktree；未指定目錄的多個 session 共用 `home/agent-workspaces/no-folder`；兩個 write-enabled session 可同時掛同一目錄。安全邊界只擋 `state/`（`gateway.rs:268-277`），不擋 home 其他子目錄。下一動作：為 no-folder／proactive 建 per-session 子目錄；或加「同一目錄只允許一個 write-enabled open session」 |

#### C-08 單一 Session 取消是否影響其他工作

| 欄位 | 內容 |
|---|---|
| 使用者需求 | close／kill 只殺該 process group？estop 全殺？ |
| 實作 owner | `agents.rs`／`gateway.rs` ＋ `process.rs` |
| 正式入口及呼叫路徑 | `POST …/close`（`routes.rs:1548-1563` → `agents.rs:1266-1381` → `gateway_spawn_kill` `gateway.rs:1013-1029`，只 remove 該 id、只對該 group `terminate(2 s)`）；`POST …/interrupt`（`gateway.rs:995-1010`，只作用該 handle）；`POST /v1/emergency-stop`（`runtime.rs:1116-1201` → `estop_agent_sessions` `agents.rs:1394-1428`，快照全部 open session、各 2 s 有界關閉、join_all 併行） |
| 資料與協定契約 | `AgentSessionHandle::kill`／`interrupt`／`process_group`（`gateway lib.rs:245-251`）；`ProcessGroup::terminate`（`process.rs:143-163`） |
| 狀態與儲存 owner | `GatewayManager.sessions`（remove by id）＋ meta `gateway_pgids` |
| 現有測試 | `gateway_loop.rs:2095-2155`、`:1932`；`agents_loop.rs:428`、`:1068`；`process.rs` unit 280-340；`api_e2e.rs:2340-2360`；`e2e/work-delegate.spec.ts:159-250` |
| 證據 commit | `78dcda1` |
| 本次結果 | `implemented-and-connected`；`static-inspection`＋`unit`＋`integration`＋`browser`。**E0-07＝completed（real-agent）**：取消 claude 不影響 codex 的時間線（`real-agent-e2e-prior/multi-E-claude-codex/`）；E0-06 也確認取消其一不干擾另一個 |
| 限制與下一動作 | pgid 歸屬為 best-effort（`agents.rs:1562-1604`），只影響崩潰重啟後的孤兒清理；非 unix 無 process-group 語意。§4 C-X3 裁定：本列描述的是「取消不外溢」，claude interrupt 的**分類錯誤**歸 B-07／D1，兩者不衝突。下一動作：把本輪的「關閉 A 之後 B 仍活」結果固化成回歸測試 |

#### C-09 達限制時明確拒絕

| 欄位 | 內容 |
|---|---|
| 使用者需求 | 達限制時要有明確錯誤碼／訊息 |
| 實作 owner | `agents.rs`／`gateway.rs`（DomainError）＋ `crates/interaction-api/src/error.rs`（HTTP 映射） |
| 正式入口及呼叫路徑 | HTTP `ApiError::from(DomainError)`（`error.rs:53-71`）→ body `{error:{code,message}}`；CLI 原樣輸出 status＋JSON；Tauri 以 `err_s` 壓成字串（`src-tauri/lib.rs:1577,1613,1628`） |
| 資料與協定契約 | `DomainError::code()`（`core error.rs:76-93`）；HTTP 狀態（`api error.rs:56-66`：PolicyBlocked／ConsentRequired 403、Conflict 409、Validation 400、Unavailable 503、Expired 410、NotFound 404） |
| 狀態與儲存 owner | 無（純錯誤路徑） |
| 現有測試 | `agents_loop.rs:228`、`:278`、`:390`；`gateway_loop.rs:1427`、`:1518`、`:2757`；`api_e2e.rs:2229`；`scripts/v03-cli-e2e.sh:186-190`、`:673-678` |
| 證據 commit | `78dcda1` |
| 本次結果 | `implemented-and-connected`；`static-inspection`＋`unit`＋`integration`（懷疑者維持層級）。**E0-04 直接觀察到**：重啟後 `send 409／interrupt 404／renew 409`（`real-agent-e2e-prior/restart-claude-kill/result.json`）；E0-10 觀察到 `重複 verify 409` |
| 限制與下一動作 | Tauri 路徑把 code 壓掉只剩 message，桌面 UI 無法依 code 分流（§4 C-X6 記錄了「以入口為準則應判 partial」的爭議，本文件維持 connected 並在此明列缺口）；`PATCH /v1/policy` 對 delegation 不驗證；`agents.rs:503-508` 對 providerId 與未知 agentId 靜默接受（§4 C-X8）。下一動作：Tauri 回傳結構化 `{code,message}`；policy PATCH 加範圍驗證 |

#### C-10 不誤續接另一個 Session

| 欄位 | 內容 |
|---|---|
| 使用者需求 | resume 用 providerSessionId 是否驗 agentId／owner |
| 實作 owner | `agents.rs`（resumed_session_record／check_resume_not_wider）＋ `api lib.rs`（principal 守門） |
| 正式入口及呼叫路徑 | `POST /v1/agent-sessions{resumeProviderSessionId}`（`routes.rs:1389-1395`；只有 human token 可 POST）→ `agents.rs:453-468` → `resumed_session_record`（`:352-365`）→ `check_resume_not_wider`（`:105-178`）→ `gateway_attach`（`gateway.rs:337`）；CLI `commands.rs:941-1002`；UI `AiPage.tsx:122-134` |
| 資料與協定契約 | `AgentSessionRecord.provider_session_id`「不是 runtime 的 session 身分」（`agent.rs:215-218`）；resume「不得放寬任何 scope」（`agents.rs:273-277`、`:441-452`） |
| 狀態與儲存 owner | `agent_sessions` 記憶體 map（含 restore 回載的已結束紀錄，保留 200 筆／30 天） |
| 現有測試 | `gateway_loop.rs:842`、`:1007`、`:2157`、`:2295`、`:2505`；`agents.rs` `resume_workdir_tests`；`aiPageResume.test.tsx` |
| 證據 commit | `78dcda1` |
| 本次結果 | `implemented-partial`；`static-inspection`＋`unit`＋`integration`（懷疑者維持層級）。**E0-02／E0-10 的續開段確認 scope 不放寬**（`e2e-runs/R4/{d-resume,d-resume2}/`）。缺陷 **D9** |
| 限制與下一動作 | `resumed_session_record` 只以 `provider_session_id` 相等取最近一筆，**不看 state 也不看 agent_id**；`check_resume_not_wider` 沒有 `input.agent_id == original.agent_id` 比對，因此人類可用 codex thread id 搭配 `agentId=claude-code` 建立一個註定失敗的 session；也沒有「同一 provider thread 不得同時被兩個 open session 掛載」的檢查。§4 C-X7 裁定：`gateway_loop.rs:1044-1095` 這支既有的通過測試就示範了「原 session 仍 open 時再續開同一 thread」，所以是**已被既有測試示範的缺口**，不是「未測」。下一動作：加 `agent_id` 一致性檢查與雙重續開檢查，各補一支回歸測試 |

### 2.D 維度 D — 策略與上下文（6 列）

#### D-01 本地互動／直接查詢／Agent 呼叫的分流

| 欄位 | 內容 |
|---|---|
| 使用者需求 | 一個需求由誰決定「本機確定性文案（text.rs）／本機 recipe／交給哪個 Agent」，路由建議是否真的被消費，主動說話五模式如何分流 |
| 實作 owner | `gateway.rs`（agent_route_suggestion）＋ `human.rs`（UiPreferences.agent_routes）＋ `{proactive,runtime,executor,text,orchestrator}.rs`；桌面 `TaskComposer.tsx`／`AiPage.tsx` |
| 正式入口及呼叫路徑 | `GET /v1/agents/routing?kind=`（`routes.rs:2057-2064` → `gateway.rs:1144-1194`）；CLI `agents route`（`main.rs:357-360`）；Tauri `agents_routing`（`src-tauri/lib.rs:1291-1293`）。桌面 `AiPage.tsx:875-878,964-965` 只把 `routing.reason` 當提示文字；`TaskComposer.tsx:78-93,403` 用前端 `DEFAULT_ROUTES`＋`prefs.agentRoutes` 直接決定 agentId，**不呼叫該端點**。主動說話：`runtime.rs:2069-2090` `AiGenerated` → `start_proactive_agent_task`（`proactive.rs:706-770`，只允許 claude-code），否則走本機 `TextSelector::select`（`text.rs:79-125`） |
| 資料與協定契約 | kind→role 對照（code/test/patch/repo-review→programming；knowledge*→knowledge；review-second-opinion→review；chat/docs/…→conversation；其他→None）→ `preferences.agent_routes[role]`（`"none"`＝不交給 Agent），回 `{kind,suggestion,role,reason,candidates,note}`，純建議不強制（`gateway.rs:1142-1143`） |
| 狀態與儲存 owner | `UiPreferences`（SQLite meta）；`ProactiveDialogueState`（meta `proactive_dialogue`）；`GatewayManager.discoveries`（記憶體快照） |
| 現有測試 | `human_layer.rs:186-188`；`proactive.rs:873-1097` unit；`proactive_loop.rs:67-437`（含 `:341` 用 fake_claude proactive 模式）；`text.rs:186-236` unit；`orchestrator.rs:271-372` unit；`regressions-v04.test.tsx:407`；`e2e/general-mode-tasks.spec.ts:1084-1140`（任務 14，不涵蓋路由建議）。`gateway.rs` 無 route_suggestion 單元測試（grep 0 命中） |
| 證據 commit | `78dcda1` |
| 本次結果 | `implemented-partial`；`static-inspection`＋`unit`＋`fixture`＋`integration`（**§4 D-X4 裁定：移除 `browser`**——該列自己註明引用的 e2e 不涵蓋路由建議）。無專屬 E0-xx |
| 限制與下一動作 | 沒有任何 runtime 元件做「本機直接回答 vs 交給 Agent」的分流：工作頁每筆工作都建立 Agent Session；路由建議只是 JSON，桌面工作流程不消費；「模糊任務列出兩者讓人選」只存在於 `candidates` 欄位，桌面沒有對應 UI。下一動作：讓 TaskComposer 消費 `/v1/agents/routing`，或刪掉該端點的「讓人選」宣稱；補 kind→role 對照的單元測試 |

#### D-02 題詞模板與版本

| 欄位 | 內容 |
|---|---|
| 使用者需求 | 送進 Agent 的文字如何組裝、是否有 system prompt／persona／版本號、是否可設定（含 model）、各 connector 用什麼管道遞送 |
| 實作 owner | `gateway.rs`（gateway_deliver 包裝）＋ `proactive.rs`（主動對話 prompt）＋ `{claude,codex,codex_exec}.rs`（遞送管道） |
| 正式入口及呼叫路徑 | `mailbox_send(ToSession, kind=task)`（`agents.rs:855-960`）→ `gateway_deliver`（`gateway.rs:701-735` 組字串）→ claude stdin stream-json user message（`claude.rs:347-362`）／codex `turn/start` input text（`codex.rs:604-617`）／codex exec 把 prompt 放進 **argv**（`codex_exec.rs:69-110,131-168`）。主動對話 prompt 由 `proactive.rs:741-752` 的 `format!` 組成 |
| 資料與協定契約 | 有 contextBundle 時送 pretty JSON `{task, contextBundle, runtimeRules:[3 條固定英文句]}`（`gateway.rs:717-735`）；沒有 bundle 就送原文。**沒有 system prompt、persona、語言、角色名稱注入**；註解明言「This wrapper is not persona-editable」。`grep prompt_version／PROMPT_VERSION／promptVersion` 全 repo 0 命中 |
| 狀態與儲存 owner | 無持久狀態：prompt 每次即時組裝，沒有版本號、沒有設定檔欄位 |
| 現有測試 | `claude.rs:459-490`（旗標，不含 prompt 內容）；`codex_exec.rs:470-510`；`gateway_loop.rs:277`（fixture：fake_claude 記錄的 stdin 含 `contextBundle` 與 `agentId`，是唯一對送出 prompt 形狀的斷言）；`proactive_loop.rs:341`（不檢查 prompt 文字） |
| 證據 commit | `78dcda1` |
| 本次結果 | `implemented-partial`；`static-inspection`＋`unit`＋`fixture`＋`integration`。**E0-02 提供了真 agent 對這份 prompt 的反應證據**（答案逐字正確，代表 bundle＋runtimeRules 的包裝可被真模型解讀）；**E0-05 進一步證明 bundle 被讀到**（summary 含暗號，`e2e-runs/R2/E0-05-delivery-claude/result.json`） |
| 限制與下一動作 | 題詞是硬編碼字串、無版本號、無設定入口；`runtimeRules` 是給模型看的建議文字，不是強制；model 選擇只到 `SessionSpec` 為止（見 K-01）；codex exec fallback 把整份 prompt＋contextBundle 放進 argv，本機其他程序可用 `ps` 讀到（與 **D5** 對 app-server 路徑的 argv 外洩是同一類問題）。下一動作：把包裝抽成常數＋版本欄位並加單元測試釘住形狀；評估 argv 外洩 |

#### D-03 Context Bundle 建立、派送與消費

| 欄位 | 內容 |
|---|---|
| 使用者需求 | 結構、由哪個函式產生、附在哪個訊息、Agent 端怎麼收到、Skill 是否教 Agent 使用、收據是否可見 |
| 實作 owner | `memory.rs`（memory_context_bundle）＋ `agents.rs`（附掛＋收據）＋ `gateway.rs`（派送）＋ `domain_packs.rs`；`MemoryKnowledgePage.tsx BundleSection`；`skills/orchestrate-adaptive-interaction` |
| 正式入口及呼叫路徑 | 產生 `memory.rs:239-380`。自動附掛只在 `direction==ToSession && kind=="task"`（`agents.rs:863-895`），domains 取自 `data_scope` 的 `domain:` 前綴，呼叫端傳入的 contextBundle 會被覆寫；收據 `AgentContextBundleReceipt{bundle_id,message_id,generated_at,content_hash,bundle}` 推入 `record.context_bundles`（`agents.rs:928-942`）。派送 `gateway.rs:717-735`。人類預覽 `POST /v1/memory/context-bundle`（`routes.rs:2146-2163`）、CLI `memory bundle`、Tauri `memory_bundle`、`MemoryKnowledgePage.tsx:1286-1317`（固定 `domains=[]`）。收據讀取只有 `GET /v1/agent-sessions/{id}` 與 CLI `agents show` |
| 資料與協定契約 | Bundle JSON：`{task, agentId, domains, generatedAt, includes[], excluded{…}, truncated, limits{maxItems:24,maxBytes:49152,scanLimit:1000}, note}`（`memory.rs:340-380`）。Agent 端唯一接收管道＝任務訊息文字本身，沒有 env、沒有 API 拉取 |
| 狀態與儲存 owner | SQLite memory 表；domain pack 安裝清單（meta）；收據隨 `AgentSessionRecord` 持久化（上限 32 筆，`agents.rs:51`） |
| 現有測試 | `agents_loop.rs:309-388`（domain 過濾、hash 64 字元、重啟後收據仍在）；`memory_loop.rs:299`、`:422`、`:641`；`gateway_loop.rs:277`；`scripts/v03-cli-e2e.sh:275-276`、`:290-292`。Playwright 無 bundle 斷言 |
| 證據 commit | `78dcda1` |
| 本次結果 | `implemented-and-connected`；`static-inspection`＋`unit`＋`integration`＋`fixture`。**E0-05＝completed（real-agent）**：bundle 送達且被讀（summary 含暗號）；刪除後 `includes=[]` 且 agent 誠實說不知道（`e2e-runs/R2/step{1,2,4,5}-*.json`）。**但本列必須與 D-06 一起讀**：同一條派送路徑會被自身附掛的 bundle 擋死（§4 D-X2 裁定） |
| 限制與下一動作 | Agent 只能從任務文字讀到 bundle，沒有結構化管道；Skill 沒教 Agent 如何解讀 `contextBundle`／`runtimeRules`；桌面不顯示 session 實際送出的收據，只有事前預覽且預覽固定 `domains=[]`（與真實送出不同）。`content_hash` 對含 `generatedAt` 的整份 bundle 取 sha256，同樣內容每次 hash 都不同，撐不起去重比對（§4 D-X9）。下一動作：在 SessionCard 渲染 `contextBundles`；Skill 補一節；hash 改對正規化內容取 |

#### D-04 依任務相關性選取資料的程度

| 欄位 | 內容 |
|---|---|
| 使用者需求 | Context Bundle 是否依 task 內容挑選；Agent 是否有其他依相關性拉取知識的路徑 |
| 實作 owner | `memory.rs`（bundle 選取）＋ `knowledge.rs`（knowledge_search_in_domains、LocalSubwordEmbeddingIndex）＋ `domain_packs.rs`；`routes.rs dispatch_tool` |
| 正式入口及呼叫路徑 | Bundle：`memory_context_bundle`（`memory.rs:239-380`）——`task` 參數只回填到輸出 JSON（`:356`），**不參與過濾**。Pull：session token 呼叫 `POST /v1/tools/interaction.knowledge_search/call`（`api lib.rs:452-454`）→ `routes.rs:754-762` → `knowledge.rs:1255-1313`（FTS bm25＋本機稀疏向量融合，限 domain） |
| 資料與協定契約 | 選取＝確定性、非相關性：domain pack 先入（`domain_packs.rs:351-357`）→ 記憶掃最多 1000 筆 → 依 layer rank → `updated_at desc` → id（`memory.rs:279-296`）→ 逐筆過濾 expired／stale／sensitive／`!visible_to_agent`／Candidate／知識層須命中授權 domain（空 domains＝全部排除，fail-closed）→ 遇第一個放不下就停 |
| 狀態與儲存 owner | 記憶表（SQLite）；知識 FTS 表＋記憶體向量索引（啟動 rebuild） |
| 現有測試 | `memory_loop.rs:299`、`:422`、`:641`；`knowledge_loop.rs`；`api_e2e.rs:301`、`:1081`、`:1184` |
| 證據 commit | `78dcda1` |
| 本次結果 | `implemented-partial`；`static-inspection`＋`integration`＋`unit`。**E0-05 直接觀察到**：同一 session 的 bundle 內容不隨任務文字改變；矛盾記憶會同時進入 bundle（缺陷 **D13**，`e2e-runs/R2/E0-05-supersede/`） |
| 限制與下一動作 | Bundle 完全不看任務內容；24 筆／48 KiB 上限之後任務相關的舊條目會被最新條目擠掉；知識節點不進 bundle，只能由 Agent 主動用工具檢索（但需 session token＋tool_scope，而出貨入口從不授權——見 §5.1 的 E-10／D-09）。桌面預覽文案「這次實際會提供哪些記憶與知識」與實作（只有記憶＋domain pack）不一致。下一動作：把「確定性、非相關性」明寫進 FEATURES／UI 文案；若要相關性，最小改動是對 task 做 FTS 命中加權後再套現有過濾 |

#### D-05 外部內容與指令的信任邊界

| 欄位 | 內容 |
|---|---|
| 使用者需求 | Agent 回傳文字是否被當指令執行、bundle／記憶內容的 prompt injection 防線、Agent 拿到的 token 範圍 |
| 實作 owner | `crates/interaction-api/src/lib.rs`（auth_middleware／agent_request_allowed／session_request_allowed）＋ `routes.rs` ＋ connectors（旗標／沙箱／env）＋ `{gateway,agents,proactive,memory}.rs` |
| 正式入口及呼叫路徑 | Token：`create_agent_session` → `issue_agent_session_capability`（`agents.rs:284-320`，sha256 digest 存記憶體）→ `SessionSpec.session_capability_token` → `process.rs:15-36` 移除 runtime auth env 後只注入 `INTERACT_AI_SESSION_TOKEN`＋`INTERACT_AI_API_URL`。Agent 輸出：`GatewayEvent` → pump → `report_agent_session` 把 payload 放進 `inferences.report`，`actionId` 改名 `claimActionId`；summary 只進 mailbox `result` |
| 資料與協定契約 | Session token 可做：GET `/v1/tools*`、GET 自己的 `/messages`、`POST /v1/tools/*/call`（受 tool_scope）、停止類、`POST` 自己的 `/interrupt`；**不可**建立／續租／關閉 session、report、memory／knowledge 人類端點、verify（`api lib.rs:432-470`） |
| 狀態與儲存 owner | `agent_session_capabilities` 記憶體 map（不落地，重啟即失效）；`api-token`／`api-agent-token`（0600） |
| 現有測試 | `api lib.rs:630-830 auth_scope_tests`；`api_e2e.rs:301`、`:1081`、`:1184`、`:1262`、`:2260`；`claude.rs:459-490`、`:560-585`、`:520-545`；`gateway_loop.rs:2258`、`:2592`、`:2620`、`:2650`、`:2694`；`agents_loop.rs:608`、`:1143`；`memory_loop.rs:89`、`:144`、`:200`；`proactive_loop.rs:341` |
| 證據 commit | `78dcda1` |
| 本次結果 | `implemented-partial`；`static-inspection`＋`unit`＋`integration`＋`fixture`。**E0-10＝correctly-blocked**（受限 agent token 全部確定性阻擋，`e2e-runs/R4/f-agent-token/`；`e2e-runs/R2/step7-agenttoken-*.json` 另有記憶端點的 403 證據）。**但執行期揭露一個 static 沒抓到的破口：D5** —— codex 連接器沒有等價於 claude 的 MCP／plugin 封鎖（`codex.rs:166-169` 只設 `current_dir`，對照 `claude.rs:82-84` 的 `--strict-mcp-config --mcp-config {"mcpServers":{}}`），宣告唯讀的 session 一建立就啟動使用者 `~/.codex` 設定裡的 MCP server 與 helper，且任務全文出現在 world-readable 的 process argv |
| 限制與下一動作 | Bundle／記憶內容的注入防線只有 `runtimeRules` 文字，真正的邊界靠 provider 旗標與 token 範圍；`docs/aip/threat-model.md` 沒有 bundle／agent-output prompt injection 條目。下一動作：**先修 D5**（codex 啟動加 `--strict-config`／`-c mcp_servers={}`，並把 prompt 改走 stdin）；威脅模型補列；補一支以惡意記憶內容跑 fixture、斷言 runtime 側無權限變化的測試 |

#### D-06 容量截斷、壓縮、Session 失效與恢復（**confirmed-defect**）

| 欄位 | 內容 |
|---|---|
| 使用者需求 | bundle 大小上限、mailbox 上限、訊息預算、lease 到期、子程序結束／daemon 重啟時行為、resume 路徑、未送達訊息的處置 |
| 實作 owner | `agents.rs`（MAILBOX_CAP／MAX_BODY_BYTES／lease／restore／check_resume_not_wider）＋ `memory.rs`（BUNDLE_MAX_*）＋ `gateway.rs`（錯誤分支、gateway_sweep）＋ connectors |
| 正式入口及呼叫路徑 | `mailbox_send`（`agents.rs:855-960`）→ `memory_context_bundle` → `body.insert("contextBundle")` → `serde_json::to_vec(&body).len() > MAX_BODY_BYTES(16384)` → `Err(Validation "mailbox body too large")`；HTTP／Tauri／CLI／主動對話都經此。Lease：`expire_if_needed`（`:673-702`）、`renew`（`:830-852`）、`gateway_sweep`。Restart：`restore_agent_sessions`（`:1478-1520`） |
| 資料與協定契約 | `BUNDLE_MAX_ITEMS=24`、`BUNDLE_MAX_BYTES=48*1024`、`BUNDLE_SCAN_LIMIT=1000`（`memory.rs:18-22`）；`MAX_BODY_BYTES=16*1024`（`agents.rs:50`）；`MAILBOX_CAP=200` FIFO；`CONTEXT_BUNDLE_RECEIPT_CAP=32` |
| 狀態與儲存 owner | `AgentSessionEntry`（記憶體 map＋mailbox＋next_message）；record 持久化 SQLite；**mailbox 不持久化**；`gateway_pgids` meta |
| 現有測試 | `memory_loop.rs:422`（**直接呼叫 `memory_context_bundle`，未經 `mailbox_send`**——這正是缺陷沒被發現的原因）；`agents_loop.rs:309`（兩筆極小記憶）、`:278`、`:390`、`:491`；`gateway_loop.rs:624`、`:782`、`:842`、`:1007`。**未覆蓋**：bundle > 16 KB 時 mailbox_send 失敗、MAILBOX_CAP 淘汰、resume 後未送達訊息重送 |
| 證據 commit | `78dcda1` |
| 本次結果 | **`confirmed-defect`**（懷疑者維持）；`static-inspection`＋`unit`＋`integration`＋`fixture`。缺陷路徑：`BUNDLE_MAX_BYTES(48 KiB) > MAX_BODY_BYTES(16 KiB)`，且 bundle 由 runtime 塞進 body **之後**才做 16 KiB 檢查（`agents.rs:886-895`）；記憶項單筆上限 8 KiB（`crates/interaction-core/src/memory.rs:183`），2–3 筆合法的 know-how 就能讓該 session 的每一次 task 派送都回 400，使用者無法用 UI 解決。**E0-04／E0-05 未觸發**（本輪記憶量小），故本列狀態來自靜態證據，執行期為 `not-run` |
| 限制與下一動作 | 其他限制：mailbox 不持久化、重啟即清空（收據仍在）；MAILBOX_CAP 淘汰靜默無 audit；runtime 自己寫的 result／approval 訊息會扣人類的訊息預算；沒有壓縮／摘要，只有截斷；未送達訊息不會在 resume 時重放。**§4 D-X3 裁定**：觸發面比原描述更大——`AgentHandoff`／`TaskMemory`／`WorldKnowledge` 都不在 domain 過濾名單，不需任何 domain 授權就會進 bundle，所以「桌面建立、domains=[]」這個唯一出貨流程同樣可觸發。下一動作：讓 `memory_context_bundle` 接受 `max_bytes` 參數（由 `mailbox_send` 扣掉 task 長度後傳入），或把 contextBundle 排除在 `MAX_BODY_BYTES` 之外；新增回歸測試「3 筆 8 KB 同 domain know-how → mailbox_send 必須成功且 `bundle.truncated=true`」 |

### 2.E 維度 E — 記憶與知識（9 列）

#### E-01 記憶／知識節點／素材／角色記憶的關係

| 欄位 | 內容 |
|---|---|
| 使用者需求 | 四種資料的實際關係（memory.rs 層級、knowledge.rs 節點／邊、assets CAS、角色記憶是否存在） |
| 實作 owner | `crates/interaction-runtime/src/{memory,knowledge,curator}.rs`；`crates/interaction-core/src/{memory,knowledge}.rs`；`apps/interaction-desktop/src/companion/interactionMemory.ts` |
| 正式入口及呼叫路徑 | `/v1/memory*` → `routes.rs:2082-2181` → `memory.rs:36-391`；`/v1/knowledge/*`、`/v1/assets/*` → `routes.rs:2184-2434`；agent 只能經 canonical tools `interaction.knowledge_*`（`routes.rs:772-943`）；CLI `commands.rs:508-720`；Tauri `lib.rs:1317-1563`（embedded 直呼 Runtime）；桌面 `MemoryKnowledgePage.tsx` |
| 資料與協定契約 | 記憶＝`MemoryItem` 10 層（`core memory.rs:17-38`）＋kind fact／inference／preference／know-how／candidate；知識＝`KnowledgeNode`（entity／claim／source）＋12 種 edge relation＋狀態機 candidate→active→stale/disputed→superseded/archived；素材＝SHA-256 CAS write-once；**bundle 只含記憶＋domain pack，不含知識節點** |
| 狀態與儲存 owner | SQLite `memory_items`／`knowledge_nodes`／`knowledge_edges`／`knowledge_fts`／`assets`／`asset_derivatives`／`knowledge_receipts`；素材 blob 在 `<home>/state/assets/<hh>/<hash>`；角色互動記憶在 Tauri `DesktopPrefs`（`supervisor.rs:135,226`） |
| 現有測試 | `memory_loop.rs:62`、`:593`、`:641`；`knowledge_loop.rs:55`、`:82`、`:227`；`curator_loop.rs:342`；`core memory.rs`／`knowledge.rs` unit；`scripts/v03-cli-e2e.sh:282-338`；vitest（api 以 mock 替身） |
| 證據 commit | `78dcda1` |
| 本次結果 | `implemented-partial`；`static-inspection`＋`unit`＋`integration`。**E0-05 提供真 agent 端到端證據**（記憶→bundle→agent 讀到→刪除→誠實說不知道） |
| 限制與下一動作 | 四種資料之間沒有資料庫層關聯（無 FK、無反向指標）；記憶→知識只靠 `evidence.url` 字串（`curator.rs:302-347`），刪掉記憶後 evidence 懸空不會被偵測；`MemoryLayer::CharacterMemory` 是**死 enum**（crates 內沒有任何寫入者），真正的角色互動記憶完全在桌面 prefs，不受 Runtime retention／export／visibility 規則管轄；iOS 端沒有任何記憶／知識入口。§4 E-X10 補充：`state/character-session.json` 是 Runtime 側第二份持久化狀態，無保存期限、無刪除入口，本輪沒有任何一列覆蓋它。下一動作：決定 CharacterMemory 的去留；為 user-correction 的 evidence 加級聯或懸空偵測 |

#### E-02 SQLite 與檔案儲存的用途

| 欄位 | 內容 |
|---|---|
| 使用者需求 | storage crate 表清單、哪些走檔案 |
| 實作 owner | `crates/interaction-storage/src/lib.rs`；`crates/interaction-runtime/src/{config,runtime,knowledge}.rs` |
| 正式入口及呼叫路徑 | `Runtime::start` → `Store::open(paths.db_file())`（`runtime.rs:272-290`）；所有 memory／knowledge／asset 服務經 `self.store` |
| 資料與協定契約 | schema `user_version` 1..8 逐版 migrate（`storage lib.rs:76-283`，`CURRENT_SCHEMA=8` 於 `:18`）；JSON body 欄位＋少量 typed 欄位做索引；FTS5 只給知識；blob 走檔案系統 CAS；向量索引只在記憶體 |
| 狀態與儲存 owner | `<HOME>/state/interaction.db`；`<home>/state/assets/**`；`<home>/state/{api-token,api-agent-token,runtime.lock,emergency-stop.requested}`；policy 檔／recipes 目錄；桌面 `DesktopPrefs` 由 Tauri 端持有 |
| 現有測試 | `crates/interaction-storage/src/lib.rs` unit；`agents_loop.rs:309`（shutdown→restart 後讀回）；`memory_loop.rs:124`、`:266`；`knowledge_loop.rs:55`、`:478` |
| 證據 commit | `78dcda1` |
| 本次結果 | `implemented-and-connected`；`static-inspection`＋`unit`＋`integration`。**E0-04／E0-11 的重啟與 fresh-home 還原提供執行期佐證**（`e2e-runs/R5/E0-11-restore-fresh-*`） |
| 限制與下一動作 | 跨表關聯沒有 FK 或觸發器；§4 E-X6 補充：`Store::init` 開了 `PRAGMA foreign_keys=ON`（`storage lib.rs:65`）但整個 schema 沒有任何 `REFERENCES`／`FOREIGN KEY` 子句——這個 pragma 是純裝飾，會讓讀碼者誤以為級聯有資料庫層保障。向量索引不持久化、啟動 O(N) 重建；`memory_items` 沒有全文索引。下一動作：移除誤導性的 pragma 或補真正的關聯欄位 |

#### E-03 期限、複查、刪除與可見權限

| 欄位 | 內容 |
|---|---|
| 使用者需求 | TTL、review、delete、agent 可見範圍 |
| 實作 owner | `core memory.rs`（規則）；`{memory,curator,runtime}.rs`（執行）；`api lib.rs`（token 分權） |
| 正式入口及呼叫路徑 | TTL：watchdog `runtime.rs:2421-2427` → `sweep_memory`／`knowledge_freshness_sweep`；讀取端 `memory_get` 在 sweep 前就拒絕 expired（`memory.rs:96-101`）。複查：`PATCH /v1/memory/{id}` retention（`routes.rs:2118-2125` → `memory.rs:56-87`）。刪除：`DELETE /v1/memory/{id}`、`POST /v1/memory/clear-session-context`。可見：`visible_to_agent`＋bundle 過濾（`memory.rs:302-335`）＋token 分權（`api lib.rs:489-545`） |
| 資料與協定契約 | `RetentionPolicy` 三態（expiresAt／reviewAfter／until-deleted）；status 派生 Active/Stale/Expired；agent 建立的內容降權且 horizon 不得超過層級預設；denylist > visibility > 層級預設（UserMemory／SessionContext／PersonaCore 預設不給 agent） |
| 狀態與儲存 owner | `memory_items.expires_at`／`review_after`；`knowledge_nodes.review_after`（JSON body）；狀態不落地、讀時派生 |
| 現有測試 | `core memory.rs:325`、`:378`、`:418`；`memory_loop.rs:124`、`:234`、`:266`；`curator_loop.rs:51`、`:181`；`api_e2e.rs:216-230`、`:252`；`scripts/v03-cli-e2e.sh:293-295`；`regressions-review2-memory.test.tsx:195-239` |
| 證據 commit | `78dcda1` |
| 本次結果 | `implemented-partial`；`static-inspection`＋`unit`＋`integration`。**E0-05 執行期證據**：預設可見性、agent token 403、human `asAgent` 降權都如契約（`e2e-runs/R2/step1-probe-default-visibility.json`、`step7-*.json`） |
| 限制與下一動作 | 「沒有你不能刪除的記憶」只對 `memory_items` 成立；知識節點／邊／收據沒有刪除路徑（review reject 只到 archived）；agent 建立的 UserMemory 永遠被重新降回 Candidate＋30 天，人類無法經 PATCH 升格；UserMemory 預設對 agent 不可見且 UI 沒有 `agentVisibility` 設定，因此使用者偏好幾乎不會進 bundle。§4 E-X4 裁定：本列與 E-09 是同一條不變量的兩個面向——**E-09 保留為獨立的 confirmed-defect**（副本殘留是可指出的錯誤路徑），本列維持 partial 並在此交叉引用。§4 E-X5 補充：`api lib.rs` 的白名單只約束 HTTP 外部呼叫者，Tauri embedded 直呼 Runtime、CLI 用人類 token，不走那道閘。下一動作：補知識節點刪除；決定候選升格流程；UI 暴露 agentVisibility |

#### E-04 查詢、排序、分頁及容量限制（**confirmed-defect**）

| 欄位 | 內容 |
|---|---|
| 使用者需求 | `/v1/memory` GET 參數、`/v1/knowledge/search`、上限常數 |
| 實作 owner | `routes.rs`（參數）；`storage lib.rs`（clamp／ORDER BY）；`{memory,agents}.rs`（上限常數） |
| 正式入口及呼叫路徑 | `GET /v1/memory?layer&limit`（`routes.rs:2075-2091`，預設 200）→ `memory.rs:119-142` → `list_memory` clamp 1..1000 `ORDER BY updated_at DESC`；`GET /v1/knowledge/nodes?status&limit`（預設 100）；`GET /v1/knowledge/search?q&k`（預設 10）；`GET /v1/assets` 固定 200；`GET /v1/knowledge/receipts` 固定 100；CLI `memory list --limit` 預設 50 |
| 資料與協定契約 | 回傳 `items/count/total/limit/limitReached`；知識 list 只回 `nodes/count`；**沒有 offset／cursor／sort／kind／tag／text 參數**；bundle 上限 24 條／48 KiB／掃描 1000；export 1000 |
| 狀態與儲存 owner | 無（純查詢）；上限常數散落 `memory.rs:18-25`、`core memory.rs:183-185`、`core knowledge.rs:232-233`、`knowledge.rs:29-30`、`routes.rs:2232`、`:817`、`agents.rs:50` |
| 現有測試 | `memory_loop.rs:422`、`:467`、`:506`；`knowledge_loop.rs:478`；`curator_loop.rs:181`；`regressions-review3-memory.test.tsx:62-77`；`regressions-review2-memory.test.tsx:102-131`。**沒有任何測試把接近 `BUNDLE_MAX_BYTES` 的 bundle 送進 `mailbox_send`** |
| 證據 commit | `78dcda1` |
| 本次結果 | **`confirmed-defect`**（懷疑者維持）；`static-inspection`＋`unit`＋`integration`＋`not-run`（缺陷路徑本輪未實跑）。與 D-06 是**同一個缺陷的兩個維度視角**：bundle 位元組預算（48 KiB）與 mailbox body 上限（16 KiB）互相矛盾 |
| 限制與下一動作 | 無分頁／游標／排序選項；知識 list 不回 total；HTTP 參數無 golden schema（`schemas/openapi.json` 只含三條路徑）。§4 E-X1 裁定：同一個 48 KiB 預算用兩把尺——domain pack 條目以**完整序列化長度**計費（`memory.rs:266-276`），記憶項只加 `content.len()+title.len()`（`:336`），所以缺陷觸發門檻比原算式更低。§4 E-X2 裁定：原 repro 前提（`data_scope=[domain:rust]`）在任何出貨入口都做不到，但缺陷本身仍成立（D-06 的無 domain 路徑可觸發）。下一動作：把 `BUNDLE_MAX_BYTES` 壓到 mailbox 可容納的值，或讓 bundle 以獨立回執附帶、mailbox 只帶 `bundle_id`；補 cursor 分頁與 total |

#### E-05 關鍵字／語意檢索是否接線

| 欄位 | 內容 |
|---|---|
| 使用者需求 | FTS5、本機向量／稀疏 embedding 真的被 search 路由用到嗎 |
| 實作 owner | `knowledge.rs`（VectorIndex／search）；`storage lib.rs`（knowledge_fts） |
| 正式入口及呼叫路徑 | `GET /v1/knowledge/search`（`routes.rs:2380-2399`）與 tool `interaction.knowledge_search`（`routes.rs:772-781`，session 走 scoped 版）→ `knowledge_search_in_domains`（`knowledge.rs:1260-1315`）→ `store.search_knowledge`（FTS5 bm25）＋`vector_index.query`；CLI `knowledge search`；Tauri `knowledge_search` → 只有進階頁 `KnowledgeAdvanced.tsx:19` 使用 |
| 資料與協定契約 | 回傳 `results[{nodeId,title,status,nodeType,confidence,domains,usable,retrieval:{fts\|vector}}]`＋`retrievalNote`（「FTS=bm25；vector=local-subword-embedding-v1…檢索只產生候選，不代表可信」） |
| 狀態與儲存 owner | `knowledge_fts` 虛擬表（預設 unicode61 tokenizer）；`LocalSubwordEmbeddingIndex` 記憶體 HashMap，啟動重建 |
| 現有測試 | `knowledge_loop.rs:169`、`:596`；`api_e2e.rs:252`；`scripts/v03-cli-e2e.sh:325-326` |
| 證據 commit | `78dcda1` |
| 本次結果 | `implemented-partial`；`static-inspection`＋`integration`（懷疑者維持層級）。無專屬 E0-xx |
| 限制與下一動作 | 語意檢索＝離線稀疏特徵（token＋2/3 字元 subword＋6 組跨語言 concept anchor 雜湊到 8192 桶＋cosine），**不是神經 embedding**（`knowledge.rs:191-193` 已誠實註明）；只服務知識節點，記憶完全沒有檢索；融合只是「FTS 結果先、vector 後、依 id 去重」，沒有分數融合；一般模式沒有檢索入口。CJK 對 FTS 的實際命中率未驗證。§4 E-X8 補充的產品後果：一旦記憶量超過 24 筆／48 KiB，哪些記憶進 bundle 完全由層級序與時間決定，與任務相關性無關——「AI 記得對這次任務有用的事」在資料一多就會確定性失效。下一動作：文案改稱 lexical/subword 候選；測 CJK 命中路徑；決定 bundle 是否改以相關性挑選 |

#### E-06 provenance、附件、版本與關聯

| 欄位 | 內容 |
|---|---|
| 使用者需求 | knowledge receipts、edges、supersede |
| 實作 owner | `core knowledge.rs`（SourceRef／edge 驗證）；`{knowledge,curator}.rs`（receipt／supersede／conflict） |
| 正式入口及呼叫路徑 | propose：`POST /v1/knowledge/nodes` 與 `/edges`（`routes.rs:2300-2363`）、tools `knowledge_propose_*`（`routes.rs:827-933`）；review／supersede 生效：`POST /v1/knowledge/nodes/{id}/review approve`（`routes.rs:2365-2387` → `knowledge.rs:1150-1264`）；receipts：`GET /v1/knowledge/receipts`＋SSE `knowledge.updated`；素材 `/v1/assets/{hash}/{derive,derivatives,impact}` |
| 資料與協定契約 | Claim 每筆 evidence 必含 assetHash 或 url；causes／influences 不得來自 analogy／inspiration／ai-conjecture；review 狀態機 superseded／archived 不可復活；`KnowledgeReceipt{updateId,triggeredBy,agentSessions,sources,sourceHashes,changes,verification,published}` |
| 狀態與儲存 owner | `knowledge_nodes.body`（evidence／reviews／supersedes／version）、`knowledge_edges`、`knowledge_receipts`、`asset_derivatives`；`memory_items.body.provenance` |
| 現有測試 | `knowledge_loop.rs:122`、`:201`、`:304`；`curator_loop.rs:321`；`api_e2e.rs:389`、`:422-445`；`scripts/v03-cli-e2e.sh:306-338`；`e2e/evidence.spec.ts:517-533`；`crates/interaction-tool-schema/src/lib.rs:671` |
| 證據 commit | `78dcda1` |
| 本次結果 | `implemented-and-connected`；`static-inspection`＋`unit`＋`integration`＋`browser`（懷疑者維持該層級，但§4 E-X7 記為「偏寬」：唯一的 browser 根據 `evidence.spec.ts:517-533` 斷言的是 Inbox 通知出現，不是 receipts／version／supersede）。無專屬 E0-xx |
| 限制與下一動作 | 版本號永遠是 1、不遞增；supersede 鏈只有單向 `supersedes` 欄位、不自動建 Supersedes 邊、無反向指標；收據不可查詢過濾也不在匯出範圍；記憶 provenance 只是字串且不隨 bundle 送給 agent；user-correction 的 `evidence.url` 指向記憶但不做存在性驗證。下一動作：approve supersede 時自動建邊並遞增 version；receipts 加 cursor／過濾 |

#### E-07 新偏好取代舊偏好

| 欄位 | 內容 |
|---|---|
| 使用者需求 | supersede／conflict 流程是否確定性、是否影響 bundle |
| 實作 owner | `{memory,curator,knowledge}.rs`；`core memory.rs` |
| 正式入口及呼叫路徑 | 記憶：只有 `PATCH /v1/memory/{id}`（原地覆寫同一 id）與 `POST /v1/knowledge/user-corrections`（`curator.rs:272-385`，每次新建一筆 UserMemory Preference＋Candidate）；知識：approve 帶 supersedes（`knowledge.rs:1191-1198`）＋`knowledge_conflict_check`（`curator.rs:482-552`） |
| 資料與協定契約 | **記憶層沒有 supersede／conflict／dedupe 概念**（`memory.rs` 與 `core memory.rs` 全文無相關邏輯）；知識層 supersede 與 dispute 皆確定性、無 AI |
| 狀態與儲存 owner | `memory_items`（同 id 覆寫，無歷史版本）；`knowledge_nodes.status`／`supersedes` |
| 現有測試 | `knowledge_loop.rs:201`；`curator_loop.rs:74`、`:117`、`:342`；`core knowledge.rs:408`。**沒有測試涵蓋「兩筆同主題偏好同時存在時 bundle 給哪一筆」** |
| 證據 commit | `78dcda1` |
| 本次結果 | `implemented-partial`；`static-inspection`＋`integration`（懷疑者維持層級）。**E0-05 的矛盾記憶段直接觀察到**（`e2e-runs/R2/E0-05-supersede/`）：新舊偏好並存、都進 bundle，新舊判斷外包給下游模型的文字推論——缺陷 **D13** |
| 限制與下一動作 | 記憶偏好層沒有取代／衝突機制；唯一確定性的 supersede 在知識層，而知識不在 bundle；UI 沒有「取代舊偏好」動作。下一動作：為 UserMemory 偏好加 supersedes／dedupe，或在 user-correction 時把舊糾正標 stale；文件明說記憶層無取代流程 |

#### E-08 Agent 原生 Session／記憶如何利用

| 欄位 | 內容 |
|---|---|
| 使用者需求 | resume providerSessionId 是唯一途徑？CLAUDE.md／專案記憶是否被讀 |
| 實作 owner | `{agents,gateway}.rs`；`crates/interaction-agent-gateway/src/{lib,claude,codex,codex_exec}.rs` |
| 正式入口及呼叫路徑 | `POST /v1/agent-sessions{resumeProviderSessionId}` → 驗證（`agents.rs:453-467`）→ `gateway_attach spec.resume_provider_session` → `claude.rs:106-108 --resume`／`codex.rs:321-330 thread/resume`／`codex_exec.rs:69-78 exec resume`；`provider_session_id` 由連接器事件回填；Runtime→agent 的記憶只有 mailbox task 的 contextBundle 包裝 |
| 資料與協定契約 | Runtime 不讀、不存 provider transcript；resume 不得放寬；handoff 落 `AgentHandoff` 層 30 天、confidence 0.5；task-experience 落 `TaskMemory` |
| 狀態與儲存 owner | `agent_sessions.body.provider_session_id`／`resolved_workdir`；provider 端 thread 由 claude／codex 自己的家目錄保存 |
| 現有測試 | `gateway_loop.rs:842`、`:1007`、`:2157`；`codex.rs:907`；`memory_loop.rs:593` |
| 證據 commit | `78dcda1` |
| 本次結果 | `implemented-partial`；`static-inspection`＋`fixture`＋`integration`（懷疑者維持層級；repo 測試仍無真 agent）。**E0-02 補上真 agent 的 resume 證據**（`e2e-runs/R8/argv-claude/run.txt` 直接拍到 `--resume <psid>`；codex 續寫同一 rollout）。**E0-05 補上關鍵的負面證據**：resume 之後真 claude 完整覆誦已刪除的機密暗號（`~/.claude/projects/.../1bf60429-….jsonl:20`，provider-local log，非經 gateway）——這是 E-09 的執行期證明 |
| 限制與下一動作 | CLAUDE.md 是否被 claude 子程序讀取仍未驗（非 Runtime 控制；claude 啟動參數沒有 `--setting-sources` 之類的隔離旗標）；Runtime 無法把 agent 原生記憶納入自己的記憶層或稽核；`TaskMemory` 內容含 `providerSessionId`／`costUsd`，會隨 bundle 送給後續 agent。下一動作：明確決定是否隔離專案 CLAUDE.md；把 providerSessionId 移出 TaskMemory 或標 sensitive |

#### E-09 刪除或撤銷後，舊 Agent 上下文的限制（**confirmed-defect**）

| 欄位 | 內容 |
|---|---|
| 使用者需求 | 刪除記憶後已派送的 bundle 不可收回：有沒有標示／稽核 |
| 實作 owner | `{memory,agents,gateway}.rs`；`routes.rs`（agent_session_get） |
| 正式入口及呼叫路徑 | `DELETE /v1/memory/{id}` → `memory_delete`（`memory.rs:105-112`）只刪一列＋audit。已派送的副本存在四處：(a) mailbox `message.body.contextBundle`（記憶體）、(b) `AgentSessionRecord.context_bundles[]` **完整 bundle 含 content**（落地 `agent_sessions`）、(c) 已送進子程序 stdin 的文字、(d) provider 端原生 transcript（resume 會重用） |
| 資料與協定契約 | `AgentContextBundleReceipt`（`core agent.rs:261-268`）是「實際提供了什麼」的證據，設計上不可由 agent 偽造；**沒有 revoked／stale-after-delete 欄位**；`audit memory.deleted` 不連結 bundle |
| 狀態與儲存 owner | `agent_sessions.body.context_bundles`（每 session 最多 32 筆）；已結束 session 保留最多 200 筆／30 天 |
| 現有測試 | `agents_loop.rs:309-388`（證明 bundle 連內容一起落地並跨重啟保留）。**沒有任何測試涵蓋「刪除記憶後 bundle 回執／mailbox／子程序的處置」** |
| 證據 commit | `78dcda1` |
| 本次結果 | **`confirmed-defect`**（懷疑者維持）；`static-inspection`＋`not-run`（靜態）。**E0-05＝completed 並直接證實此缺陷**：刪除記憶後 bundle `includes=[]` 且新一輪 agent 誠實說不知道，**但 resume 舊 session 後真 claude 完整覆誦已刪除的機密暗號**（`e2e-runs/R2/step5-*.json`、provider-local log）。因此本列的執行期層級為 `real-agent` |
| 限制與下一動作 | 刪除只是單表刪除；四處副本都無標示、無稽核連結、無 UI；`CLOSED_AGENT_SESSION_RETENTION` 30 天內可經 human API 讀回已刪內容；素材刪除有對稱設計（Active 知識→disputed），記憶刪除沒有。下一動作：`memory_delete` 時掃 `agent_sessions.context_bundles`，把命中的 includes 遮蔽為 `{memoryId, redactedAt}` 並寫 `audit memory.deleted{affectedSessions}`；在工作頁顯示「此工作階段曾收到已刪除的記憶」；文件把「已派送不可收回」列為明示限制 |

### 2.F 維度 F — iPhone（6 列）

> 本維度的 `real-iphone` 標記全部來自 [v0.5.0-iphone-device-evidence.md](v0.5.0-iphone-device-evidence.md)（2026-09-03、v0.5.0 建置）。
> **本階段真機執行 0 筆**；E0-08 的真機列為 needs-environment。

#### F-01 配對與可信身分

| 欄位 | 內容 |
|---|---|
| 使用者需求 | pairing code、HMAC challenge、TLS 指紋釘選、每機 token（只存 sha256）、撤銷、`identityStrength=paired-token` |
| 實作 owner | `crates/interaction-runtime/src/mobile.rs`（wss server／配對／token／撤銷）；`character_session.rs`（身分強度投影）；iOS `ConnectionManager.swift`／`PairingStore.swift`／`PairingView.swift` |
| 正式入口及呼叫路徑 | CLI `main.rs:210-213,478` → `commands.rs:1066-1092`；HTTP `api lib.rs:243-254`（`/v1/mobile/status`、`POST /v1/mobile/pairing-session`、`DELETE /v1/mobile/devices/{id}`）→ `routes.rs:2440-2453` → `mobile.rs:2633-2670`／`:2672-2728`／`:2730-2795`；Tauri `lib.rs:1635-1650`（**只走 `rt()`，embedded 專用**；外部模式 WebView 改走 HTTP）；UI `CapabilitiesHub.tsx:672-800` |
| 資料與協定契約 | QR payload `{v:1,host,port,fp,code}`；wire：pair-request → pair-challenge{nonce} → pair-response{HMAC-SHA256(key=code,msg=nonce)} → paired{deviceId,deviceToken}；錯誤 HMAC 燒掉整段配對窗（`mobile.rs:3814-3840`）；`identity_strength` 只認三種常數，其餘一律 unknown（`character_session.rs:1018-1029`） |
| 狀態與儲存 owner | `MobileBridge.devices`（記憶體）＋ `state/mobile-devices.json`（原子寫，壞檔隔離並回報 `devicesUnknown`）；`state/mobile-cert.der`／`mobile-key.der`；配對 session 記憶體、300 s TTL、單一槽位；iOS 端 Keychain |
| 現有測試 | `mobile_loop.rs`（pair helper、`pairing_observation_act_and_estop_loop`、`wrong_pairing_code_is_refused_and_session_burned`、`token_reconnect_works_until_revoked`、`revoke_disconnects_live_connection_immediately`、`the_tls_private_key_is_owner_only…`、`a_corrupt_paired_device_list_is_reported_as_unknown_not_as_none` 等）；`e2e/iphone.spec.ts:78`、`:188`；`scripts/v03-cli-e2e.sh:567-580`；iOS `ProtocolTests.swift`、`ConnectionManagerGateTests.swift`、`ReconnectHintTests.swift` |
| 證據 commit | `78dcda1`（`real-iphone` 列除外，見本節開頭） |
| 本次結果 | `implemented-and-connected`；`static-inspection`＋`unit`＋`simulator`＋`integration`＋`fixture`＋`browser`＋`real-iphone`(v0.5.0 建置)。**E0-08＝completed（fixture 範圍）**：配對成功、撤銷立即斷線且舊 token 被拒（`e2e-runs/R6/logs/{pair-2.json,phone.txt,daemon-responses.jsonl}`）；**真機列＝needs-environment** |
| 限制與下一動作 | TOFU 指紋釘選、無 CA；host／port 釘在配對當下、iOS 無 Bonjour 探索 → 桌面換 IP 必須重新配對；配對碼 6 位、單次錯誤即燒窗；真機證據來自 v0.5.0 建置，之後的配對相關程式只有 integration／fixture／模擬器證據。§4 F-X9 記錄的未追事項：tray／原生（非 WebView）路徑是否呼叫了只走 `rt()` 的 mobile IPC → `needs-investigation`。下一動作：人類在 Xcode 選 Team 後重跑 `device-build.sh`＋`device-acceptance.sh` |

#### F-02 雙向互動及套用回執

| 欄位 | 內容 |
|---|---|
| 使用者需求 | AIP frame 雙向、`aip.applied/1` 回執、StateAppliedTracker、桌面顯示 `stateAppliedCurrent` |
| 實作 owner | `mobile.rs`（wire 分派、per-connection tracker）＋ `character_session.rs`（Session Host 接線）＋ `crates/interaction-session`（安全管線、決策表）＋ `crates/interaction-adapter-declarative/src/state_applied.rs`；iOS `SessionClient.swift`／`SessionReceive.swift` |
| 正式入口及呼叫路徑 | 手機→桌面：iOS `SessionClient.connectionDidConnect:644-670` 送 capability（`features.stateApplied="aip.applied/1"`）→ `mobile.rs:3988-4080` → `character_session_device_frame`（`character_session.rs:1566-1729`）。桌面→手機：`register_device_outbound`（`character_session.rs:971-991`）→ `MobileOutbound::send_aip`（`mobile.rs:118-124`）→ `send_aip_on_connection`（`:1395-1452`，tracker 加回執） |
| 資料與協定契約 | `docs/aip/transport-bindings.md` §1；`docs/aip/character-session.md` §7.2 十六列決策表（Rust `receive.rs` 權威，iOS 對 `manifest.json` 45 案例）；`state_applied.rs:20-22`（`APPLIED_PROFILE="aip.applied/1"`、`MAX_PENDING_APPLIED=32`、`APPLIED_TIMEOUT_MS=5000`） |
| 狀態與儲存 owner | 權威語意狀態：Session Host（持久化 JSON 快照）；回執追蹤：每條連線的 `StateAppliedTracker`（斷線／被取代即 invalidate）；iOS `SessionSyncLocal` 只在記憶體 |
| 現有測試 | `character_session_loop.rs:2417-2500+`（真 TLS＋程序內模擬手機：錯 hash 拒、對的收、patch 後回執、取代連線即失效）、`:454`、`:750`、`:781`；`mobile_loop.rs:3759`；`state_applied.rs` unit；iOS `SessionClientTests`／`AIPConformanceTests`／`ReceiveDecisionConformanceTests`／`StateHashConformanceTests`／`CanonicalVectorsTests`；`e2e/character-session.spec.ts:263/275/289`；`scripts/v03-cli-e2e.sh:525-600+` |
| 證據 commit | `78dcda1` |
| 本次結果 | `implemented-and-connected`；`static-inspection`＋`unit`＋`contract`＋`simulator`＋`integration`＋`fixture`＋`browser`（**不含 real-iphone**）。**E0-08＝completed（fixture 範圍）**：AIP revision 1→3、reconnect patches（`e2e-runs/R6/logs/`） |
| 限制與下一動作 | applied 是 **peer 自報**，不是畫面或實體驗證；只有雙方協商後才有回執，未協商成員永遠 unconfirmed；iOS 不主動送 AIP heartbeat，presence 靠 v1 status；touch 5 秒過期、重連不重播；**真 iPhone 上的 AIP／applied 零執行**。下一動作：真機執行一次 capability→snapshot→applied→patch→applied 閉環並記錄 `diagnostics.stateAppliedCurrent` |

#### F-03 工作與對話訊息

| 欄位 | 內容 |
|---|---|
| 使用者需求 | iPhone 能否看到 agent session／工作狀態／串流？iOS app 有哪些畫面；mobile wire 有無 work/task 訊息 |
| 實作 owner | `character.rs`（session_projection）＋ `character_session.rs`（`character_session_note_agent_state`）＋ `agents.rs`（verify → `mobile_present_verified`）＋ `mobile.rs`；iOS `CharacterView.swift`／`CharacterSemantic.swift`／`ActuatorCenter.swift` |
| 正式入口及呼叫路徑 | iOS 只有三個分頁（`ContentView.swift:29-47`：連線／感測／角色），**沒有工作或訊息頁**。wire 型別（`Protocol.swift:301-329`／`:479-505`；`mobile.rs:3769-4240`）沒有任何 work／task／message 型別。工作狀態到手機只有兩條路：legacy 動器 `iphone.character`（五態）與 AIP 語意狀態的 `truthState` |
| 資料與協定契約 | `docs/aip/iphone-companion.md` §1：手機是 remote-renderer，只送語意事件、只收語意狀態與 Behavior Intent；`docs/aip/semantic-state.md`（15 truthState，只由 Runtime 設定）；`mobile.rs:276-295`（verified-success／emergency 不可由 AI 冒充） |
| 狀態與儲存 owner | Agent session 記錄在 Runtime（SQLite）；語意 truth 在 Session Host（只轉錄不推論）；手機端只有記憶體投影 |
| 現有測試 | `mobile_loop.rs:458`、`:516`、`:1517`；`character_session_loop.rs`（RuntimeFact 廣播到 fixture iPhone）；iOS `SessionClientTests.swift`；`e2e/character-session.spec.ts:263-289` |
| 證據 commit | `78dcda1`（`real-iphone` 列除外） |
| 本次結果 | `implemented-partial`；`static-inspection`＋`unit`＋`simulator`＋`integration`＋`fixture`＋`browser`＋`real-iphone`(v0.5.0 建置，僅 legacy `character.present` 五態)。E0-08 不涵蓋工作面（fixture 只驗配對／狀態／感測） |
| 限制與下一動作 | 手機只看得到粗粒度真相與角色動作，沒有 session 身分、訊息文字、串流、等待輸入的問題內容、通知推播；對話訊息面 **absent**。§4 F-X1 裁定：`iphone.notify`／`iphone.tts` 動器**存在且有正式入口**（`mobile.rs:222-226`、`:477-496`；CLI `main.rs:179` 的 plan／act），真正不存在的是「工作事件自動推到手機」的產生者（grep `iphone.notify` 於 proactive/orchestrator/agents/executor/activity/gateway ＝ 0）——本列措辭已依此限定。下一動作：產品決策手機是否成為工作面；若否，在 USER-GUIDE 明說 |

#### F-04 回答、核准的可信通道

| 欄位 | 內容 |
|---|---|
| 使用者需求 | iPhone 能否 approve／回答 agent？走什麼 API／token？ |
| 實作 owner | 核准只在桌面可信介面：`crates/interaction-api`（`/approve`、`/messages`，human token）＋ `gateway.rs`／`agents.rs`；AIP `approval-*` 型別在 `crates/interaction-aip`；iOS 端刻意忽略 |
| 正式入口及呼叫路徑 | **不存在**。手機入口只有 wss，inbound match 沒有 approve／answer／mailbox 型別；aip frame 進 `character_session_device_frame` 後成員只能送 Event／Cancel／Query／Result／Heartbeat／Capability（`session.rs:1034-1045`），`Result{verified}` 被拒（`:1017-1022`），任何 `consentGrantId` 被拒（`:1027-1033`）。核准的正式入口是 `POST /v1/agent-sessions/{id}/approve`（human token） |
| 資料與協定契約 | `MessageType::ApprovalRequest`／`ApprovalResult` 與 Party kind human 存在（`crates/interaction-aip/src/message.rs:17-18,54-55,82`）；`crates/interaction-session/src/ports.rs:70-78` 明說 approval-* 是「1.1 以後」、1.0 未接進 gate；CLAUDE.md：AI 不可授予 consent；每機 token 是**裝置身分**不是人類身分 |
| 狀態與儲存 owner | 核准狀態在 gateway approvals map ＋ agent session mailbox——手機無讀寫路徑 |
| 現有測試 | iOS `LifecycleTests.swift:458`（其他 message type 被忽略並留 note）；`crates/interaction-session` gate 測試；`api lib.rs:751-763`（agent token 不能打 `/approve`、`/messages`） |
| 證據 commit | `78dcda1` |
| 本次結果 | `absent`；`static-inspection`＋`unit`＋`simulator`（懷疑者維持）。**E0-10 的 agent token 阻擋段間接佐證邊界成立**（`e2e-runs/R4/f-agent-token/`） |
| 限制與下一動作 | AIP `approval-*` 只是型別定義（defined-only），沒有 host 產生者、沒有 session gate 支援、沒有 iOS UI。§4 F-X7 補充：手機確實有一條 member→host 的**控制回饋**通道（`Result{observed}`／`cancelConfirmed`，限於角色呈現），本列的 absent 只針對「人類核准／回答」。下一動作：列為產品／安全決策題——要支援就先寫 AIP 1.1 的 approval profile 與威脅模型（裝置身分 vs 人類身分），否則在 USER-GUIDE 明說手機不能核准 |

#### F-05 前背景、系統終止、安全重連

| 欄位 | 內容 |
|---|---|
| 使用者需求 | ConnectionManager 生命週期、heartbeat、重連策略、撤銷即斷線、被取代連線、冷啟動 |
| 實作 owner | iOS `ConnectionManager.swift`／`AppLifecycle.swift`／`SocketTransport.swift`／`InteractionCompanionApp.swift`；Rust `mobile.rs`＋`character_session.rs`＋`interaction-session`（presence tick） |
| 正式入口及呼叫路徑 | iOS `InteractionCompanionApp.swift:183-192` scenePhase → `sensors.setForeground`＋`connection.lifecyclePhaseChanged`（`:510-580`：background 停 status timer、取消重試；active 立即 status＋重啟 timer、未協商補送 capability、socket 死且仍想連線則跳過退避立即重連）；心跳 `startStatusTimer:822-841`（每 15 s）；斷線 `handleConnectionLost:851-878` → `scheduleRetry:883-908`（退避 1→15 s，背景不排重連）。Rust：收到 status 才 `touch_presence`（`mobile.rs:4200-4212`），45 s 逾時 → offline |
| 資料與協定契約 | `docs/aip/iphone-companion.md` §2 生命週期表；`docs/aip/transport-bindings.md` §1.4（status 即存活證明；AIP heartbeat 尚未主動送出）；`docs/aip/pairing-security.md` §5／§6／§7；`PresenceHeartbeatPolicy`（15 s 間隔、45 s 逾時，跨端契約）；`Info.plist:47` 刻意不宣告 `UIBackgroundModes` |
| 狀態與儲存 owner | iOS：`UserDefaults` autoConnectIntent、Keychain 配對、記憶體 failureHistory／connectionGeneration；Rust：per-connection `ConnState`、providers registry、session 成員 presence、audit |
| 現有測試 | iOS `LifecycleTests.swift`（決策表、背景不重連、status 間隔留餘裕…）、`ConnectionManagerGateTests.swift`、`ReconnectHintTests.swift`（冷啟動意圖、TLS 不符不誤報為位址變更）；`mobile_loop.rs:882-925`、`:734`、`:773`、`:930`；`character_session_loop.rs:1001`、`:1952`；`e2e/iphone.spec.ts:163`、`:188`；`e2e/character-session.spec.ts:263`；`scripts/tests/tauri-mobile-sensor-walkthrough.py` |
| 證據 commit | `78dcda1`（`real-iphone` 列除外） |
| 本次結果 | `implemented-and-connected`；`static-inspection`＋`unit`＋`simulator`＋`integration`＋`fixture`＋`browser`＋`native-desktop`＋`real-iphone`(v0.5.0 建置)。**E0-08＝completed（fixture 範圍）**：reconnect patches、撤銷即斷線且舊 token 被拒。**本輪 native 走查 harness-failed**（4/10 與 6/10 journey 後 AX dump 超過 55 s 上限，四個 sensor journey 從未跑到——缺陷 **D19**） |
| 限制與下一動作 | 無 Background Mode → 進背景後桌面最遲 45 s 標 offline（設計如此）；真 URLSession 回呼執行緒／Timer 排程未在真裝置驗證；系統終止後的冷啟動自動重連真機 0 次。§4 F-X2 裁定：本列的 `native-desktop` 標記來自 `tauri-mobile-sensor-walkthrough.py`（fixture 驅動桌面 AX），**對 iOS 端的前背景／重連不提供證據**，標籤保留但語意限定於桌面側呈現。§4 F-X5 裁定：「AIP heartbeat 未實作」應精確寫成「iOS 會回應 host 的 AIP heartbeat（以 legacy status 回），只是不主動送」（`LifecycleTests.swift:390`）。§4 F-X8 裁定：location **不是真的感測**（`SensorCenter.swift` 從不呼叫 `startUpdatingLocation`、runtime 無 `iphone.location` 受器），本列與 F-06 不得把它當安全行為的證據。下一動作：真機重跑背景／終止／撤銷／daemon 重啟；先修 D19 的 AX dump |

#### F-06 核心離線行為

| 欄位 | 內容 |
|---|---|
| 使用者需求 | daemon 不在／連不到時 iPhone 顯示什麼？有無佇列？桌面端如何呈現手機離線？daemon 重啟後如何恢復？ |
| 實作 owner | iOS `ConnectionManager.swift`（phase 文案、退避、ReconnectDiagnosis）／`PairingView.swift`／`CharacterSemantic.swift`／`SessionClient.swift`；Rust `mobile.rs`／`character.rs`／`interaction-session` |
| 正式入口及呼叫路徑 | 手機側：連不上 → `handleConnectionLost:851-878` → `scheduleRetry:883-908`（phase `.waitingRetry`）；連續 4 次或持續 60 s connectivity 失敗 → `ReconnectDiagnosis.suggestRepair`（`:146-190`）→ `PairingView.swift:94-105` 顯示「連不上桌面：可能是桌面的網路位址已變更。請在桌面重新產生配對碼並重新配對。」；角色頁 `syncLine .offline`。**無任何佇列**：`send()` 未連線即丟棄並計數 `droppedFrames`。Rust：`runtime.rs:531` → `mobile_autostart_if_paired` |
| 資料與協定契約 | `docs/aip/iphone-companion.md` §2／§7；`docs/aip/character-session.md` §11；`docs/aip/general-mode-ux.md:31-42`；AIP §8 離線政策（touch expire-by-deadline、intent drop-if-offline） |
| 狀態與儲存 owner | iOS：phase／failureHistory／reconnectDiagnosis／CharacterState（記憶體，斷線不清）、`UserDefaults` autoConnectIntent；Rust：`MobileBridge.conns`／`devices`、`mobile-devices.json`、session presence、audit |
| 現有測試 | iOS `ReconnectHintTests.swift`（4 次連續失敗建議重新配對、3 次仍重試、60 s 持續失敗、POSIX 分類）、`ConnectionManagerGateTests.swift:51`、`LifecycleTests.swift:248/102`、`SessionClientTests.swift`；`mobile_loop.rs:882`、`:930`、`:2217`；`character_session_loop.rs:1952`；`e2e/iphone.spec.ts:163`；`e2e/character-session.spec.ts:263` |
| 證據 commit | `78dcda1` |
| 本次結果 | `implemented-partial`；`static-inspection`＋`unit`＋`simulator`＋`integration`＋`fixture`＋`browser`（懷疑者維持）。**E0-08 的 F-06 段：daemon 側 completed，裝置側 harness-failed** —— 出貨 fixture 在連線被拒時 `process::exit(2)`（`crates/interaction-runtime/examples/fake_iphone.rs:177`←`:298`，缺陷 **D17**），本輪改用自寫 WS probe 驗證（`e2e-runs/R6/logs/{probe-while-dead.json,probe-after-restart.json}`） |
| 限制與下一動作 | 無離線佇列（設計：queued≠completed，丟棄並計數）；手機離線時角色頁顯示的是舊狀態（有 offline 提示但未標「舊」）；`ConnectionFailureKind.classify` 把 ECONNREFUSED 與 EHOSTUNREACH 一起歸為 connectivity，因此**「daemon 只是沒開」與「桌面換 IP」在手機端不可區分**，退避 1+2+4+8 s 後就會建議重新配對。§4 F-X3 裁定：懷疑者判為 disclosed limitation、盤點者判為 confirmed-defect 候選，雙方都承認未實跑 → 本文件維持 `implemented-partial` 並把該爭議記為 **open**（§4）。下一動作：修 D17 讓出貨 fixture 可走完恢復路徑；區分 daemon-down 與 IP 變更的文案；真機演練 daemon stop/start 與 Wi-Fi 切換 |

### 2.G 維度 G — 視覺化與一般模式（6 列）

#### G-01 對話、工作時間線、串流、待回答事項

| 欄位 | 內容 |
|---|---|
| 使用者需求 | `WorkPage`／`work/` 元件、`Timeline`、`ActivityPage` 各吃哪個 API／SSE；有沒有「待回答」清單 |
| 實作 owner | `apps/interaction-desktop`（`WorkPage.tsx`、`AiPage.tsx`、`work/TaskComposer.tsx`、`ActivityPage.tsx`、`Timeline.tsx`、`components/NotificationPanel.tsx`）＋ `{agents,gateway,activity}.rs` ＋ `crates/interaction-api/src/{lib,sse}.rs` |
| 正式入口及呼叫路徑 | `App.tsx:609-627` → `WorkPage.tsx:84-107` → `TaskComposer.tsx:460-502` → `/v1/agent-sessions*`（`api lib.rs:216-240`）或 Tauri IPC；SessionCard `AiPage.tsx:523-542` 每 5 s 輪詢 `agentSessionMessages('from-session')`。事件流 `transport.ts:440-475`（`/v1/events`＋Last-Event-ID）或 `:517`（Tauri `runtime-event-raw`）→ `App.tsx:108-111` bump refreshKey。待回答：`App.tsx:295-299`／`HomePage.tsx:470`／`ActivityPage.tsx:286-289` → `GET /v1/activity/inbox`（`activity.rs:93-135`） |
| 資料與協定契約 | `AgentSessionRecord`＋mailbox message；SSE `agent.session.state`（human-only）；`ActivityInboxItem{kind,status,route,needsDecision,pendingCount,pendingCountExact}`；桌面投影 `workState.ts:9-35,112-170`、`inbox.ts:17-154`；delivery 六態 `work/delivery.ts` |
| 狀態與儲存 owner | Runtime `agent_sessions` map＋persist；mailbox VecDeque per session；SSE replay buffer；桌面只保留最近 300 則事件與輪詢結果，不持久化 |
| 現有測試 | `workPage.test.tsx:121-433`；`aiPageResume.test.tsx:141-200`；`statusProjection.test.tsx:234-380`；`regressions-v04.test.tsx:440-480`；`pendingCountExact.test.tsx:35-110`；`homePage.test.tsx:189-243`；`e2e/work-delegate.spec.ts:110-360`、`home-state.spec.ts:76-200`、`evidence.spec.ts:320-432,515`、`app.spec.ts:288`；`gateway_loop.rs`／`agents_loop.rs:681`；`api_e2e.rs:475-501`；`scripts/v03-cli-e2e.sh:182-198`；`scripts/tests/tauri-work-cancel.py` |
| 證據 commit | `78dcda1` |
| 本次結果 | `implemented-partial`；`static-inspection`＋`unit`＋`fixture`＋`integration`＋`browser`＋`native-desktop`（懷疑者維持；§4 G-X8 記錄了「原生證據對不到本列核心需求」的爭議）。**E0-09＝not-implemented**（待回答的自動偵測不存在，缺陷 **D11**） |
| 限制與下一動作 | (1) 進度／工具／產出不進信箱，一般模式只看得到最終 result 與核可請求（後端只寫三種 from-session 訊息：`approval-request`／`result`／`approval-resolved`）；(2) 訊息靠 5 s 輪詢而非事件驅動；(3) waiting-for-input 僅手動回報可達；(4) ai-assist 待決定項 route 到 `automations`，但 `AutomationsPage` 沒有對應處理（`aiAssistResolve`／`aiAssistsList` 在 `src` 都找不到非測試呼叫端，§4 G-X7）；(5) 進階 Timeline 無渲染測試。下一動作：決定 progress／tool／artifact 的呈現面（與 G-05 是**同一個決策點**，§4 G-X5）；把 ai-assist 接進頁面或改 route |

#### G-02 角色與 UI 是否使用同一工作真相

| 欄位 | 內容 |
|---|---|
| 使用者需求 | `character.rs` 對 agent session 狀態的投影 vs `statusProjection/workState.ts`；角色會不會自己推測完成 |
| 實作 owner | `{agents,character,character_session}.rs`；`statusProjection/workState.ts`、`AiPage.tsx`；`companion/`、`character/` |
| 正式入口及呼叫路徑 | 唯一寫入路徑 `report_agent_session`（`agents.rs:1174-1261`）／`verify_agent_session`（`:1111-1172`，唯一 verified 來源）／`close`／`expire`／restart→unknown → `emit_agent_session_state`（`:1098-1106`）→ (a) SSE → UI 重抓 → `projectWorkState`；(b) `character_project_session`（`character.rs:1878-1893`）→ `session_projection`（`:467-486`）→ `character.intent`；iPhone 由 Character Session snapshot 讀 `truth.state` |
| 資料與協定契約 | `agent.session.state` payload `{agentSessionId, agentId, state}`；CPP §11 對照表；桌面 `WORK_STATE_PROJECTION`（`workState.ts:112-170`，含 taxonomy 別名 fetched／working／waiting-input） |
| 狀態與儲存 owner | Runtime：`record.state`／`claim_id`／`human_verified`（持久化）；角色端無持久化真相（`machine.ts` transient 2.5 s）；Character Session 另存 truth 快照 |
| 現有測試 | `character.rs:2552`；`character_loop.rs:361`、`:704`；`agents_loop.rs:681`；`gateway_loop.rs:1286`、`:1772`；`statusProjection.test.tsx:101-124`；`regressions-v06-general-mode.test.tsx:135-147`；`rig.test.ts:142`；iOS `SessionClientTests.swift:474`、`AIPConformanceTests.swift:222-224`；`e2e/evidence.spec.ts:320-432` |
| 證據 commit | `78dcda1` |
| 本次結果 | `implemented-and-connected`（**限縮為 agent session 層**，§4 G-X1 裁定）；`static-inspection`＋`unit`＋`integration`＋`fixture`＋`browser`。缺陷 **D10**（SSE 混用 gateway 階段名與 record 狀態） |
| 限制與下一動作 | 角色不自行推測完成（`character_session.rs:1393-1413`「只轉錄，不推論」；桌面 `character/gateway.ts:824-838` 對非 runtime 來源的 truthState 強制 none 並 audit forged-truth-state）。載體差異：UI 的「已由你確認」在 closed 後仍顯示，角色只收到一次性 verified 事件、closed 後投影成 Idle；未知 raw 值時 UI 顯示「結果不確定」而角色沉默。**動作收據層不同源**：`character.rs:495` 把 `action.completed` 投影成 claimed，桌面 `ui.tsx:12-16` 卻畫成綠色 ok（見 G-06）。§4 G-X10 補充：iOS 四個 View 內 grep `agent`／`approve`／`工作` ＝ 0，手機沒有工作面（見 §5.1 的 G-09）。下一動作：補一支跨端測試同時斷言 GET／SSE／character.intent／Session truth 四者一致 |

#### G-03 五入口與主要操作

| 欄位 | 內容 |
|---|---|
| 使用者需求 | `general-mode-ux.md` 五入口對應頁面與測試 |
| 實作 owner | `routing.ts`（純資料）、`App.tsx PageBody`、`components/NarrowNav.tsx`、`pages/{HomePage,CompanionPage,WorkPage,ConnectPage,MorePage}.tsx` |
| 正式入口及呼叫路徑 | `routing.ts:25-31 SIMPLE_NAV`（home／companion／work／connect／more）→ `App.tsx:584-748 PageBody`；`LEGACY_ANCHORS`（`routing.ts:55-71`）折疊舊 id；窄視窗 `NARROW_PRIMARY:92`＋`NARROW_MORE_ITEMS:96-102` |
| 資料與協定契約 | `general-mode-ux.md:8-20`（主入口恰五個；角色同步不是第六入口）；`DESKTOP-GUIDE.md:176-188`、`:276-293` |
| 狀態與儲存 owner | `useNavigation.ts`（目前路由）；`prefs.mode`（simple／advanced）由後端 UI 偏好持久化 |
| 現有測試 | `regressions-v06-general-mode.test.tsx:57-65`（SIMPLE_NAV 長度 5 與 id 鎖死）；`e2e/app.spec.ts:122-137`、`:62`、`:198`、`:230`、`:350`；`homePage.test.tsx:189-243,319-401`；`narrowNav.test.tsx:37-100`、`deep-links.test.tsx:170-300`；`e2e/evidence.spec.ts:85,167`、`general-mode-tasks.spec.ts:574-724`；`scripts/tests/tauri-settings-walkthrough.py`、`tauri-work-cancel.py` |
| 證據 commit | `78dcda1` |
| 本次結果 | `implemented-and-connected`；`static-inspection`＋`unit`＋`browser`＋`native-desktop`＋`fixture`。E0-01 的 native 走查（`e2e-runs/R7/preset/`）間接覆蓋入口可達性 |
| 限制與下一動作 | **文件漂移**：`DESKTOP-GUIDE.md:181` 列「排隊中／進行中／說做完了／驗證成功」，程式標籤是「正在準備／處理中／對方說已完成／已由你確認」（`workState.ts:73,86,130,137`）。§4 G-X2 裁定：不能直接照 `:289-291` 對齊——那份清單同樣列了「等你回答」，而該狀態沒有 connector 會產生，必須**先**採 G-01／A-05a 的處置（標明只有手動 report 可達）再談文案對齊。真人可用性為零。下一動作：修文案；把「主入口恰五個」加進 `docs-claims.sh` 或 e2e 斷言 |

#### G-04 一般文案與進階診斷

| 欄位 | 內容 |
|---|---|
| 使用者需求 | 技術詞守門測試、進階頁 |
| 實作 owner | `statusProjection/*`、`MorePage.tsx AdvancedFeaturesSection`、`App.tsx`（ADVANCED_NAV gating）、各頁 `advanced` prop |
| 正式入口及呼叫路徑 | 更多→進階模式 `MorePage.tsx:134-165` → `App.tsx:379-395` 顯示 ADVANCED_NAV → `PageBody adv-*`；各頁以 `advanced` 開關次要行（`AiPage.tsx:553-575`、`ActivityPage.tsx:184-189`、`TaskComposer.tsx:694,711`、`MemoryKnowledgePage.tsx:75-88`） |
| 資料與協定契約 | `general-mode-ux.md` §2／§5（一般模式不出現 revision／sequence／epoch／eventLog／storeNote／token／provider id／裝置識別碼／原始 payload）；`DESKTOP-GUIDE.md:292`（一般模式不顯示 JSON） |
| 狀態與儲存 owner | `prefs.mode`（Runtime UI 偏好） |
| 現有測試 | `general-mode-no-technical-terms.test.tsx:21,137-280`（DOM 掃描；`:228` 進階模式反向斷言）；`workPage.test.tsx:511-570`（DOM 掃描＋原始碼字面掃描）；`statusProjection.test.tsx:88,182-216,252,340-380,455-479`；`regressions-v05.test.tsx:411-427`；`regressions-v06-general-mode.test.tsx:68-80`；`e2e/app.spec.ts:245`、`:301-322` |
| 證據 commit | `78dcda1` |
| 本次結果 | `implemented-and-connected`；`static-inspection`＋`unit`＋`browser`。無專屬 E0-xx |
| 限制與下一動作 | 守門是逐元件 DOM 掃描（同步卡、未解決停止、WorkPage、記憶頁），**沒有全 app 的技術詞掃描**；一般模式仍會直出後端英文原文的三處：`ActivityPage.tsx:144 receipt.errors[].message`、`:363 item.domains` 原始 domain id、`AiPage.tsx:339` agents discovery detail——三處都沒有守門測試覆蓋。§4 G-X4 記錄了與 G-06 的判準不一致（同為可指出的契約違反卻差兩級），本文件維持 connected 並在此明列缺口。下一動作：加 TECHNICAL regex 掃 HomePage／ActivityPage／ConnectPage 的一般模式 DOM；為 `receipt.errors` 與 discovery detail 加人話投影 |

#### G-05 上下文來源、執行證據與成果是否可查看

| 欄位 | 內容 |
|---|---|
| 使用者需求 | bundle viewer？receipts viewer？evidence 畫面？ |
| 實作 owner | `{agents,memory,executor}.rs`（證據產生）；`crates/interaction-api`；`pages/{MemoryKnowledgePage,ActivityPage,HomePage,AiPage}.tsx`；CLI |
| 正式入口及呼叫路徑 | 真實 bundle：`agents.rs:860-886` dispatch 時寫進 to-session task body ＋ `AgentContextBundleReceipt` 進 `record.context_bundles`（`:928-941`）→ 只能經 `GET /v1/agent-sessions/{id}` 或 `messages?direction=to-session`（CLI `commands.rs:1008-1016`）看到。桌面預覽：`MemoryKnowledgePage.tsx:1286-1332`（固定 `domains=[]`）。執行證據：SessionCard result summary／costUsd、核可裁決含 `deliveredToAgent`、`ActivityPage.tsx:80-192` 動作收據＋重新驗證 |
| 資料與協定契約 | `AgentContextBundleReceipt`（`core agent.rs:243`）；`ActionReceipt{policyDecisions, verification{verdict,detail}, errors}`；knowledge receipts |
| 狀態與儲存 owner | Runtime：`record.context_bundles`（隨 session 持久化）、receipts、knowledge receipts；桌面不快取 |
| 現有測試 | `e2e/evidence.spec.ts:85-740`（各頁截圖；`:320-432` 工作四態）；`regressions-review2-ia.test.tsx:292-345`；`regressions-v05.test.tsx:411-427`；`agents_loop.rs`／`gateway_loop.rs`／`proactive_loop.rs`（含 contextBundle 斷言） |
| 證據 commit | `78dcda1` |
| 本次結果 | `implemented-partial`；`static-inspection`＋`unit`＋`integration`＋`fixture`＋`browser`。§4 G-X3 裁定：本列原本引用 `api_e2e.rs:475-501` 作為 bundle／收據的 integration 證據**不成立**（該段是 knowledge-node／inbox 內容），應改引 `agents_loop.rs:309-383`／`proactive_loop.rs:426-427`／`gateway_loop.rs:277`。**E0-05 提供真 agent 的 bundle 端到端證據**（`e2e-runs/R2/step2-bundle-{cli,http}.*`） |
| 限制與下一動作 | 桌面看不到「這次真的送了什麼」：`api.ts` 沒有 `contextBundles` 存取，SessionCard 只抓 from-session，to-session 的 task 訊息在桌面沒有讀取點；只有預覽且固定 `domains=[]`。執行過程證據（tool／artifact／token）只進觀察，一般模式沒有檢視面。下一動作：在 SessionCard 加「這次提供了什麼」讀 `record.contextBundles`（與 G-01 合為同一個決策點） |

#### G-06 unknown／submitted／claimed／verified 是否混淆（**confirmed-defect**）

| 欄位 | 內容 |
|---|---|
| 使用者需求 | 找出任何把 claimed 顯示成完成的地方 |
| 實作 owner | `statusProjection/{workState,inbox}.ts`、`ui.tsx statusBadgeKind`、`appstate.tsx actionStatusLabel`、`pages/{ActivityPage,HomePage,AiPage,Timeline}.tsx`、`work/delivery.ts`；`executor.rs`、`core receipt.rs` |
| 正式入口及呼叫路徑 | Agent session：`record.state` → `projectWorkState`（`workState.ts:196-199`）→ SessionCard／NowStrip／Inbox；verified 只在 `verifiedForCurrentClaim`（`AiPage.tsx:177-189`）。動作收據：`receipt.currentStatus` → `statusBadgeKind`（`ui.tsx:10-30`）＋`actionStatusLabel`（`appstate.tsx:209-239`）→ `ActivityPage.tsx:104-106,129-133`。後端：`executor.rs:1060-1063`（observed→completed）與 `:1115-1134`（ack-only 但 actuator 宣告 `ack=delivered` → Completed，verdict `AcknowledgedOnly`） |
| 資料與協定契約 | CLAUDE.md 誠實階梯 completed≠verified；CPP README §11 `action.completed`→claim-completed／claimed（`docs/character-protocol/README.md:367`；`character.rs:495`）；**但** `core receipt.rs:29-30` 註解說 Completed＝「goal achieved and verified to the configured standard」——三份定義不一致 |
| 狀態與儲存 owner | Runtime（`record.state`／`human_verified`／`claim_id`；`ActionReceipt.current_status`／`verification`）；桌面純投影 |
| 現有測試 | `statusProjection.test.tsx:75-124`、`:155-195`（**只斷言 kind，不斷言 badge**）；`honesty.test.ts:7-28`（未覆蓋 completed）；`workPage.test.tsx:433-510`、`delivery.test.ts`；`e2e/evidence.spec.ts:320-432`、`work-delegate.spec.ts:252-326`、`home-state.spec.ts:94-200`；`gateway_loop.rs:624`、`:1286`、`:1772`；`agents_loop.rs:681`；`scripts/v03-cli-e2e.sh:192-198`；`scripts/tests/tauri-work-cancel.py` |
| 證據 commit | `78dcda1` |
| 本次結果 | **`confirmed-defect`**（懷疑者維持）；`static-inspection`＋`unit`＋`integration`＋`fixture`＋`browser`＋`native-desktop`。**agent session 這一層是乾淨的**（E0-02／E0-03／E0-10 執行期也未觀察到 claimed 被畫成成功）；缺陷只在**動作收據**：`executor.rs:1115-1134` 對宣告 `ack=delivered` 的 actuator 直接把 Acknowledged 轉 Completed（verdict `AcknowledgedOnly`），CPP 把它投影成 claimed，桌面 `ui.tsx:12-16` 卻把 `completed` 畫成綠色 `ok`。相關缺陷 **D2**（close 投影壓掉終局狀態）、**D14**（claimed session 收新任務時 state 不反映） |
| 限制與下一動作 | 測試缺口：沒有任何斷言擋住 `completed → ok` 徽章。次要不一致（懷疑者已確認為真，本文件據此升為需處置項）：`HomePage.tsx:787` 以 `!s.closedAt` 計「交代中」而 `:489` 用 `isOpenWorkState`，而 `report_agent_session` 設 Failed／Unknown／TimedOut 時不碰 `closed_at`、`is_open()` 又排除這四態 → 失敗／未知工作可能被算成「交代中」。下一動作：裁定 `ActionStatus::Completed` 的語意（core 註解 vs executor vs CPP）；把 `completed` 徽章對 `verdict != observed` 改成非 ok 並加「（未觀察到效果）」；補 badge 斷言測試；統一 HomePage 兩處判準 |

### 2.K 維度 K — 模型選擇與 connector 相容（8 列）

> 本維度的靜態盤點另外做了唯讀的環境探測：`claude --help`、`codex --version`、`codex exec --help`、
> `codex app-server --help`、`codex app-server generate-json-schema`（輸出只寫到 scratchpad）。
> 這些是 **environment-probe**，不是 repo 測試證據，也不使該列取得 `real-agent` 層級。

#### K-01 create payload 能否指定模型

| 欄位 | 內容 |
|---|---|
| 使用者需求 | API／CLI／Tauri／UI 的 create payload 能否指定 model，並真的傳到 `SessionSpec.model` |
| 實作 owner | `crates/interaction-agent-gateway` ＋ `{agents,gateway}.rs` |
| 正式入口及呼叫路徑 | `POST /v1/agent-sessions`（`routes.rs:1389-1395`）／CLI `main.rs:377-405` → `commands.rs:915-940`／Tauri `lib.rs:1573-1577`／`TaskComposer.buildSessionCreateInput`（`:166-179`）→ `create_agent_session`（`agents.rs:387`）→ `gateway_attach`（`gateway.rs:285-339`）→ `connector.start_session(SessionSpec)` |
| 資料與協定契約 | `CreateAgentSession`（`agents.rs:245-278`）**沒有 model**，且未加 `deny_unknown_fields` → payload 帶 `model` 會被 serde 靜默丟棄。`SessionSpec.model`（`gateway lib.rs:166`）只有 connector 端消費（`claude.rs:103-105`、`codex_exec.rs:87-89,105-107`）；codex app-server 路徑連讀取端都沒有 |
| 狀態與儲存 owner | 無（沒有任何地方保存 requested model） |
| 現有測試 | `claude.rs:480-508`（不涉及 `--model`）；`gateway_loop.rs` 全檔 grep `--model` 0 命中；`scripts/v03-cli-e2e.sh:263` 無 `--model` |
| 證據 commit | `78dcda1` |
| 本次結果 | `absent`；`static-inspection`（懷疑者維持）。E0-02 的 `e2e-runs/R8/argv-claude/run.txt` 是真 claude 的 argv 直接證據，可用來確認 argv 內沒有 `--model` |
| 限制與下一動作 | 缺口位置：`CreateAgentSession` 沒有 model 欄位 → `gateway_attach` 沒把它寫進 `spec.model` → 兩個 connector 永遠用各自 CLI 的預設模型。`--model` 旗標是 defined-only 的死碼。0.153.4 schema 的 `ThreadStartParams` 含 `model`／`modelProvider`、`TurnStartParams` 含 `model`／`effort`——**協定支援但 connector 不送**。文件沒有宣稱可選模型（無文件／程式不一致）。下一動作：若要支援，`CreateAgentSession` 加 `model: Option<String>`、`gateway_attach` 寫入 `spec.model`、`codex.rs` 的 thread/start 與 thread/resume 加 `model`、CLI 加 `--model`、TaskComposer 加選單，並用 fixture argv 斷言旗標真的送出；同時把「不可選模型」寫進 known-limitations |

#### K-02 實際使用的模型能否被辨認並記錄

| 欄位 | 內容 |
|---|---|
| 使用者需求 | claude stream-json `system/init` 的 model、codex `thread/start` 回應或 turn 事件的 model，能否記到 session record／事件 |
| 實作 owner | `crates/interaction-agent-gateway`（三個 parser）＋ `core agent.rs`（AgentSessionRecord） |
| 正式入口及呼叫路徑 | connector stdout → `parse_claude_line`（`claude.rs:378-473`）／codex reader（`codex.rs:223-289`）／`parse_exec_line`（`codex_exec.rs:358-417`）→ `GatewayEvent` → `spawn_gateway_pump` → record／report |
| 資料與協定契約 | `GatewayEvent` 沒有任何 model 欄位（`gateway lib.rs:79-152`）；`AgentSessionRecord` 沒有 model 欄位（`core agent.rs:183-244`）；桌面 `api.ts:713-743` 亦無 |
| 狀態與儲存 owner | 無（沒有任何 model 落地點） |
| 現有測試 | `claude.rs:510-521`（樣本 init 行含 `"model":"claude-haiku-4-5"`，但只斷言 `SessionStarted{provider_session_id}`）；`fake_claude.sh:44` 也送 `"model":"fake-model"`，runtime 測試沒有任何斷言讀它 |
| 證據 commit | `78dcda1` |
| 本次結果 | `absent`；`static-inspection`＋`unit`（懷疑者維持）。**E0-02 的執行期補救**：本輪是靠 provider-local log 與 `raw.json` 才知道實際模型（`analysis/prior-runs-model-table.json` 記錄 claude 側 `claude-fable-5-1`、codex 側 `gpt-6-astra`＋effort medium），**不是靠 runtime**——record／事件／UI 全程沒有模型資訊。缺陷 **D20** 是同一類投影缺口 |
| 限制與下一動作 | requested 無（K-01）、actual unknown：連 connector 已經拿到手的資訊（claude `init.model`；codex `ThreadStartResponse.model`／`modelProvider`／`reasoningEffort`）也被丟掉，無法對帳費用與模型。下一動作：`GatewayEvent::SessionStarted` 加 `model: Option<String>`，`AgentSessionRecord` 加 `provider_model` 並持久化、投影到 `api.ts` 與 AiPage 進階詳情；fixture 已送 model，補斷言即可 |

#### K-03 推理設定（effort／reasoning）

| 欄位 | 內容 |
|---|---|
| 使用者需求 | 能否透過 create payload 傳到 connector（claude `--effort`；codex `turn/start.effort` 或 `-c model_reasoning_effort`） |
| 實作 owner | `crates/interaction-agent-gateway` ＋ `{agents,gateway}.rs` |
| 正式入口及呼叫路徑 | 同 K-01 |
| 資料與協定契約 | `SessionSpec`（`gateway lib.rs:156-180`）與 `CreateAgentSession`（`agents.rs:245-278`）都沒有 effort／reasoning 欄位；`claude_session_args` 不產生 `--effort`；`codex.rs` 的 thread/start 與 turn/start 不送 `effort`；`codex_exec.rs` 不送 `-c model_reasoning_effort` |
| 狀態與儲存 owner | 無 |
| 現有測試 | 無 |
| 證據 commit | `78dcda1` |
| 本次結果 | `absent`；`static-inspection`（懷疑者維持）。環境探測（唯讀）：Claude Code 2.1.263 的 `--help` 有 `--effort <level>`；0.153.4 schema 的 `TurnStartParams` 有 `effort`、`ThreadStartResponse` 有 `reasoningEffort`——**兩邊都支援，connector 都不用** |
| 限制與下一動作 | 沒有任何入口可設定推理強度；兩個 provider 各自吃使用者本機設定，而 codex exec fallback 帶 `--ignore-user-config`、app-server 未帶 `--strict-config`，**兩條路徑對使用者設定的繼承不一致**（與 **D5** 同源）。推理輸出也不投影（`codex.rs:579-581` 與 `codex_exec.rs:463` 直接丟棄）。下一動作：若要支援，`CreateAgentSession` 加 `effort` → `SessionSpec.effort` → 三個 connector；並決定是否把實際 `reasoningEffort` 記回 record（與 K-02 一併） |

#### K-04 Session 續接與原生授權保持

| 欄位 | 內容 |
|---|---|
| 使用者需求 | claude `--resume`、codex `thread/resume`、`exec resume`；續開不繼承寫入權，且沙箱／權限旗標重新上鎖 |
| 實作 owner | `agents.rs`（check_resume_not_wider／resumed_session_record）＋ `gateway.rs` ＋ 三個 connector |
| 正式入口及呼叫路徑 | `POST /v1/agent-sessions` 帶 `resumeProviderSessionId`（`routes.rs:1389-1395`）／CLI `agents create --resume` 與 `agents resume`（`main.rs:399-431` → `commands.rs:938`、`:943-1000`）／桌面「接續上次」（`AiPage.tsx:116-135`、`:692`）→ `agents.rs:441-468` → `gateway.rs:335-337` → `claude.rs:106-108 --resume`／`codex.rs:321-331 thread/resume{threadId,cwd,approvalPolicy,sandbox}`／`codex_exec.rs:76-90 exec resume …` |
| 資料與協定契約 | resume 必須找得到本 runtime 授權過的舊紀錄，並經 `check_resume_not_wider`（ttl／maxCost／maxMessages／toolScope／workdir 不得比原 session 寬）；`allow_write` 仍要再走三重檢查；codex 於 resume 重送 cwd／approvalPolicy／sandbox（`codex.rs:309-343`），claude 續開仍走 plan／acceptEdits 決定 |
| 狀態與儲存 owner | `AgentSessionRecord.provider_session_id`（由 SessionStarted 回填）與 `resolved_workdir`，持久化於 SQLite `agent_sessions` |
| 現有測試 | `gateway_loop.rs:842-890`（argv 含 `--resume fake-123` 且 `--permission-mode plan`）、`:1007-1095`（fake-thread-resume 檔斷言 threadId／sandbox=read-only／approvalPolicy=untrusted／cwd）、`:2172-2212`、`:2699-2745`；`agents.rs:1892-2030` unit；`codex.rs:907-988`；`commands.rs:1616-1680` unit。**CLI E2E 與 Playwright 無 resume 案例** |
| 證據 commit | `78dcda1` |
| 本次結果 | `implemented-and-connected`；`unit`＋`fixture`＋`integration`（懷疑者維持；repo 測試層級不含 real-agent）。**E0-02 的續接段＝completed（real-agent）**：claude argv 直接證據 `--resume <psid>`、provider log 續寫同一檔且 prompt 不含暗號；codex `thread/resume` 續寫同一 rollout 且 read-only／untrusted 重新上鎖（`e2e-runs/R8/{argv-claude/run.txt,codex/raw.json}`）。缺陷 **D9**（無稽核）、**D20**（record 不保存 resumeProviderSessionId） |
| 限制與下一動作 | 純對話 session 的 `resumeProviderSessionId` 不會被任何 connector 使用（`agents.rs:449-452` 誠實揭露）。§4 K-X5 裁定：本列 contract 提到的 `[--model]` 旗標**在目前任何入口都不會被觸發**（見 K-01），敘述已在此限定。下一動作：補 CLI E2E 與 Playwright 的 resume 覆蓋；為 resume guard 加 audit |

#### K-05 工具限制

| 欄位 | 內容 |
|---|---|
| 使用者需求 | claude `--tools`／`--permission-mode`；codex sandbox／approvalPolicy；intent-only（零工具）；絕不 bypass |
| 實作 owner | `crates/interaction-agent-gateway`（三個 args 產生器）＋ `gateway.rs`（gateway_attach 守門、tools_disabled） |
| 正式入口及呼叫路徑 | create payload `allowWrite`＋`toolScope`＋`consentScope`（`agents.rs:412-438` 三重檢查）→ `record.allow_write`／`tool_scope` → `gateway_attach`（`gateway.rs:297-334`）→ `claude.rs:87-102`／`codex.rs:315-343`／`codex_exec.rs:69-111` |
| 資料與協定契約 | claude 唯讀＝`--permission-mode plan --tools Read,Glob,Grep`；寫入＝`acceptEdits --tools Read,Glob,Grep,Edit,Write`（無 Bash／WebFetch／WebSearch）；intent-only＝`plan` ＋ `--tools ""`；一律 `--safe-mode --strict-mcp-config --mcp-config {"mcpServers":{}}`；**永不** `--dangerously-skip-permissions`。codex app-server：`sandbox` read-only／workspace-write ＋ `approvalPolicy: untrusted` |
| 狀態與儲存 owner | `AgentSessionRecord.allow_write`／`tool_scope`／`consent_scope`；生效旗標只在子程序 argv／JSON-RPC params |
| 現有測試 | `claude.rs:480-508`；`gateway_loop.rs:2650-2682`（codex 拒絕 intent-only 且不起子程序）、`:842-890`、`:1007-1095`、`:1160-1215`（逾時拒絕真的寫進子程序 stdin）；`codex.rs:907-988`（exec fallback argv 無 danger） |
| 證據 commit | `78dcda1` |
| 本次結果 | `implemented-and-connected`（**限縮：claude 側成立；codex 側因 D4／D5／D6 應視為缺關鍵環**——§4 K-X2 裁定，本文件把處置寫在本列而不改整列狀態，因為型別與強制鏈確實接線）；`unit`＋`fixture`＋`integration`。**E0-10＝correctly-blocked／completed**：真 claude 唯讀 session 連 Write 工具都沒給、`permissionMode=plan`；授權寫入時 `acceptEdits` 且檔案落地（`e2e-runs/R8/{guard-claude,guard-codex}/run.txt`、`e2e-runs/R4/{b-claude,c-claude-write}/`）。**但執行期揭露三個缺陷**：**D4**（deny 值不合法）、**D5**（codex 無 MCP／plugin 封鎖，唯讀 session 仍啟動使用者的 MCP server 與 helper）、**D6**（`writable_roots` 併入使用者全域設定，超出 session 授權的 resolvedWorkdir） |
| 限制與下一動作 | codex sandbox 擋寫不擋讀檔／shell，唯讀 session 仍可讀整個 workdir（程式註解誠實揭露）；app-server 路徑不帶 `--strict-config`／`-c`，會繼承使用者 `~/.codex/config.toml`，與 exec fallback 的 `--ignore-user-config` 不一致。下一動作（依序）：修 D5（app-server 啟動加 `--strict-config`／`-c mcp_servers={}`）→ 修 D4（`"reject"`→`"decline"`）→ 修 D6（送 `writable_roots`）→ 補 known-limitation 說明「codex 唯讀 ≠ 不可讀」 |

#### K-06 connector 參數與已安裝版本的相容性（**confirmed-defect**）

| 欄位 | 內容 |
|---|---|
| 使用者需求 | 對照 Claude Code 2.1.263 與 Codex 0.153.4：CLI 旗標、app-server JSON-RPC 方法／欄位／enum |
| 實作 owner | `crates/interaction-agent-gateway`（`claude.rs`／`codex.rs`／`codex_exec.rs`） |
| 正式入口及呼叫路徑 | `connector.discover()`（`claude.rs:124-154`：`claude --version`＋`claude auth status`；`codex.rs:84-133`：`codex --version`＋`codex login status`＋`app-server --help`＋`exec --help`）→ `refresh_agent_providers`（`gateway.rs:189-225`）→ `start_session` |
| 資料與協定契約 | claude 旗標清單（`claude.rs:74-116`）；codex app-server `initialize` → `initialized` → `thread/start`／`thread/resume` → `turn/start`／`turn/interrupt`；approval 回應 `{decision}`（`codex.rs:623-660`） |
| 狀態與儲存 owner | `gateway.discoveries`（記憶體暫存版本／登入）；**無版本相容性檢查邏輯** |
| 現有測試 | `claude.rs:480-508`（旗標 unit，不對照真 CLI）；`codex.rs:715-903`；**`gateway_loop.rs:1195-1209` 斷言 fixture 收到的 decision 行 `contains("reject")`——測試把不合法的值固化成期望值**；`fake_codex.sh:74-76` 對任何含 `decision` 的行照單全收，無 enum 檢查 |
| 證據 commit | `78dcda1` |
| 本次結果 | **`confirmed-defect`**（懷疑者維持）；`static-inspection`（＋唯讀 environment-probe）。**確認缺陷**：`codex.rs:643-644` `ApprovalDecision::Deny => "reject"`（本文件已現場複核 grep），而 0.153.4 的 `CommandExecutionApprovalDecision` oneOf ＝ `accept`／`acceptForSession`／物件型 amendment／`decline`／`cancel`。**E0-10 執行期確認後果**：codex 的人類「拒絕」在 agent 端變成核可機制錯誤而非語意上的拒絕（缺陷 **D4**）；安全結果靠 provider fail-closed，不是靠我們送對值。相關 **D7**（ServerRequest 一律當核可）、**D16**（stderr 丟棄使協定層錯誤無痕跡） |
| 限制與下一動作 | 相容 OK 的部分（唯讀 `--help` 對照）：claude 的 `-p`／`--input-format`／`--output-format`／`--verbose`／`--safe-mode`／`--strict-mcp-config`／`--mcp-config`／`--permission-mode`／`--tools`／`--model`／`--resume`／`--max-budget-usd` 全部存在；codex `exec`／`exec resume`／`app-server` 子命令與 schema 形狀相符。**風險**：2.1.263 的 `--help` **沒有** `--max-turns`，而 `claude.rs:109-111` 在 `spec.max_turns` 有值時會送出（目前是死碼）。**needs-investigation**：`codex.rs:250-266` 把任何帶 id 的 method 都登記成 approval（0.153.4 的 ServerRequest 清單包含 `item/tool/requestUserInput`、`mcpServer/elicitation/request`、`attestation/generate` 等回應形狀不同的方法）；`item type toolCall` 不在 0.153.4 的 ThreadItem 清單。**版本認知漂移**：`codex.rs:8` 註解說鎖 0.149.1、`CHANGELOG.md:1479` 說 0.150.1、對抗審查文件用 0.151.0、實機是 0.153.4，repo 內沒有任何 schema 副本。下一動作：(1) `"reject"`→`"decline"` 並依 method 分派回應形狀，同步改 `gateway_loop.rs:1199` 的斷言；(2) 把 0.153.4 schema 摘要落成 repo 內 golden 並加對照測試；(3) 刪除或守住 `--max-turns` 死碼；(4) 保留 codex stderr tail 到 session detail（D16） |

#### K-07 token／費用回報

| 欄位 | 內容 |
|---|---|
| 使用者需求 | `TokenUsage`、`cost_usd` 的來源、落地位置、預算強制與 UI 顯示 |
| 實作 owner | 三個 parser ＋ `gateway.rs`（pump、add_agent_session_cost、預算）＋ `AiPage.tsx` |
| 正式入口及呼叫路徑 | claude `result.total_cost_usd`／`num_turns`（`claude.rs:442-466`）→ `TaskClaimedCompleted` → `gateway.rs:511-556`：`add_agent_session_cost`（`:650-656`，record.budget.spent_cost 累加並 persist）＋mailbox `result{summary,costUsd}`＋report。codex `thread/tokenUsage/updated`（`codex.rs:564-577`）／exec `turn.completed.usage` → `TokenUsage` → `gateway.rs:486-509`（本輪未結局則 report progress；已結局則只 ingest）。UI `AiPage.tsx:562-572`（**只有進階模式**顯示「費用 $spent/$max」） |
| 資料與協定契約 | `SessionBudget{max_cost,spent_cost,max_messages,spent_messages,max_duration_ms}`；claude 以 `--max-budget-usd` 交給 CLI（**僅 `max_cost>0` 才送**）＋runtime 於 spent≥max 時拒開新 turn；codex 在建立時直接拒絕 maxCost |
| 狀態與儲存 owner | `record.budget.spent_cost`（SQLite）；**token 數不落 record**，只在 report payload → observation 與 SSE 事件 |
| 現有測試 | `claude.rs:530-543`；`codex.rs:860-892`（token usage 不冒充 USD）；`gateway_loop.rs:294-297`；`proactive_loop.rs:430`。桌面 grep `tokenUsage`／`totalTokens` ＝ 0 命中；e2e grep `spentCost`／`costUsd` ＝ 0 命中 |
| 證據 commit | `78dcda1` |
| 本次結果 | `implemented-partial`；`unit`＋`fixture`＋`integration`（懷疑者維持）。**E0-02 執行期數字**：真 claude `spentCost=0.14471075`（`analysis/prior-runs-model-table.json`）；真 codex `gatewaySpentCost=0.0` 但 provider 端有 `total_token_usage`——**兩者無法對帳同一口徑** |
| 限制與下一動作 | (1) claude 只有 USD 沒有 token 數、codex 只有 token 數沒有 USD；(2) token 數不持久化，UI 與 `agents show` 都看不到；(3) cost 只在 claude `result` 出現時累加，被 kill／unknown 收場的 turn 費用不入帳（誠實但低估）；(4) exec fallback 的 total_tokens 定義是否與 app-server 一致未驗。§4 K-X6 裁定：本列混了「落地」與「使用者可見」兩個需求——**「token／費用對一般模式使用者可見」實際上是 `absent`**。§4 K-X9 裁定：CLI 省略 `--max-cost` 時 payload 送 null → `agents.rs:528 unwrap_or(0.0)` → **預設完全不送預算旗標**，「claude 有可強制預算」在預設路徑上不成立。下一動作：把 TokenUsage 累加進 `SessionBudget` 並持久化；一般模式呈現用量；決定 codex 是否以 token 上限替代 maxCost |

#### K-08 執行時間記錄

| 欄位 | 內容 |
|---|---|
| 使用者需求 | session record 是否有 started／finished 時戳、turn 時長；能否從事件重建 |
| 實作 owner | `core agent.rs`（AgentSessionRecord）＋ `agents.rs`（create／close／report）＋ `crates/interaction-events` |
| 正式入口及呼叫路徑 | `create_agent_session`（`agents.rs:503-544`：`created_at`、lease `issued_at`／`expires_at`）→ `report_agent_session`（`:1174-1261`：**只改 state，不寫時戳**）→ close／expire（`:679`、`:1312`：`closed_at`）；事件 `events.rs:85-86` 以 `Utc::now()` 蓋章 → SSE／Timeline |
| 資料與協定契約 | `AgentSessionRecord` 只有 `created_at`／`closed_at`（`core agent.rs:207-209`）＋lease 時戳＋`budget.max_duration_ms`；**沒有** task started_at／finished_at、turn 時長、累計工作時間。`MailboxMessage` 有 `created_at`／`delivered_at` |
| 狀態與儲存 owner | `record.created_at`／`closed_at`（SQLite）；狀態轉換時刻只在事件日誌與 observation |
| 現有測試 | `agents_loop.rs`／`gateway_loop.rs` 以 `wait_for` 觀察狀態轉換，**未見對 `created_at`／`closed_at` 值的直接斷言**；無任何測試斷言 turn 時長 |
| 證據 commit | `78dcda1` |
| 本次結果 | `implemented-partial`；`static-inspection`＋`fixture`（懷疑者維持；§4 K-X1 記錄了同一份證據在 K 維度被標成三種層級的不一致，建議統一為「runtime tests ＝ fixture＋integration；connector 內 `#[test]` ＝ unit」）。**E0-03 的時間斷言受環境影響**：R1 五次 claude 執行從 task-sent 到 active 一律約 61–65 s（見 §4 環境阻礙），本輪無法判斷這是產品層問題還是額度／冷啟動 |
| 限制與下一動作 | 無法從 record 直接回答「這個任務跑了多久／何時開始工作」，只能從有界的事件日誌推算；connector 已收到的 turn 時長被丟棄（0.153.4 的 `Turn` 有 `startedAt`／`completedAt`／`durationMs`，`codex.rs:510-553` 只讀 status／error）。與 **D3** 同源：`report_agent_session` 從不寫 `entry.record.detail`。下一動作：record 加 `first_working_at`／`last_report_at`／`settled_at`；`TaskClaimedCompleted` 加 `duration_ms`；AiPage 進階詳情顯示；補 fixture 斷言時戳單調 |

---

## 3. 懷疑者改判一覽

每一列都由一個獨立懷疑者逐條反駁（67 列 / 67 份裁決）。整體結果：

| 類別 | 數量 | 列 |
|---|---|---|
| `refuted = true`（列內有事實錯誤） | 1 | A-08a |
| `refuted = false` 但 `correctedStatus` 與原判不同 | 1 | B-07 |
| `correctedEvidenceLevel` 與原列不同 | 0 | — |
| 其餘（逐條核對後維持原判） | 65 | — |

### 3.1 A-08a：原判 `implemented-and-connected` → 改判 `implemented-and-connected`（狀態不變，證據更正）

懷疑者 `refuted=true` 的理由：該列的 evidence 寫「SQLite 無 schema 版本遷移框架跡象（`CREATE TABLE IF NOT EXISTS`）」是**事實錯誤**——
`crates/interaction-storage/src/lib.rs:76` 的 `migrate()` 讀 `PRAGMA user_version`，以 `if version < N`（N=1..8）逐步套用，
結束後把 `CURRENT_SCHEMA`（`:18`，值為 8）寫回 `user_version`。本文件已現場複核（grep 命中 `:18`、`:76`、`:79`、`:81`、`:282`）。
整體狀態分類仍正確（重啟恢復語意確實接線且有 `agents_loop.rs:491`／`:812` 釘住），故**狀態不變**；
但該列 limitation 的「WAL 檔與 config 目錄的搬遷／升級沒有工具」必須限定為 **config YAML 目錄**，
SQLite 側已有版本化 migration（目前 8 個步驟都只新增資料表／索引，**尚未驗證真正的欄位遷移**）。
本矩陣的 A-08a 已照此改寫。

### 3.2 B-07：原判 `implemented-partial` → **改判 `confirmed-defect`**

理由摘要（懷疑者 `correctedStatus`，本文件採納並補上執行期證據）：

1. `claude.rs:354-356` 的 `interrupt()` 只對 process group 送 SIGINT，檔內 `grep -c TaskCancelled` ＝ **0**，也沒有任何
   `cancel_requested`／`interrupt_requested` 旗標——對照 `codex_exec.rs`（約 `:308-329`）明確記錄取消旗標並產生 `TaskCancelled`。
2. 後果可端到端追出：被 SIGINT 的 claude 程序退出時沒有 claim，`gateway.rs:607-633` 的 `SessionClosed` 報 `unknown`（不是 cancelled），
   桌面對 unknown 的文案是「既不是成功也不是失敗」（`workState.ts:161-166`）。
3. **測試覆蓋為零**：`gateway_loop.rs` 裡 `gateway_interrupt` 只出現在 `:1805`、`:1900`，兩處都是 codex；
   `scripts/tests/tauri-work-cancel.py` 自己的註解說明它對 Claude 只測 close／hang；`human.rs:139` 讓 programming 任務預設路由 codex，
   所以既有的通過測試沒有一支走過這條路。
4. **執行期比靜態更糟**：本輪 E0-03 的真 claude interrupt 5/5 都收在 `failed`（不是靜態預測的 `unknown`），
   見 `e2e-runs/R1/claude-interrupt/{result.json,sse.jsonl}`、`R1/b07-claude/result.json` 與另外三次前一輪重現。
   終態分類以**執行期為準**（見 §4 的裁定 X-新1）。

### 3.3 其餘 65 列

懷疑者逐一重開所有 file:line 引用，回報「精確或漂移數行內」，並確認狀態分類與證據層級沒有過度宣稱。
逐列裁決全文在 `evidence/2026-09-07-phase-0/static-matrix/dim-{A,B,C,D,E,F,G,K}.json` 的 `verdicts` 陣列。

---

## 4. 矛盾（completeness critic 的 contradictions）與裁定

critic 對八個維度共提出 **82 條** contradictions（A 10、B 10、C 12、D 10、E 11、F 10、G 10、K 9）。
下表只列**本文件做出裁定**的項目；其餘已在 §2 的對應列以「§4 X-XN 裁定／記錄」就地引用。
裁定分三種：`已裁定`（能從來源或現場複核判定，本矩陣已照此改寫）、`已記錄`（爭議屬實但需產品決策，寫進該列的限制）、
`open`（現有證據不足以判定，不猜）。

| 編號 | 爭議 | 裁定 |
|---|---|---|
| **X-新1** | 靜態盤點（A-07／B-07／C-04）預測 claude interrupt 的終態是 `unknown`；本輪執行期是 `failed` | **已裁定：以執行期為準**。E0-03 五次重現皆 `failed`（`e2e-runs/R1/claude-interrupt/result.json`）。原因在 `claude.rs:442-455`：`result` 的 subtype 以 `error` 開頭 → `TaskFailed`，且 `:213-231` 的 `saw_result` 讓 `:267-283` 的誠實退路失效。矩陣已改寫 |
| A-X1 | A-05／A-07 寫「`tauri-work-cancel.py` 存在但找不到本版執行紀錄，故不標 native-desktop」，但 v0.8.0 的原生證據就在 repo 內 | **已裁定：critic 正確**。現場複核 `docs/releases/evidence/2026-09-06-convergence/final/native-final-checkpoint/runs.json` 有 `work-clean`（exit 0、52.936 s）與 `work-legacy`，產物含 `codex-turn-cancelled-ax.txt`、`claude-long-work-closed-and-processes-exited-ax.txt`。A-05／A-07 已補 `native-desktop`，並標明是 2026-09-06 bundle＋fixture agent |
| A-X2 | `agent_session_renew` 在桌面兩條 transport 都不存在 | **已裁定：屬實（現場複核）**。`grep agent_session_renew apps/interaction-desktop/src/transport.ts apps/interaction-desktop/src-tauri/src/lib.rs` 零命中；而 `api.ts:374` 會 invoke 它、`AiPage.tsx:634` 會呼叫 `api.agentSessionRenew`；後端 `routes.rs:1527`、`agents.rs:826`、CLI `commands.rs:1033` 都在。結論：**續租按鈕在 http 與 tauri 兩種模式都會失敗，是桌面單邊斷線**。既有列沒有涵蓋此需求 → 見 §5.1 的 A-15（標為未逐列核實，但本條的 grep 已由本文件複核） |
| A-X3 | `docs/capability-completion-matrix.md` 的 UI-AI-001 把「續租」標為完成 | **已裁定：與 A-X2 直接衝突**。該檔已在本輪加上「現況以本矩陣為準」的指引行 |
| A-X4 | `docs/INSTALL.md` 把 Windows x64 CLI 標為可用，但 `get.sh` 沒有 Windows 分支 | **已裁定：升級為文件缺陷**（不只是 A-01 的限制）。`release.yml:242` 把 `get.sh` 原樣複製成 `install.sh`，所以 Windows 上沒有任何被文件描述過的 CLI 安裝路徑 |
| A-X5 | A-08a 的 SQLite migration 敘述錯誤 | **已裁定：見 §3.1，已改寫** |
| A-X6 | A-05（成果在 mailbox）與 A-08a（重啟後狀態誠實保留）合起來給人「重啟後還看得到成果」的印象 | **已裁定：屬實**。mailbox 是記憶體結構，restore 以空 mailbox 重建；重啟後 claimed-completed 的工作顯示「對方說已完成」＋「Agent 尚未回報任何結果。」，**而且仍可按「標記為已驗證」**（verify 只檢查 claim_id）。已寫進 A-08a 的限制 |
| A-X7 | 「等你回答」的假承諾還有兩個未標註出處：`DESKTOP-GUIDE.md:181,289`、CLI `main.rs:428-433` 的 `--kind` help | **已裁定：屬實**，已寫進 A-05a 與 G-03 |
| A-X8 | 重啟後 Activity Inbox 裡「等你允許」的待決定項會直接消失，沒有任何說明 | **open**：需實跑確認 inbox 在該情境的輸出，本輪未測。標為 `needs-investigation`（不新增矩陣列） |
| A-X9 | A-01 的 limitation 說「沒有安裝流程的自動化測試」，但同列已列出 `release-scripts.sh:262-321` | **已裁定：措辭過強**。應為「沒有端到端安裝測試；有腳本層 fail-closed 測試」。A-01 已改寫 |
| A-X10 | estop 嚴格要求未確認動器一律列 unconfirmed，但桌面 `lib.rs:2158-2170` 把 autostart `enable()`／`disable()` 的 Result 用 `let _ =` 丟掉 | **已記錄**：同一個 dispatched≠confirmed 不變量在兩處採用不同標準。屬 §5.1 的 A-11，本輪未逐列核實 |
| B-X1 | B-04 把整個「agent 提問／人類回答」寫成 defined-only，但 waiting-for-**consent** 有真實 connector 產生者與 UI | **已裁定：B-04 的結論限定於 waiting-for-input**，已在該列改寫；核可往返歸 A-06 與 §5.1 的 B-M01 |
| B-X2 | B-04 引用的文件出處記錯，懷疑者又想整條刪掉 | **已裁定：矛盾為真、出處要改**。原文在 `crates/interaction-cli/src/main.rs:446` 與 `skills/…/references/api.md:104`，不是 `references/cli.md:127`。已在 B-04 改寫 |
| B-X3 / B-X7 | 「daemon 存活時可完整找回」不成立：close 之後信箱已清空；卡片收合時不輪詢 | **已裁定：屬實**。`agents.rs:1321` 只保留有 `delivered_at` 的信件，而 from-session 訊息永遠沒有戳記。已寫進 B-05 |
| B-X4 | B-02 的「串流文字沒有任何 UI／SSE 可見面」過強 | **已裁定：應限定為「桌面與 SSE 看不到」**——`POST /v1/observations/query` 與 CLI `observe` 是正式入口。同時 critic 指出 `/v1/observations/query` 在 agent token 白名單內，A agent 讀得到 B agent 的過程文字 → **open**（跨 session 文字隔離零測試，本輪未驗） |
| B-X5 | interrupt 的語意在 trait 註解、CLI help、codex 實作、claude 實作四處互斥 | **已裁定：屬實**，已寫進 B-07 與 A-07 |
| B-X6 | B-01 判 connected 但同樣缺兩個關鍵環，B-02／B-06 以同等程度判 partial | **已記錄（尺度爭議，未改判）**：B-01 的兩個缺口（model、domains）都有獨立列（A-05b／K-01；D-09／E-10）承接，B-02／B-06 的缺口沒有。維持原判並在 B-01 明列 |
| B-X8 | 「再交代一句」沒有 in-flight 保護，連按兩下產生兩則訊息、兩次 deliver、兩格預算 | **已裁定：屬實**，已寫進 B-06 |
| B-X9 | B-03 的 evidenceLevel 同時列 unit／fixture 與 not-run | **已裁定：語意需說明**——unit／fixture 覆蓋 parser，not-run 指投影路徑（fixture 不輸出工具事件）。已在 B-03 註明。另 codex app-server **有** ToolCompleted，缺的是 `fileChange` 不帶 path，已改寫 |
| B-X10 | 訊息預算的語意在 CLI help、契約敘述、實作三處不一致 | **已記錄**：實作對 from-session 訊息照扣，預算用盡時 pump 的錯誤被 `let _ =` 吞掉。屬 §5.1 的 B-M04 |
| C-X1 / C-X2 | C-02 與 C-07 對「共用 no-folder 目錄」給出不同狀態；C-02 的 limitation 自承缺投遞隔離測試 | **已裁定：本輪執行期補上了關鍵環**——E0-06／E0-07 以真 agent 驗證了投遞隔離與取消獨立性，故 C-02 維持 `implemented-and-connected`；共用目錄的缺口由 C-07（partial）承接。已在兩列交叉引用 |
| C-X3 | C-04（claude interrupt→unknown）與 C-08（取消 connected）不一致 | **已裁定：不衝突**。C-08 的需求是「取消不外溢到其他 session」（E0-07 已驗），claude interrupt 的**分類錯誤**歸 B-07／D1。已在 C-08 註明 |
| C-X4 | C-05 evidence 自相矛盾（有 envelope 時是否仍檢查 max_sessions） | **已裁定：以 `check_delegation` 為準**（`core agent.rs:171-176` 仍強制 max_sessions）。已在 C-05 改寫 |
| C-X5 | `docs/MAINTAINERS-MAP.md` 在一列被判漂移、在另一列被當權威 | **已裁定：`:100` 的 policy crate 歸屬確為錯誤**（policy crate 無 agent／delegation 程式碼），`:98` 的 owner 對照仍可用。已在 C-06 列為下一動作 |
| C-X6 | C-09 判 connected，但 Tauri 把 error code 壓成字串 | **已記錄（尺度爭議，未改判）**：錯誤碼在 HTTP／CLI 完整且有測試，Tauri 是投影層缺口，已在 C-09 明列 |
| C-X7 | C-10 說「沒有測試覆蓋雙重續開」，裁決說既有測試就示範了 | **已裁定：採裁決版本**——`gateway_loop.rs:1044-1095` 這支通過的測試示範了「原 session 仍 open 時再續開同一 thread」。已在 C-10 改寫 |
| C-X8 | C-09 的「明確拒絕」與 C-01 的「providerId 不驗證、未知 agentId 不拒絕」並存 | **已裁定：屬實**，已在 C-09 與 C-01 交叉引用 |
| C-X9 | 建立被序列化＋每次 attach 重新 discover（每 connector 上限 6 s），使用者感受到序列建立＋延遲累加，無人記為需求 | **已記錄**，寫進 C-03；本輪 E0-03 觀察到的 61–65 s 冷啟動有部分可能來自此，但**未證實**（見環境阻礙） |
| C-X10 | maxCost 在三種 agent 上分別是強制／明確拒絕／靜默無效 | **已裁定：屬實**，已寫進 C-06 與 K-07 |
| C-X11 | `skills/…/references/api.md:101-108` 把 `/report` 與 `/messages` 列成 agent 可用端點，但守門讓 agent／session token 都送不出 | **已裁定：屬實**，與 B-X2 同源，已寫進 B-04 |
| C-X12 | 現場執行 `codex --version`／`claude --version` 得到的版本事實被當成 repo 證據 | **已裁定：應標為 environment-probe**。本矩陣在 2.K 開頭已明確區隔 |
| D-X1 | D-04 把 knowledge pull 當現存替代方案，但沒有任何正式入口能設定 knowledge 類 tool_scope | **已裁定：屬實**——桌面只產生 `[]` 或 `["workspace.write"]`，CLI 無旗標，空 tool_scope 在 `allows_tool` 下對每個工具都回 false → 403。已寫進 D-04 與 E-03；缺口列見 §5.1 的 D-09／E-10 |
| D-X2 | D-03 標 connected、D-06 對同一段程式標 confirmed-defect | **已裁定：兩者都成立但必須互相限定**。D-03 已加註「必須與 D-06 一起讀」 |
| D-X3 | D-06 的觸發條件被低估 | **已裁定：屬實**。`AgentHandoff`／`TaskMemory`／`WorldKnowledge` 不在 domain 過濾名單，不需任何 domain 授權就會進 bundle，「桌面建立、domains=[]」同樣可觸發。已寫進 D-06 |
| D-X4 | D-01 的 evidenceLevel 列了 browser，但引用的 e2e 自己註明不涵蓋路由建議 | **已裁定：移除 `browser`**。D-01 已改 |
| D-X5 | `gateway.rs:1189` 的 note 宣稱「建立 session 前會顯示資料範圍與成本預覽」，但 TaskComposer 的預覽不含 bundle 會送出的記憶內容 | **已記錄**：程式自述與同意介面不一致，屬 §5.1 的 D-14 |
| D-X6 | `docs/FEATURES.md:141` 把 Context Bundle 說成「本次提供了哪些」，暗示事後可查，實際桌面完全不顯示收據 | **已裁定：屬實**，已寫進 D-03 與 G-05 |
| D-X7 | 主動說話 session 宣告 intent-only 且 prompt 明寫「不讀檔、不使用工具」，但訊息走 `kind="task"`，仍會被自動附上完整 bundle | **已裁定：屬實（宣告 vs 實際不一致）**，屬 §5.1 的 D-17；本輪未實跑確認 bundle 內容 → 處置優先度待定 |
| D-X8 | `validate_handoff` 自稱 bounded，但長度檢查漏掉 `inferences[].text` 與 `artifacts[].uri`；落記憶時被 8 KB 上限擋下而錯誤被 `let _ =` 吞掉 | **已記錄**，屬 §5.1 的 D-07 |
| D-X9 | `content_hash` 含 `generatedAt`，同樣內容每次 hash 都不同 | **已裁定：屬實**，已寫進 D-03 |
| D-X10 | fixture `fake_claude.sh:45` 的 init 事件帶 `"model":"fake-model"`，任何以 fixture 觀察「有沒有選到模型」的驗收都會假陽性 | **已裁定：屬實**，已在 K-02 註明（fixture 已送 model，補斷言即可） |
| E-X1 | 48 KiB 預算對 domain pack 與記憶項用兩把尺 | **已裁定：屬實**，已寫進 E-04 |
| E-X2 | E-04 的 repro 前提在出貨入口做不到 | **已裁定：缺陷仍成立**（D-06 的無 domain 路徑可觸發），但描述已限定。已寫進 E-04 |
| E-X3 | E-03 的權限分層語氣過強：對出貨路徑而言是「全關」而非「分權」 | **已裁定：屬實**，已寫進 D-04／E-03，缺口列見 §5.1 的 E-10 |
| E-X4 | 同一條刪除不變量在 E-03（partial）與 E-09（confirmed-defect）得到兩種嚴重度 | **已裁定：E-09 保留為獨立 confirmed-defect**（副本殘留是可指出的錯誤路徑），E-03 維持 partial 並交叉引用。E0-05 的執行期證據支持 E-09 的獨立性 |
| E-X5 | `api lib.rs` 的白名單只約束 HTTP 外部呼叫者，桌面 embedded 與 CLI 不走那道閘 | **已裁定：屬實**，已寫進 E-03 |
| E-X6 | `PRAGMA foreign_keys=ON` 是純裝飾（schema 無任何 FK 子句） | **已裁定：屬實**，已寫進 E-02 |
| E-X7 | E-06 的 browser 標記偏寬 | **已記錄（未改層級）**：懷疑者也註為「slightly generous」，但 `evidence.spec.ts:517-533` 確實在真 daemon 下建立 candidate 並驗 Inbox。已在 E-06 明註其實際涵蓋範圍 |
| E-X8 | 「AI 記得對這次任務有用的事」在資料一多就會確定性失效，沒有任何一列指出 | **已裁定：屬實**，已寫進 E-05 |
| E-X9 | E-08 說「repo 內無法驗證 CLAUDE.md 是否被讀」，但 repo 內就有 provider-local log 的既定程序 | **已裁定：程序確實存在**（`scripts/tests/phase0/README.md`），本輪 E0-05 也用它取得 E-09 的關鍵證據。E-08 的下一動作已改寫為「用該程序實測」 |
| E-X10 | `state/character-session.json` 是 Runtime 側第二份持久化狀態，無保存期限、無刪除入口，沒有任何一列覆蓋 | **已記錄**，寫進 E-01；獨立需求見 §5.1 的 E-20 |
| E-X11 | `memory_export` 的 notIncluded 四類資料**在任何地方都拿不到**，不只是這個端點不含 | **已裁定：屬實**，已寫進 A-08 與 E-01 |
| F-X1 | F-03 的「沒有通知推播」措辭會被讀成「手機收不到通知」 | **已裁定：需區分**——`iphone.notify`／`iphone.tts` 動器存在且有 plan／act 入口，缺的是「工作事件自動推到手機」的產生者。已寫進 F-03 |
| F-X2 | F-05 的 `native-desktop` 來自 fixture 驅動的桌面 AX 走查，對 iOS 生命週期不提供證據 | **已裁定：標籤保留但語意限定於桌面側**，已在 F-05 註明 |
| F-X3 | F-06 的「位址可能已變更」文案：盤點者判 confirmed-defect 候選，懷疑者判 disclosed limitation | **open**：雙方都承認未實跑。本文件維持 `implemented-partial`，並在 F-06 記下判別方式（§5.2 的 F-R12：關掉 daemon、觀察約 15 秒後的 PairingView 文案） |
| F-X4 | 同一句「請重新配對」在 F-01 被寫成正確限制、在 F-06 被寫成多餘動作 | **已裁定：兩者都成立**，關鍵是手機端無法分辨兩種情境。已寫進 F-06 |
| F-X5 | F-05 說「AIP heartbeat 未實作」，但同列引用的測試名說 iOS 會回應 host 的 AIP heartbeat | **已裁定：措辭需精確**——iOS 會回應（以 legacy status 回），只是不主動送。已在 F-05 改寫 |
| F-X6 | F-01 含 real-iphone、F-02 不含，矩陣讀者只看陣列會誤解 | **已裁定：以本節開頭的統一聲明處理**（F 維度所有 real-iphone 標記都來自 v0.5.0 建置，本階段 0 筆） |
| F-X7 | F-04 的「手機完全不是核准／回答面」會被讀成「只能看不能動」 | **已裁定：需限定**——手機有一條限於角色呈現的 member→host 控制回饋通道（`Result{observed}`／`cancelConfirmed`）。已寫進 F-04 |
| F-X8 | F-05／F-06 把 location 當成真的感測，但 `SensorCenter.swift` 從不呼叫 `startUpdatingLocation`、runtime 沒有 `iphone.location` 受器 | **已裁定：屬實**，已寫進 F-05；獨立需求見 §5.1 的 F-09 |
| F-X9 | F-01 在 connected 的列裡塞了一個未追到的問題（tray／原生路徑是否用 `rt()`-only 的 mobile IPC） | **已裁定：外顯為 needs-investigation**，已寫進 F-01 |
| F-X10 | 六列沒有一列提到 BLE；`fake_iphone.rs:219` 永遠回報 `bleGateway:false`，連 scan 都零覆蓋 | **已記錄**，屬 §5.1 的 F-12 |
| G-X1 | G-02 的「同一真相」結論比證據大（動作收據層 UI 與角色不同源） | **已裁定：限縮為 agent session 層**，已在 G-02 改寫 |
| G-X2 | G-03 想把 `DESKTOP-GUIDE.md:181` 對齊 `:289-291`，但後者同樣列了不存在的「等你回答」 | **已裁定：必須先採 G-01／A-05a 的處置再談文案對齊**，已在 G-03 改寫 |
| G-X3 | G-05 引用 `api_e2e.rs:475-501` 作為 bundle／收據證據不成立 | **已裁定：屬實**，G-05 已改引 `agents_loop.rs:309-383` 等 |
| G-X4 | G-04（一般模式直出英文原文）與 G-06 證據強度相同卻差兩級 | **已記錄（尺度爭議，未改判）**：G-06 有明確的**視覺語法錯誤**（綠色成功）與三份互斥定義，G-04 是**未守門的漏網文字**。兩者都已在各自列明列缺口 |
| G-X5 | G-01 與 G-05 把同一缺陷計兩次並開出兩種藥方 | **已裁定：是同一個決策點**，已在兩列互相引用 |
| G-X6 | G-06 把 `HomePage.tsx:787` 的 `!s.closedAt` 判準寫成「未逐行核實」，懷疑者已確認為真 | **已裁定：採懷疑者版本**，已升為 G-06 的處置項 |
| G-X7 | ai-assist 的缺口比 G-01 描述更大（`aiAssistsList` 也沒有非測試呼叫端） | **已裁定：屬實**，已寫進 G-01 |
| G-X8 | G-01／G-06 的 `native-desktop` 對不到該列核心需求 | **已記錄（標籤保留）**：已在 G-01 註明爭議；本矩陣的判準是「該列引用的測試檔實際存在的層級」，不是「該層級證明了整列需求」 |
| G-X9 | `docs/USER-GUIDE.md:23`／`docs/FEATURES.md:147` 的對外承諾與 G-01／G-05 的發現相衝突，且不在既有限制清單裡 | **已裁定：屬實（文件缺陷）**，已寫進 G-05 的下一動作 |
| G-X10 | G-02 把 iOS 列為 owner 容易讀成「iPhone 也呈現工作真相」 | **已裁定：已在 G-02 註明**，iPhone 工作面見 §5.1 的 G-09（absent） |
| K-X1 | 同一份 runtime 測試在 K 維度被標成三種證據層級 | **已裁定：建議統一為「runtime tests ＝ fixture＋integration；connector 內 `#[test]` ＝ unit」**，已在 K-08 記下；本輪不追溯改寫既有標記，以免與懷疑者已核可的層級脫節 |
| K-X2 | K-05 判 connected，但強制鏈的「拒絕」腿被 K-06 判 confirmed-defect | **已裁定：狀態不改，但在 K-05 明確限縮**（claude 側成立；codex 側因 D4／D5／D6 缺關鍵環），並把處置順序寫進該列 |
| K-X3 | K-06 把「任何帶 id 的 method 都當 approval」標成 needs-investigation，但既有 schema 已足以判定回應形狀不合法 | **已裁定：證據強度確實高於標籤**，已在 K-06 把它與 `"reject"` 一起列為需處置項（runtime 後果仍待實測） |
| K-X4 | `codex app-server --help` 的子命令清單與任務簡報的既知事實不一致 | **已裁定：以實跑為準**（`generate-json-schema` 確實可用且本輪產出過 schema），簡報那條既知事實過時 |
| K-X5 | K-04 的 contract 把 `[--model]` 寫進「已接線的續開契約」 | **已裁定：需限定**，已在 K-04 註明該旗標在任何入口都不會被觸發 |
| K-X6 | K-07 的 need 混了「落地」與「UI 顯示」兩個需求 | **已裁定：屬實**——「token／費用對一般模式使用者可見」是 `absent`，已寫進 K-07 |
| K-X7 | K-01／K-02／K-03／K-06 至今沒有出現在任何已發布的限制清單 | **已裁定：屬實（文件缺口）**。`docs/releases/v0.8.0-known-limitations.md` 沒有 agent connector 條目，只以「其餘 v0.7.0 限制仍保留」概括。下一動作：把這四項寫進限制清單 |
| K-X8 | `gateway.rs:1192` 的文案宣稱「建立 session 前會顯示資料範圍與成本預覽」，但 UI 只有 maxCost 輸入與事後 spentCost | **open**：critic 自陳只做了 grep、confidence 低；本輪未實跑 UI 確認。與 D-X5 同源，一併留待實跑 |
| K-X9 | CLI 省略 `--max-cost` 時預設 0 ＝ 完全不送預算旗標 | **已裁定：屬實**，已寫進 K-07 |

### 4.1 本輪的環境阻礙（來源 `analysis/e2e-baseline.json` 的 `environmentBlockers`，照抄要點）

| 阻礙 | 影響的案例 | 需要的人類動作 |
|---|---|---|
| 真 iPhone 簽章：iPhone 11 已連線（available (paired)）且 Developer Mode enabled，但 Xcode 的 IDEProvisioningTeams 為空，`device-build.sh --check-only` 卡在 3/5 | E0-08 real-iphone；任何真機驗收欄位 | 在 Xcode → Settings → Accounts 登入 Apple ID 並選 Team |
| macOS 合成鍵盤事件遺失：AppleScript keystroke 打進 Tauri WebView 會非決定性掉字（兩次分別得到 `Native fixture codex can` 與 `Native fixture code`，`osascript` 全部 exit 0） | E0-03 native-desktop；E0-02 native 派工 | 在低負載機器重跑以區分負載因素；或授權改用剪貼簿貼上＋逐字元驗證 |
| NSOpenPanel 記憶目錄＋未驗證的 `choosefile`（缺陷 D18） | E0-11 native-desktop | 授權修改 `scripts/lib/tauri-ax.applescript` 加上「確認選到的檔名」斷言，並重驗 2026-09-06 的綠燈 |
| 磁碟只剩約 12 GB，本輪禁止 `cargo build`／`pnpm build`，原生 App 無法從 HEAD 重建 | E0-01／E0-03／E0-08／E0-11 的 native-desktop | 清出磁碟空間後從 HEAD 重建 `apps/interaction-desktop` |
| Claude 額度／冷啟動：R1 的五次 claude 執行從 task-sent 到 active 一律約 61–65 s；上一輪某次 session 送出任務後 21 s 仍是 created 且 provider log 沒有 assistant 紀錄（時間接近額度重置窗口，合理但未證實） | E0-02／E0-03／E0-04；任何有時間斷言的案例 | 避開額度重置窗口重跑並開 debug 級 daemon 日誌 |
| 共用機器＋多個並行 agent：整輪期間有其他 agent 的 daemon／CLI 同時在跑，所有 `ps` 斷言都必須依 ppid／pgid 鏈過濾 | 所有以子程序存活與否為證據的案例 | 由人類決定 sibling run 的殘留 daemon 由誰回收 |

---

## 5. 附錄

### 5.1 completeness critic 提出的漏列需求（**未逐列核實**）

> 以下 113 條全部由各維度的 completeness critic 提出，**本文件未逐列核實**，不得當成盤點結論。
> 它們的價值在於指出「67 列沒有覆蓋到的使用者需求」。`likelyStatus` 是 critic 的推測，不是本矩陣的狀態判定。
> 全文（含 `whyItMatters`／`whereToLook`）在 `evidence/2026-09-07-phase-0/static-matrix/dim-*.json` 的 `critic.missingRows`。

#### A 電腦與核心（20 條）

| 代號 | 需求 | critic 推測 |
|---|---|---|
| A-09 | 狀態列（tray）常駐與快捷動作；estop 與停感測不經 WebView | implemented-and-connected（原生證據只覆蓋部分選單項） |
| A-10 | 視窗生命週期（關閉主視窗、完全結束不動外部 daemon、桌面單一實例） | implemented-and-connected |
| A-11 | 開機自啟（launchAtLogin → macOS LaunchAgent）與啟動時開視窗 | confirmed-defect（autostart 失敗靜默）＋其餘 partial |
| A-12 | 事件串流（SSE）：即時更新、Last-Event-ID 續傳、重啟游標重置、斷線誠實降級 | implemented-and-connected |
| A-13 | 可追溯：稽核軌跡、活動收件匣、動作收據、時間軸、outbox | implemented-and-connected（保留期限／匯出／完整性 needs-investigation） |
| A-14 | 多 Agent：偵測、未安裝提示、停用、路由建議、同時上限、agent→agent 委派 | implemented-and-connected（委派的使用者入口 needs-investigation） |
| A-15 | 租約續期（renew） | **confirmed-defect**（後端／CLI 可用；桌面按鈕在三種模式都會丟錯——本文件已在 §4 A-X2 現場複核 grep） |
| A-16 | 追加訊息／回答（六態送達，以及有沒有真正的「回答 agent」入口） | implemented-partial |
| A-17 | 上下文注入與 session 上下文清除 | implemented-and-connected（與記憶維度重疊） |
| A-18 | 工作成果的保存與可讀性（重啟後還看不看得到成果） | confirmed-defect（與 A-08a 的宣稱不一致） |
| A-19 | 預算與政策設定的使用者入口 | implemented-partial |
| A-20 | 暫停／恢復與安靜時段（常態控制，非 estop） | implemented-and-connected |
| A-21 | 更新與版本一致性（self update、桌面與 daemon 版本不一致、schema 遷移、降級） | implemented-partial |
| A-22 | 解除安裝與資料清除（`--purge` 是否一併移除 skill／completion／LaunchAgent／App／SQLite） | needs-investigation |
| A-23 | 診斷與問題回報（daemon 日誌落地、Tauri 內嵌 stderr 去向、一鍵診斷包） | implemented-partial（接近 absent：無持久化日誌） |
| A-24 | 憑證管理（token 建立、輪換、外洩後撤換、外部模式讀不到 token 的復原） | absent（輪換與復原入口） |
| A-25 | Windows／Linux 平台對等性 | implemented-partial（Windows 取消語意 absent；安裝路徑文件層 confirmed-defect） |
| A-26 | 軟停止的層級（cancel／stop --all／session stop／estop 的差別與 UI 對應） | implemented-and-connected（語意邊界未盤點） |
| A-27 | 工作歷史的保留與清理（保留筆數／天數、mailbox 上限、ring 大小、能否刪單筆） | implemented-partial |
| A-28 | 背景守護（watchdog）與外部緊急停止檔 | implemented-and-connected（無使用者可見的健康指標） |

#### B 人類與 Agent 溝通（15 條）

| 代號 | 需求 | critic 推測 |
|---|---|---|
| B-M01 | 核可往返（唯一真正會被 connector 產生的「agent 問人、人回答」迴路） | implemented-partial（只有 codex app-server 有；卡片收合時可能整段錯過） |
| B-M02 | 非 gateway（輪詢型／跨 AI Skill）agent 的回話管道 | confirmed-defect（能力宣稱與 scope 相斥）／功能面 absent |
| B-M03 | 關閉工作之後成果與核可歷史還在不在 | confirmed-defect |
| B-M04 | 訊息預算把「agent 對人說的話」也一起扣掉 | confirmed-defect |
| B-M05 | 觀察儲存其實是事實上的對話紀錄面，且 agent token 讀得到別的 session 的文字 | implemented-and-connected（讀取面）＋needs-investigation（隔離零測試） |
| B-M06 | 跨裝置：人在 iPhone 上能看到／回應 agent 的溝通到什麼程度 | implemented-partial（只有狀態投影）／回覆與核可 absent |
| B-M07 | 人不在畫面前時的主動通知 | absent（拉取式徽章有，推送式沒有） |
| B-M08 | 選誰來做：agent 探索、登入狀態、事前擋下「按下去一定失敗」 | implemented-and-connected |
| B-M09 | 同時和多個 agent 溝通時「哪一個在等我」的優先呈現 | implemented-partial（人類面的注意力管理 absent） |
| B-M10 | 人類看不看得到「這次到底餵了什麼上下文」 | implemented-partial（CLI/HTTP 可見、桌面不可見） |
| B-M11 | 長工作的租約續期與到期的溝通 | implemented-partial |
| B-M12 | 附件與檔案（給 agent 一張圖／取回產物） | absent |
| B-M13 | 介面承諾的訊息種類（question／cancel）在 runtime 沒有語意 | confirmed-defect |
| B-M14 | 角色作為 agent 溝通面的邊界（演狀態但不轉述 agent 的話） | implemented-partial（決策未被記錄） |
| B-M15 | 第一次怎麼開始跟 agent 說話（五入口／Onboarding／FirstSuccess） | needs-investigation |

#### C 多 Agent（16 條）

| 代號 | 需求 | critic 推測 |
|---|---|---|
| C-11 | Agent 主動提問並在得到回答後續跑（waiting-for-input 的真實閉環） | defined-only |
| C-12 | 工作進行中的內容串流到人類介面 | implemented-partial |
| C-13 | 非 gateway agent 用自己的 token 回報進度／結果／交接 | implemented-partial 或 confirmed-defect |
| C-14 | 指定模型／回合上限 | defined-only（外加靜默忽略未知欄位的誠實風險） |
| C-15 | 從 iPhone 檢視／建立／核准／回覆多 Agent 工作 | absent |
| C-16 | 多個 session 同時等待人類時的主動通知與待決優先序 | implemented-partial |
| C-17 | 一件任務跨多個 session 的彙整與委派鏈可視化 | implemented-partial（介面 absent） |
| C-18 | 第二意見／交叉複審 | defined-only（只有路由標籤） |
| C-19 | daemon 重啟／當機後多 Agent 工作的恢復路徑 | implemented-partial |
| C-20 | 未知／未安裝的 agentId 也能建立出永遠不會執行的「工作」 | confirmed-defect（誠實度）或 by-design 未文件化 |
| C-21 | 多 Agent 的用量／花費總覽與跨 session 上限 | implemented-partial |
| C-22 | 同一 session 連續交代第二件事的語意 | implemented-partial／needs-investigation |
| C-23 | 真二進位的多 Agent 端到端證據 | not-run（**本輪 E0-06／E0-07 已補上**，見 §1.2） |
| C-24 | 每 session 的上下文包與跨 session 的記憶隔離 | implemented-and-connected（只覆蓋 kind==task） |
| C-25 | 多 Agent 軌跡的可追溯與保留邊界 | implemented-partial |
| C-26 | 委派給「另一台機器上的 agent」 | absent（需明確標為 out-of-scope 或列為缺口） |

#### D 策略與上下文（13 條）

| 代號 | 需求 | critic 推測 |
|---|---|---|
| D-07 | 多 Agent 之間的上下文交接（handoff → 下一位的上下文） | implemented-partial＋兩個待驗缺陷（`validate_handoff` 不限制 inferences/artifacts 長度；>8 KB 落地失敗被吞掉） |
| D-08 | 委派鏈的上下文與預算傳遞 | implemented-partial（envelope 只能由 HTTP 手工 payload 提供） |
| D-09 | Session 的資料／工具授權範圍由誰決定、有沒有正式入口 | confirmed-defect 或 implemented-partial（機制完整但桌面／CLI 無入口） |
| D-10 | 即時狀態（觀測／裝置／感測／角色／未解決停止）是否進入 Agent 上下文 | absent（push 不存在；pull 被空 tool_scope 全關） |
| D-11 | 使用者本人（偏好、人格、語言、角色設定）是否進入 Agent 上下文 | defined-only／by-design-absent（無文件記錄這條邊界） |
| D-12 | 上下文決策的可追溯性（audit／events／時間軸／活動頁） | implemented-partial |
| D-13 | 多回合上下文（第 2、3 則任務是否重附完整 bundle、有無增量／快取） | implemented-and-connected 但無策略（每回合全量重送） |
| D-14 | 事前預覽與同意的誠實性 | confirmed-defect 或 implemented-partial |
| D-15 | 人在回路的「反問／提問」上下文回路 | defined-only |
| D-16 | Agent 端契約的接線（skill 是否出現在 agent 執行環境、prompt 是否引用） | implemented-partial |
| D-17 | 主動說話 session 的宣告與實際上下文是否一致 | confirmed-defect（宣告 vs 實際）或 needs-investigation |
| D-18 | iPhone／行動端在策略與上下文維度有沒有任何入口 | absent |
| D-19 | 上下文品質治理回圈（Candidate／stale 的複審能否在正式入口完成） | needs-investigation |

#### E 記憶與知識（11 條）

| 代號 | 需求 | critic 推測 |
|---|---|---|
| E-10 | 記憶／知識到達 AI 的端到端接線（`domain:` 與 `knowledge.*` 授權在任何出貨入口都無法設定） | confirmed-defect（對使用者等同 defined-only） |
| E-11 | 備份與還原的還原半邊 | implemented-partial（知識／素材／收據 absent） |
| E-12 | 記憶變更的稽核與可追溯（PATCH 與批次清除沒有 audit） | implemented-partial |
| E-13 | 常態可追溯性：使用者能不能看到「這次實際帶了哪些記憶給 AI」 | implemented-partial |
| E-14 | Agent 提議記憶→人類核准的閉環（記憶層有沒有 inbox） | absent（記憶層）／implemented-and-connected（知識層） |
| E-15 | Domain Pack 作為 bundle 第二來源（版本、supersedes、佔用的預算） | implemented-and-connected（版本與 supersedes defined-only） |
| E-16 | 儲存成長與配額（記憶筆數、素材總量、稽核列數都沒有上限或清理） | absent |
| E-17 | 記憶／知識資料的靜態保護（DB 與素材 blob 的權限與加密） | implemented-partial |
| E-18 | iPhone 端對記憶／知識有沒有任何入口 | absent |
| E-19 | 一般模式下記憶與知識的可達性與搜尋 | implemented-partial |
| E-20 | 角色／Session 快照這第二份持久化狀態的內容與刪除路徑 | needs-investigation |

#### F iPhone（15 條）

| 代號 | 需求 | critic 推測 |
|---|---|---|
| F-07 | iPhone 感測受器閉環（motion／battery／mic-level／touch，含 governor 與感測不靜默） | implemented-and-connected（真機證據僅 v0.5.0 的 mic 停用列） |
| F-08 | iPhone 動器閉環（haptic／notify／tts／torch／flash／character） | implemented-and-connected（真機零驗收） |
| F-09 | 位置（location）：iOS 有開關、桌面有欄位，但沒有 observation 通道也沒有受器 | implemented-partial（by design）／UI 呈現面 needs-investigation |
| F-10 | 緊急停止與 stop-all 投影到手機 | implemented-and-connected |
| F-11 | 手機端能否發起停止／取消桌面正在跑的工作 | absent |
| F-12 | iPhone 作為 BLE 閘道（scan／connect／GATT） | implemented-partial（scan 有入口但零覆蓋）／defined-only（connect／gatt） |
| F-13 | iOS App 的安裝與散佈路徑 | implemented-partial（開發者自建）／absent（一般使用者） |
| F-14 | 手機互動的可追溯性（桌面能否依裝置篩選） | implemented-and-connected（runtime）／needs-investigation（頁面） |
| F-15 | 手機觀察是否進入記憶／上下文 | needs-investigation |
| F-16 | 角色跨裝置一致性（Character Pack 不可攜、降級如何讓使用者看懂） | implemented-partial |
| F-17 | 任何文字內容跨裝置（角色說的話、主動說話、agent 訊息） | absent（語意狀態面） |
| F-18 | 多支 iPhone 並存與裝置管理 | implemented-and-connected（桌面多裝置 UI needs-investigation） |
| F-19 | 網路範圍與遠端使用（wss 只綁區網／loopback，離開家就用不了） | absent（遠端）／by design（區網內） |
| F-20 | iOS 端的資料保存與隱私（Keychain 範圍、UserDefaults、日誌、清除） | needs-investigation |
| F-21 | 版本相容與升級路徑（舊 App 對新 daemon、協定棄用帳本的使用者呈現） | implemented-partial |

#### G 視覺化與一般模式（12 條）

| 代號 | 需求 | critic 推測 |
|---|---|---|
| G-07 | 多 Agent 的選擇、可用性與「誰在做／用什麼模型」的呈現 | implemented-partial（route_suggestion 在 UI 層 defined-only；模型 absent） |
| G-08 | 核可／拒絕／取消／中斷／關閉的一般模式操作面與逾時呈現 | implemented-and-connected（子程序真的消失只有 fixture 級證據） |
| G-09 | iPhone 端的工作可見度 | absent（工作／待回答／核可面） |
| G-10 | 首次使用與第一次成功的視覺流程 | implemented-and-connected（誠實度與 e2e 覆蓋 needs-investigation） |
| G-11 | 離線／Runtime 不可達／token 失效／Agent 未安裝時的誠實呈現 | needs-investigation（舊資料是否被標為過期未查） |
| G-12 | 狀態列（tray）的工作可見度與誠實度（只印工作階段計數，無待決定數） | implemented-partial |
| G-13 | 一般模式的用量與花費可見度 | implemented-partial（呈現面 absent） |
| G-14 | 記憶與知識在一般模式的呈現與人審入口 | needs-investigation |
| G-15 | 主動說話／安靜時段的呈現與可控性，及它與工作狀態的關係 | implemented-and-connected（互動 needs-investigation） |
| G-16 | 通知中心／全域搜尋／tray 深連結／舊錨點的路由一致性 | implemented-and-connected |
| G-17 | 進階模式九頁本身的內容與覆蓋 | implemented-partial |
| G-18 | 無障礙與窄視窗（<700px）下的一般模式 | implemented-and-connected |

#### K 模型選擇與 connector 相容（11 條）

| 代號 | 需求 | critic 推測 |
|---|---|---|
| K-09 | 模型清單發現（model catalog） | absent |
| K-10 | 執行中的模型改道／降級／驗證通知要被記錄並讓使用者看見 | absent |
| K-11 | 額度／配額狀態回報與耗盡處置（「卡住是因為額度」） | absent |
| K-12 | 每個 session 的 provider 溯源（二進位路徑／版本／transport 寫進 record 與稽核） | absent（record／稽核層）；整體 implemented-partial |
| K-13 | 相容性自檢與升級告警 | absent |
| K-14 | 使用者偏好層的模型／推理預設 | absent |
| K-15 | turn 級的模型／推理覆寫 | absent |
| K-16 | 對 codex 也有效的可強制用量上限（token 或 turn 上限） | implemented-partial；token 上限 absent |
| K-17 | 模型／旗標不相容時的診斷可得性（codex app-server stderr 要保留 tail） | implemented-partial（claude／exec 有；app-server absent） |
| K-18 | 模型／推理設定的公開契約（skill／tool manifest／API 文件，且未知欄位不得靜默吞掉） | absent |
| K-19 | iPhone／跨裝置入口的 agent session 面（含模型選擇） | absent |

### 5.2 可重現命令候選（reproCandidates）

> critic 為每個維度提出的驗證命令候選共 **93 條**（A 17、B 10、C 12、D 10、E 11、F 13、G 12、K 8）。
> 完整命令列在 `evidence/2026-09-07-phase-0/static-matrix/dim-*.json` 的 `critic.reproCandidates`。
> 下表只標「本輪跑過沒有」，**不重複貼命令**。

#### 已對應到本輪實跑的 E2E 案例

| 候選 | 本輪對應 | 證據 |
|---|---|---|
| A-R7 / G-R1 / G-R2（`agent_smoke.py` 對 claude 與 codex 各跑一次） | **E0-02、E0-10** | `e2e-runs/R8/{claude,codex}/`、`real-agent-e2e-prior/{claude-smoke-1,codex-smoke-1,codex-smoke-2}/` |
| A-R8 / G-R3（`agent_smoke.py --cancel-when-active --cancel-mode interrupt`） | **E0-03（product-failed）** | `e2e-runs/R1/{claude-interrupt,codex-interrupt}/`、`real-agent-e2e-prior/claude-cancel-1/` |
| A-R9（`restart_test.py --signal KILL`） | **E0-04** | `real-agent-e2e-prior/{restart-claude-kill,restart-codex-kill}/` |
| A-R10（`multi_session.py --agents claude-code,codex --cancel-index 0 --approve`） | **E0-06、E0-07** | `real-agent-e2e-prior/{multi-C-claude-x2,multi-D-codex-x2,multi-E-claude-codex}/` |
| A-R15（`tauri-work-cancel.py`／`tauri-settings-walkthrough.py` 重跑） | **E0-03／E0-11 的 native 段：harness-failed** | `e2e-runs/R7/{work,work-diag1,settings,settings-diag1,settings-diag2}/` |
| E-R5（刪除記憶後 bundle 回執仍含內容） | **E0-05** | `e2e-runs/R2/step5-*.json`、`E0-05-delete-newtask/` |
| K-R3（`codex app-server generate-json-schema` 比對 approval decision enum） | 本輪以唯讀 environment-probe 執行過（輸出只寫到 scratchpad，未歸檔） | K-06 的裁定；缺陷 **D4** |
| F-R12（daemon 停止時手機顯示什麼） | **半完成**：自動化的一半（`scripts/tests/ios-simulator.sh`）已在基線跑過（exit 0）；人工觀察那一半（關掉 daemon 後看 PairingView 文案）未做。本輪只用自寫 WS probe 驗 daemon 側，裝置側因 **D17** 走不完 | `baseline/ios-simulator.stdout.txt`、`e2e-runs/R6/logs/probe-{while-dead,after-restart}.json` |
| F-R13（真機 `device-build.sh`＋`device-acceptance.sh`） | **未實跑**（needs-environment：Xcode 未選 Team） | `analysis/e2e-baseline.json` 的 `environmentBlockers` |

#### 已由本輪基線覆蓋（全套執行，非逐項過濾）

`A-R14／B-R1／B-R8／B-R9／C-R1／C-R2／C-R3／C-R10／C-R11／D-R9／D-R10／E-R11／F-R1／F-R2／F-R3／F-R4／F-R5／G-R5／G-R6／G-R8／G-R9／G-R12／K-R8`
這一組全部是 `cargo test …`、`./scripts/v03-cli-e2e.sh`、`pnpm test`／`pnpm test:e2e`、`scripts/tests/ios-simulator.sh` 或
`scripts/tests/architecture-checks.sh`／`docs-claims.sh`／`release-scripts.sh` 的子集（A-R14 與 G-R8 只有腳本那一半算數，兩者附帶的 `sed`／`grep` 人工核對未做）。本輪基線在 HEAD `78dcda1` 上把這些**整套**跑過一次，
逐項紀錄（命令、起訖時間、秒數、exit code）在 `evidence/2026-09-07-phase-0/baseline/summary.jsonl`，
輸出在同目錄的 `*.stdout.txt`／`*.stderr.txt`。**未逐項以候選給的過濾條件單獨重跑**，所以只能說「該套件整體通過」，
不能說「候選斷言的那一條被單獨驗證過」。

#### 未實跑（純靜態或需另外起 daemon 的候選）

`A-R1…A-R6、A-R11…A-R13、A-R16、A-R17`、`B-R2…B-R7、B-R10`、`C-R4…C-R9、C-R12`、`D-R1…D-R8`、
`E-R1…E-R4、E-R6…E-R10`、`F-R6…F-R11`、`G-R4、G-R7、G-R10、G-R11`、`K-R1、K-R2、K-R4…K-R7`
本輪**未實跑**。其中三條的關鍵斷言已由本文件以現場 grep 複核（不等於跑過該候選的完整流程）：
A-R1（`agent_session_renew` 在桌面兩條 transport 皆零命中）、
A-R5 的一半（`TaskWaitingForInput` 在 connector 端零產生者）、
K-R3 的 schema 對照（`codex.rs:643-644` 送 `"reject"`）。

---

## 6. 相關文件

- [phase-0-progress.md](phase-0-progress.md)：階段 0 進度入口（下一動作與 Blockers）
- [phase-0-repository-state.md](phase-0-repository-state.md)：本階段的 repo／環境／binary 身分事實
- [phase-0-e2e-baseline.md](phase-0-e2e-baseline.md)：E0-01～E0-11 逐案結果（本檔各列的「本次結果」引用的 E0-xx 在那裡展開）
- [phase-0-known-issues-reproducibility.md](phase-0-known-issues-reproducibility.md)：D1–D20 與 L 維度的可重現性
- [phase-0-roadmap.md](phase-0-roadmap.md)：後續階段與下一個實作切片
- `evidence/2026-09-07-phase-0/artifact-manifest.json`：所有歸檔證據的原始路徑、sha256 與正規化說明
  （名稱含 `home` 的目錄——token 與 SQLite——一律不歸檔，因此本文件不引用那些路徑）
- 舊清單（保留為歷史紀錄，現況以本檔為準）：`docs/capability-completion-matrix.md`、
  `docs/v05-capability-gap-matrix.md`、`docs/v05-recovery-matrix.md`、[v0.6.0-recovery-matrix.md](v0.6.0-recovery-matrix.md)
