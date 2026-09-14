# 維度 J：文件與現況對照

查核時間：2026-09-07。基準：HEAD `78dcda1a3733c97d266ca9b60ad4461c69ca2032`（= origin/main），v0.8.0 tag → `1fa69b8fafed54687119747c652a0869cb991f89`。方法：逐份讀 README.md／AGENTS.md／CLAUDE.md／docs/ARCHITECTURE.md／docs/FEATURES.md／docs/USER-GUIDE.md／docs/DESKTOP-GUIDE.md／docs/QUICKSTART.md／docs/INSTALL.md／docs/MAINTAINERS-MAP.md／docs/capability-completion-matrix.md／docs/v05-capability-gap-matrix.md／docs/v05-recovery-matrix.md／docs/releases/next-convergence-progress.md／docs/releases/v0.8.0-*.md／docs/releases/evidence-index.json／docs/acceptance-evidence.md（首尾各 150 行）／docs/aip/README.md／docs/aip/deprecation-ledger.md，逐條與程式碼（grep／Read 實檔）及 git 事實對照；並實際跑過 scripts/tests/docs-claims.sh（純 Python／git 靜態 lint，不建置、不跑 cargo/pnpm）確認哪些宣稱已有機械保護：**188 passed / 0 failed**。

## 總結

這個 repo 的文件分兩層，維護程度差很多。**跟得上**（每次發版同步更新、逐字對照無矛盾）：AGENTS.md、CLAUDE.md、docs/MAINTAINERS-MAP.md、docs/aip/README.md、docs/aip/deprecation-ledger.md、docs/acceptance-evidence.md、docs/releases/evidence-index.json、docs/releases/next-convergence-progress.md、docs/releases/v0.8.0-*.md 全系列——全部引用 main 1fa69b8／v0.8.0，file:line 全部核對存在。**沒跟上**（整份文件停在 v0.6.0 或 v0.5.1，v0.7.0／v0.8.0 的實際新增內容完全沒寫進去，不只是版本號舊）：README.md（除頂部橫幅）、docs/ARCHITECTURE.md、docs/FEATURES.md、docs/USER-GUIDE.md、docs/DESKTOP-GUIDE.md。docs-claims.sh 的 188 項檢查沒有一項是「文件是否提到某版本的內容」，只檢查「已提到的版本字串旁邊有沒有配過期詞」與兩個具體探針，所以下列缺口全部不會被現有自動化擋下。

## 主要 findings（詳見 JSON claims，每條附 file:line）

- **J-01**：README.md 頂部橫幅是 v0.8.0，但正文（"## v0.5"、"## AIP 1.0 與 Character Session"兩個唯一的版本標題段）完全沒有 v0.8.0 的 SemanticState schema、`aip.applied/1`、sensor journal、陪伴預設恢復——`grep -c "0.8.0" README.md` = 1（只有頂部那一次）。
- **J-02**：docs/ARCHITECTURE.md 標題與全文卡在「v0.6.0 已發布，2026-09-05」，最後一個章節是「## 6.6.0 Foundation」，之後沒有 v0.7.0／v0.8.0 任何章節；`grep -c "0.8.0"` = 0。這是 README 文件表格點名的「架構總覽」正式入口，落後最嚴重。
- **J-03**：docs/FEATURES.md 有「## v0.7.0」章節但沒有「## v0.8.0」章節。
- **J-04**：docs/USER-GUIDE.md（人類使用手冊）全文 216 行從未教人類怎麼用 `interact-ai agent …`（開 session／mailbox／續租／驗證），儘管 CLI 確實有這個子指令樹（`crates/interaction-cli/src/commands.rs:1382`）且 MAINTAINERS-MAP.md §7 列為一級能力。
- **J-05**：docs/DESKTOP-GUIDE.md 頭部自稱「本文描述的是 v0.5.1」，但正文 `:193` 已經寫「v0.6.x 起…」（自相矛盾）；且完全缺 v0.7.0 的「未解決停止」三層 UI（已核對是真接線元件：`apps/interaction-desktop/src/pages/connect/UnresolvedStops.tsx`，被 `ConnectPage.tsx:779`／`HomePage.tsx:27,93` 引用）與 v0.8.0 `aip.applied/1` 改動的同步卡文案。
- **J-06**：docs-claims.sh 實際跑過 188/0 全過，但讀完整支腳本確認它只做 10 類特定檢查（過期詞配版本字串、兩個具體探針、evidence-index tag→commit、deprecation-ledger 自我計數等），**不檢查文件是否遺漏某版本內容**——J-01/02/03/05 的缺口不在其覆蓋範圍，不是重複建議。
- **J-07**（confidence: medium）：ARCHITECTURE.md §6「MQTT／BLE 共用程式碼但未測」對 MQTT 而言已部分過期——v0.7.0 已有 `mqtt_rebind_loop.rs` broker 模擬器測試（`docs/acceptance-evidence.md` v0.7.0 證據表 #13），CHANGELOG [0.8.0] 也用更精確措辭區分「rebind 有測、applied 閉環未測」。
- **J-08／J-09**（正面確認，非缺陷）：v0.4／v0.5 舊矩陣正確自我標示為歷史文件並指向繼任者，不互相矛盾；MAINTAINERS-MAP.md 引用的 11 個檔案路徑逐一核對存在，RendererPort/DevicePort「零實作」的誠實聲明用 grep 驗證屬實。
- **J-10**（confidence: low）：docs/aip/README.md §9 Transport bindings 表不含宣告式裝置線，可能是刻意分工（device-profile.md 才是裝置線契約），未找到明確聲明確認。
- **J-11**：v0.8.0 系列文件正確處理 (d) 檢查項——main 發布後修復（857f009 rebind 測試競態）明確標示「tag 不變」，沒有被誤寫成 tag 已包含。
- **J-12**（confidence: medium）：抽查範圍內沒有發現 fixture／模擬器被誤標成真機／真 Agent 的地方；唯一「真機」字樣正確指向 v0.5.0-iphone-device-evidence.md。

## 建議更新的既有 canonical 文件（優先更新既有文件，不新增）

1. **docs/ARCHITECTURE.md**（最優先）——標題與內容需延伸到 v0.8.0，補 v0.7.0／v0.8.0 crate 與功能章節，修正 §6 MQTT 那句。
2. **docs/FEATURES.md**——補「## v0.8.0」章節。
3. **README.md**——「## v0.5」與「## AIP 1.0 與 Character Session」兩個標題段延伸版本範圍。
4. **docs/DESKTOP-GUIDE.md**——修正頭部版本範圍宣告；補未解決停止 UI 區塊；更新同步卡文案。
5. **docs/USER-GUIDE.md**——補 Agent Session 使用教學一節。

不需要改動：AGENTS.md、CLAUDE.md、docs/MAINTAINERS-MAP.md、docs/aip/README.md、docs/aip/deprecation-ledger.md、docs/acceptance-evidence.md、docs/releases/evidence-index.json、docs/releases/next-convergence-progress.md、docs/releases/v0.8.0-*.md 全系列、docs/capability-completion-matrix.md、docs/v05-capability-gap-matrix.md、docs/v05-recovery-matrix.md、docs/QUICKSTART.md、docs/INSTALL.md。

## 未追到 / 信心較低

- J-10 是否為刻意的契約分工，沒有找到明確聲明。
- J-07 的嚴重度取決於「未測」二字涵蓋 rebind 還是只涵蓋 applied 閉環，語意本身模糊。
- DESKTOP-GUIDE.md 的「更多」「Inbox」「自動互動」「響應式與無障礙」「收據狀態機」幾節沒有逐項對照最新 UI 程式碼，可能還有其他小範圍過期描述未被找到。

完整報告與逐條 file:line 證據已寫入：
- `/private/tmp/claude-501/-Users-user-Workspace-claude-lab-adaptive-interaction/79983209-6a38-4bdc-894b-1b32e1b2de38/scratchpad/matrix/find-J.md`
- `/private/tmp/claude-501/-Users-user-Workspace-claude-lab-adaptive-interaction/79983209-6a38-4bdc-894b-1b32e1b2de38/scratchpad/matrix/find-J.json`

未修改 repo 任何檔案（`git status --porcelain` 核對，只有此 session 開始前就已存在的其他 agent 產出的兩個 untracked 項目：`docs/releases/phase-0-repository-state.md`、`scripts/tests/phase0/`，均未被我碰過）。
