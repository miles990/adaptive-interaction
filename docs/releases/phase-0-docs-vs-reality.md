# 階段 0：文件與現況對照——已套用修正

**這份文件是什麼**：階段 0「真實狀態恢復」的其中一項交付——把維度 J（文件與現況對照，12 個
claims＋懷疑者 verdicts）已確認為「文件錯誤／過期」的項目，逐條套用成最小幅度、與程式現況一致的
措辭修正；並額外處理兩項指派範圍外加入的檢查：A-01（`scripts/get.sh` 的平台涵蓋 vs
`docs/INSTALL.md` 的平台表）、以及 `e2e-baseline.json` `productDefects` 裡與文件宣稱相關的
D5／D6／D11（分別查核 `docs/FEATURES.md`／`docs/ARCHITECTURE.md`／`docs/USER-GUIDE.md` 是否有相反
或含糊宣稱）。

**來源**：
- `docs/releases/evidence/2026-09-07-phase-0/static-matrix/dim-J.md`
  與同目錄 `dim-J.json`（12 個 claims 逐條 file:line 證據＋獨立懷疑者 verdicts；查核基準
  HEAD `78dcda1a3733c97d266ca9b60ad4461c69ca2032` = origin/main，v0.8.0 tag → `1fa69b8`）。
- `docs/releases/evidence/2026-09-07-phase-0/analysis/e2e-baseline.json`
  的 `productDefects[]`（D5／D6／D11，真 codex／claude-code agent 與真程序觀測所得）。
- 本檔作者親自重新開啟每一處被引用的 file:line（程式碼與文件兩側）核對後才落筆；未逐列核對的地方
  一律標「open」，不假裝已核實。

**這份文件本身怎麼再產生**：dim-J 的 12 claims 與 verdicts 不會自動重跑（那是由另一個對抗審查流程
的獨立懷疑者手動產出並落盤在 scratchpad）；若程式碼或文件在此之後又變動，需要重新讀
`dim-J.json`、重新對照現況、重新套用差異。本檔列出的「處置」與「修正後文字」欄位反映的是**套用當下**
的檔案內容，之後若該檔再被編輯，欄位所引用的行號可能漂移——請以 `git log -p` 追蹤該行的後續變化。
機械檢查：`bash scripts/tests/architecture-checks.sh --docs`（`docs-claims.sh`＋`release-scripts.sh`，
純 Python／git 靜態 lint，不建置、不跑 cargo/pnpm）；本輪套用修正後**重跑通過**：
`docs-claims: 212 passed / 0 failed`、`release-scripts: 58 passed / 0 failed`（修正前為
`188 passed / 0 failed`——通過項目變多是因為新增的版本字串又餵給了既有的 stale-word 掃描窗口，
不是規則變寬）。

## 誠實階梯與裁決規則（本檔遵守）

- 「原判 → 改判」只用在懷疑者 `refuted: true` 的列，以其 `correctedStatus` 為準。12 條裡只有
  **J-07 一條 `refuted: true`**：懷疑者的結論是「原 finding 判過頭了，`docs/ARCHITECTURE.md` §6 那句
  『MQTT／BLE 共用程式碼但未測』其實**沒有真正矛盾**」（見下表 J-07 列）——按指派指示**不改**。
- 其餘 11 條 `refuted: false`（`correctedStatus` 多為 `holds`，少數帶引用行號的小勘誤），
  依「只改被程式碼證據確認錯誤的宣稱，最小措辭」原則套用。
- J-08／J-09／J-11／J-12 是懷疑者標記為「正面確認、非缺陷」的列——不是宣稱錯誤，本檔列出是為了完整
  交代 12 claims 的處置狀態，欄位一律「保留（已核實準確，不修正）」。
- D5／D6／D11 的查核範圍**限定**在 `docs/FEATURES.md`／`docs/ARCHITECTURE.md`／`docs/USER-GUIDE.md`
  三份文件（指派指示明列），逐一 grep＋開檔核對後**沒有發現**相反或含糊宣稱——見下表對應列與說明。

## 逐條處置表

| Claim | 文件位置 | 原宣稱（摘要） | 程式現況（file:line） | 處置 | 修正後文字（節錄） |
|---|---|---|---|---|---|
| J-01 | `README.md`:109,148（原始行號） | 頂部橫幅是 v0.8.0，但兩個版本段標題與內文只到 v0.6.0／v0.7.0 的日期腳注，`grep -c '0.8.0' README.md = 1` | `CHANGELOG.md`:20-45（[0.8.0] Added：SemanticState schema、`aip.applied/1`、sensor journal、陪伴預設恢復——四項均未見於原 README 內文） | **已修正** | `README.md`:109 版本段標題延伸列出「v0.8.0 已於 2026-09-06 發布」；`README.md`:148 標題改為「（v0.6.0 Foundation → v0.8.0，已發布）」；`README.md`:168-179 新增 v0.7.0／v0.8.0 兩段內容摘要（各自連到 `CHANGELOG.md` 對應版本段與 final-report／known-limitations），明確寫入「回執不代表畫面或物理效果已驗證」等既有誠實用語 |
| J-02 | `docs/ARCHITECTURE.md`:1,188（原始行號） | 標題卡在「v0.6.0 已發布，2026-09-05」；最後內容章節是 v0.6.0 Foundation，之後無 v0.7.0／v0.8.0；`grep -c '0.8.0' = 0` | `docs/MAINTAINERS-MAP.md`:62,68,86,110 列 `state_applied.rs`／`fragment.rs`／`sensor_journal.rs`／`preset_service.rs` 為已落地並附 file:line；本檔套用前已逐一確認四個檔案存在 | **已修正** | `docs/ARCHITECTURE.md`:1 標題改為誠實分工（v0.3–v0.6 詳述＋指向文末摘要，並標「最新已發布版本 v0.8.0」）；文末新增「## v0.7.0 與 v0.8.0 增量」一節（`docs/ARCHITECTURE.md`:315 起），逐模組附 file 路徑，不重複既有章節的逐 commit 詳述層級，完整證據導回 `CHANGELOG.md`／final-report |
| J-03 | `docs/FEATURES.md`（原始行號 187,191） | 有「## v0.7.0」但無「## v0.8.0」章節；`grep -c '0.8.0' = 0` | `CHANGELOG.md`:20-45（[0.8.0] Added 清單） | **已修正** | `docs/FEATURES.md`:187 新增「## v0.8.0（已於 2026-09-06 發布，tag `v0.8.0` → `1fa69b8`）」一節，摘要 SemanticState／`aip.applied/1`／sensor journal／陪伴預設恢復／五種擴充演練，並誠實註記 MQTT／BLE 專屬 applied 閉環尚未驗、真 iPhone／ESP32 真板本輪未取得驗收 |
| J-04 | `docs/USER-GUIDE.md`（全文 216 行，原無對應內容） | 人類使用手冊從未教「`interact-ai agent(s) …`」怎麼用；懷疑者更正：claim 自己引用的 `commands.rs:1382` 是錯的（那是 Inbox 過濾器的無關字串），真正的子指令族是 `main.rs:80-83` 的 `Command::Agents`（複數，`interact-ai agents ...`），子動作在 `main.rs:372-455` | `crates/interaction-cli/src/main.rs`:80-83（`Agents { action: AgentsAction }`）、:372-473（`Providers/Route/Approve/Interrupt/Sessions/Show/Create/Resume/Send/Messages/Report/Renew/Close/Verify`）；`crates/interaction-core/src/agent.rs`:16-41（`AgentSessionState` 列舉，含 `WaitingForInput` 的「no connector currently produces this state」原生誠實註解）；`crates/interaction-api/src/lib.rs`:488-533（`agent_request_allowed`：限權 agent token 連 GET `/v1/agent-sessions` 都被擋） | **已修正** | `docs/USER-GUIDE.md`:116 新增「## 4. 委派給 Agent：Codex／Claude Code」一節，逐一列出 `agents providers/route/create/sessions/show/send/messages/approve/interrupt/renew/resume/close/verify` 指令與人話註解、`AgentSessionState` 誠實階梯（含 `waiting-for-input` 目前無連接器自動產生的已知落差）、agent token 權限邊界；原第 4–8 節順移為第 5–9 節 |
| J-05 | `docs/DESKTOP-GUIDE.md`:12-16 vs :193（原始行號） | 頭部宣稱「本文描述的是 v0.5.1」，同文 :193 卻已寫「v0.6.x 起…」——自相矛盾；另缺 v0.7.0 未解決停止三層 UI 與 v0.8.0 `aip.applied/1` 同步卡自報文案 | `apps/interaction-desktop/src/pages/connect/UnresolvedStops.tsx`:1-35（真元件）；`ConnectPage.tsx`:47,779 掛載；`HomePage.tsx`:27,93 引用；`apps/interaction-desktop/src/statusProjection/characterSync.ts`:90-93（`synced.detail` = 「裝置回報已套用目前的角色狀態；這不代表已驗證畫面或實體效果。」） | **已修正** | `docs/DESKTOP-GUIDE.md`:12 改「本文的**畫面截圖基準**是 v0.5.1」（不再宣稱全文範圍鎖定該版本）；:18 新增段落列出 v0.6.x／v0.7.0／v0.8.0 之後補上、但沒有新截圖的文字變動；:242 「已同步」列的「意思」欄改為含 `aip.applied/1` 自報與「不代表已驗證」的用語；:324 新增「未解決停止」段落說明 `UnresolvedStopsSection` 的行為與誠實文案 |
| J-06 | `scripts/tests/docs-claims.sh`（全 389 行） | 現有 188/0 通過的 doc-honesty lint 只做過期詞配版本字串、兩個具體探針、evidence-index tag→commit 等 10 類特定檢查，不檢查「文件是否遺漏某版本內容」——J-01/02/03/05 的缺口不在其覆蓋範圍 | 本檔套用前完整讀過 `scripts/tests/docs-claims.sh` 全文，逐段對照確認懷疑者列出的檢查清單準確 | **保留（已核實準確，非缺陷；本身不修正 docs-claims.sh）** | 不適用——J-06 本身是「lint 覆蓋範圍」的事實陳述，不是待修正的文件宣稱；本輪套用 J-01/02/03/04/05 後重跑 `docs-claims.sh` 仍 212/0 全過，佐證 J-06 的判斷（新增內容沒有觸發任何既有規則失敗，因為根本沒有對應規則） |
| J-07 | `docs/ARCHITECTURE.md`:302-304（原始行號） | 原 finding：「§6『MQTT／BLE 共用程式碼但未測』對 MQTT 已過期，因為 v0.7.0 有 `mqtt_rebind_loop.rs`」 | `crates/interaction-runtime/tests/mqtt_rebind_loop.rs`:1-16（doc comment 明講只測 rebind 機制本身，不含 AIP binding）；:68-107（fake device 只答 who/pair/read/stop-all，無 AIP）；`grep 'rebind\|閉環' docs/ARCHITECTURE.md` = 0 hits（該文根本沒有拿 rebind 當佐證） | **保留＋理由（懷疑者判定 `refuted: true`）** | **原判「§6 這句話已過期」→ 改判「holds——兩份文件之間沒有真正矛盾」**（懷疑者理由：§6 講的是 AIP binding 閉環未測，`mqtt_rebind_loop.rs` 測的是傳輸層 rebind 機制本身，兩者是不同的測試表面；`CHANGELOG.md`:54 與 `docs/MAINTAINERS-MAP.md`:68 也用同樣的區分方式）。按任務指示**不改** `docs/ARCHITECTURE.md`:302-304 原文 |
| J-08 | `docs/capability-completion-matrix.md`、`docs/v05-capability-gap-matrix.md`、`docs/v05-recovery-matrix.md` | 正面確認：三份舊矩陣正確自我標示為歷史文件並指向繼任者，與現行 canonical 文件無矛盾 | `docs/capability-completion-matrix.md`:1-4；`docs/v05-capability-gap-matrix.md`:27-29；`docs/v05-recovery-matrix.md`:1-3 | **保留（已核實準確，非缺陷）** | 不適用——未改 |
| J-09 | `docs/MAINTAINERS-MAP.md`、`AGENTS.md`、`CLAUDE.md`、`docs/aip/README.md`、`docs/acceptance-evidence.md`、`docs/releases/evidence-index.json` 等 | 正面確認：與 HEAD `78dcda1`／v0.8.0 一致；11 個 file:line 引用逐一存在；RendererPort／DevicePort「零實作」聲明經 grep 驗證屬實 | `AGENTS.md`:5；`grep -rn 'impl.*RendererPort\|impl.*DevicePort' crates/ apps/` = 0 hits；`git rev-parse v0.8.0 v0.8.0^{commit}` | **保留（已核實準確，非缺陷）** | 不適用——未改 |
| J-10 | `docs/aip/README.md` §9（Transport bindings 表） | 表中六列沒有宣告式裝置線（Serial／MQTT／BLE）一列；懷疑者查核後認為缺口比原 claim 更廣（§0 關係段、§14 Conformance tests 同樣沒提） | `docs/aip/README.md`:243-249（六列表）；`docs/MAINTAINERS-MAP.md`:65（指定 `docs/aip/device-profile.md` 為裝置線契約） | **open（信心 low，未找到「刻意分工」的明確聲明，也未找到「這是遺漏」的反向證據）** | 不適用——本輪未改；需要 `docs/aip/README.md` 的維護者確認這是刻意的文件分工（device-profile.md 另立契約）還是應該在 §9 補一列 |
| J-11 | `CHANGELOG.md`、`docs/releases/v0.8.0-ci-followup.md`、`docs/releases/evidence-index.json` | 正面確認：v0.8.0 文件家族正確處理「main 上的測試維護修復（857f009）不算進 tag」，沒有誤寫成 tag 已包含 | `docs/releases/v0.8.0-ci-followup.md`:21；`CHANGELOG.md`:9-19；`evidence-index.json` `postReleaseTestCorrections[0].tagUnchanged == true` | **保留（已核實準確，非缺陷）** | 不適用——未改 |
| J-12 | 全域抽查（README／FEATURES／ARCHITECTURE／DESKTOP-GUIDE／acceptance-evidence.md／v0.8.0-known-limitations.md／MAINTAINERS-MAP.md） | 正面確認：抽查範圍內沒有 fixture／模擬器被誤標成真機／真 Agent 的地方 | `docs/DESKTOP-GUIDE.md`:332-334（唯一「真機」字樣正確指向 `v0.5.0-iphone-device-evidence.md`）；`README.md`:161-162（AIP 段落用「implemented-unverified」誠實用語） | **保留（已核實準確，非缺陷；僅抽查，非全文覆蓋）** | 不適用——未改 |
| A-01 | `docs/INSTALL.md`（原始行號 63-74 附近，套用前為 62-90） | 平台覆蓋表把 install.sh 隱含當成四平台（macOS arm64/x64、Linux x64、Windows x64）的統一入口，全文沒有任何一處說明 Windows 不能靠 `bash install.sh` 取得 | `scripts/get.sh`:64-73（`case "$(uname -s)/$(uname -m)"`：只列 `Darwin/arm64`、`Darwin/x86_64`、`Linux/x86_64`、`Linux/aarch64`(明確報錯) 與 `*` 兜底 `unsupported platform`；`grep -iE 'windows\|mingw\|msys\|cygwin\|\.exe\|\.msi' scripts/get.sh` = 0 hits，全檔確認無 Windows 分支） | **已修正** | `docs/INSTALL.md`:93 新增段落：「**Windows x64 使用者請不要執行 `install.sh`**：這支腳本的平台偵測只認 `uname` 回報的 `Darwin`／`Linux`……即使 Windows x64 的 CLI 壓縮檔與 `.exe`／`.msi` 安裝包確實存在於 Release。請直接到 Releases 頁面手動下載對應資產。」 |
| D5 | 查核 `docs/FEATURES.md`／`docs/ARCHITECTURE.md`／`docs/USER-GUIDE.md` 是否有相反或含糊宣稱 | D5：`codex` 連接器沒有等價於 `claude.rs` 的 `--strict-mcp-config`；唯讀 session 一建立仍會啟動使用者 `~/.codex` 設定裡的 MCP server | `docs/FEATURES.md`:46「完全不使用 MCP」——逐句核對後這句話講的是**本專案 Runtime 自己**不依賴 MCP 當介面（對應 `CLAUDE.md` 的「嚴禁 MCP」不變量，由 lockfile 測試強制），與「codex 連接器是否隔離 guest session 自己的 MCP 設定」是不同主張；`docs/ARCHITECTURE.md`:118-124 只談 token 分權與環境變數移除，未對 MCP／沙箱做任何宣稱；`docs/USER-GUIDE.md` 全文對 MCP／codex 沙箱零提及（`grep`） | **保留（已查核，未發現相反或含糊宣稱，不修正）** | 不適用——三份文件都沒有對「codex 連接器會隔離／封鎖 MCP」做出宣稱，因此談不上與 D5 矛盾；D5 本身仍是未修的 product defect，不在本檔（文件對照）修正範圍 |
| D6 | 同上 | D6：`allowWrite` 的 codex session 只送 `sandbox:"workspace-write"` 字串，不送 `writable_roots`，實際寫入範圍會併入使用者全域 `~/.codex/config.toml`，可能超出 session 授權的 `resolvedWorkdir` | 同上三份文件全文 grep `allowWrite\|writable_roots\|toolScope` 均無命中（唯一提到 `allowWrite` 語意的是 `docs/DESKTOP-GUIDE.md`:297，但該檔不在本次 D5/D6/D11 指派查核的三份文件清單內——見下方 open） | **保留（指派範圍內三份文件未發現相反或含糊宣稱，不修正）** | 不適用——`docs/FEATURES.md`／`docs/ARCHITECTURE.md`／`docs/USER-GUIDE.md` 都沒有對「寫入範圍被系統擋在 workdir 內」做出具體宣稱；`docs/DESKTOP-GUIDE.md`:297 一句「系統擋得住『改不到別的地方』」與 D6 的落差留在下方 open，本輪按指派範圍不改 |
| D11 | 同上 | D11：`GatewayEvent::TaskWaitingForInput` 只有定義與單一消費點，沒有任何連接器會自動產生它，UI 的 NEEDS_INPUT 投影永遠不會被觸發 | `crates/interaction-core/src/agent.rs`:24-30（`WaitingForInput` 的原生文件註解已自陳「HONEST GAP：no connector currently produces this state」）；`docs/FEATURES.md`:163 的 taxonomy 列表本身沒有把 `waiting-for-input` 列進去，也沒有宣稱它能運作，不構成「相反」宣稱；`docs/ARCHITECTURE.md`／`docs/USER-GUIDE.md`（套用修正前）對此零提及 | **保留（查核範圍內未發現相反或含糊宣稱，不修正原文）＋順手補強** | 不適用（三份文件原文都沒有矛盾）；額外在新增的 `docs/USER-GUIDE.md`:141-147（見 J-04 列）主動寫入這個已知誠實落差，避免這份新增內容日後被誤讀成「waiting-for-input 已可用」 |

## `editedCanonicalDocs`（本輪實際修改的檔案）

- `README.md`：見上表 J-01。
- `docs/ARCHITECTURE.md`：見上表 J-02（J-07 對應句子維持原狀，未改）。
- `docs/FEATURES.md`：見上表 J-03。
- `docs/DESKTOP-GUIDE.md`：見上表 J-05。
- `docs/USER-GUIDE.md`：見上表 J-04（同時是 D11 的順手補強落點）。
- `docs/INSTALL.md`：見上表 A-01。

未修改：`AGENTS.md`、`CLAUDE.md`、`docs/MAINTAINERS-MAP.md`、`docs/aip/README.md`（J-10 open，未動）、
`docs/aip/deprecation-ledger.md`、`docs/acceptance-evidence.md`、`docs/releases/evidence-index.json`、
`docs/releases/next-convergence-progress.md`、`docs/releases/v0.8.0-*.md` 全系列、
`docs/capability-completion-matrix.md`、`docs/v05-capability-gap-matrix.md`、`docs/v05-recovery-matrix.md`、
`docs/QUICKSTART.md`、`scripts/tests/docs-claims.sh`。

## Open（未修正，留給後續）

1. **J-10**：`docs/aip/README.md` §9 Transport bindings 表是否刻意不列宣告式裝置線（因為
   `docs/aip/device-profile.md` 另立契約），還是應該補一列——沒有找到明確聲明，需要維護者裁決。
2. **D6 在 `docs/DESKTOP-GUIDE.md`:297 的落差（超出本輪指派範圍，記錄供下一輪處理）**：該句「系統擋得住
   『改不到別的地方』」把「寫入範圍被系統限制在選定資料夾內」講成已兌現的保證，但 D6 顯示 codex
   session 的 `allowWrite` 只送 `sandbox:"workspace-write"` 字串、未送 `writable_roots`
   （`crates/interaction-agent-gateway/src/codex.rs`:315-340），實際生效範圍會併入使用者全域
   `~/.codex/config.toml` 的既有設定，理論上可能超出 `resolvedWorkdir`。本輪指派的查核清單明列只看
   `docs/FEATURES.md`／`docs/ARCHITECTURE.md`／`docs/USER-GUIDE.md` 三份文件，`docs/DESKTOP-GUIDE.md`
   不在清單內，故本輪**未修改**該句；建議下一輪把它與 D6 的 product-side 修復（`suggestedPhase: "2"`）
   一起排入，避免文件先誠實化、程式碼後補上，或反過來造成文件與已修正程式碼再度脫節。
3. **DESKTOP-GUIDE.md 未逐節核對的區塊**（沿用 dim-J 原始 open）：「更多」「右上角 Inbox」
   「自動互動」「響應式與無障礙」「收據狀態機」五節本輪未逐項對照最新 UI 程式碼，可能還有其他小範圍
   過期描述未被找到。
4. **是否與其他維度（A–I、K、L）的 find 重複**：沿用 dim-J 原始建議，由整合者跨維度核對，避免重複
   修復工作。
