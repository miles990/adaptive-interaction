# 階段 0：真實狀態恢復、架構與流程盤點、雙平台真 Agent 驗收、測試基線與後續規劃（進度入口）

> 新 session 先讀 **§6 下一動作** 與 **§7 Blockers**，再決定動什麼。這份文件是階段 0 的唯一入口；
> 其他交付文件在 §4 逐一列出。所有「已驗收」都附證據路徑；沒有證據的一律標 not-run／needs-environment。

## 1. 起點與授權

- 查核時間：2026-09-07 11:15 +08:00 起（額度中斷後於 2026-09-07 16:46 與 2026-09-08 17:24 再核對；2026-09-08 18:22 網路 403 中斷後續接），Asia/Taipei。daemon binary：基線、上一輪真 agent 與 R1–R8 用 `1e066d84…`（09-07 由 HEAD 建置，有 `analysis/commands.json` 佐證）；H 輪補跑（09-07 傍晚）所用的 daemon binary 未被記錄，標 uncertain；上一個 session 記下的「09-08 17:29 重建 `9b013e95…`」在證據內查無來源，見 [Repository 真實狀態 §6](phase-0-repository-state.md)。逐案的 binary 身分見各文件。
- 起點：`main` = origin/main = `78dcda1a3733c97d266ca9b60ad4461c69ca2032`；最新 stable tag `v0.8.0`（object `5bca366…`→ `1fa69b8…`）；tag 之後只有 docs／test commit；無 open PR；工作樹乾淨。完整查核見 [Repository 真實狀態](phase-0-repository-state.md)。
- 施工分支：`phase-0/state-recovery-baseline`（自 78dcda1）。
- 使用者授權：建分支、修改本階段範圍檔案（現況／架構／基線／驗收文件、基線 runner、fixture、診斷、regression tests、被證據確認錯誤的文件、阻止驗證的最小修復）、跑測試、Conventional Commits、push、PR、必要 gates 通過後合併 main、符合條件時建立 annotated checkpoint tag（`phase-0-baseline-YYYYMMDD`；release.yml 只吃 `v*`，不會誤觸發發布）。不繞過 branch protection／密碼／簽章／平台限制；不購買額度。
- 不在本階段：新人機對話框架、完整非同步／流式協作、語音、大型記憶檢索、多 Agent 自動編排、多主機／雲端同步、新公開 SDK。階段 1 的功能實作**沒有**展開。

## 2. 本階段做了什麼（方法與模型分工）

| 步驟 | 方式 | 模型分工 |
|---|---|---|
| 真實狀態恢復 | 當場 `git`／`gh`／工具版本查核，寫成 [phase-0-repository-state.md](phase-0-repository-state.md) | 主模型（Claude Fable 5.1，本 session 的 orchestrator） |
| 靜態能力盤點 A–G／K／L、架構 H、測試責任 I、文件對照 J | Workflow：每維度一個 finder → 獨立懷疑者逐列反駁 → completeness critic；69 個 agent | finder／verifier：Sonnet（例行核對）；critic、H、I：Opus（架構判斷） |
| 測試基線 | `scripts/tests/phase0/baseline.sh`（本階段新增；沿用 repo 既有命令） | 無模型 |
| 真 Agent／多 Session／重啟／記憶／權限／備份／iPhone fixture／原生桌面 E2E | Workflow：11 個實跑或分析 agent，每個結果由獨立懷疑者用原始 artifact 反駁；25 個 agent | 實跑：Opus（取消／權限／原生／續接）與 Sonnet（記憶／問答／備份／fixture）；懷疑者：Sonnet |
| 測試設備修復與補證據（H 輪，2026-09-07 傍晚） | Workflow：fake_iphone 修復後重跑 F-06（跑完）；SIGTERM 重啟兩平台（跑完）；AX helper 修復後只留下一個中斷的定位探針（**未**重跑 native 走查）；核可逾時／併發上限執行期／Codex E-09 **沒有補跑**（仍在 [E2E 基線 §10](phase-0-e2e-baseline.md) 的「沒有驗到」清單） | Opus（修設備）、Sonnet（補證據） |
| 文件 | Workflow：每份文件一個作者＋一個懷疑者；整合者（主模型）裁決與收尾 | Opus 撰寫、Sonnet 核對 |
| 收尾（2026-09-14） | Workflow：補寫缺的 [E2E 基線](phase-0-e2e-baseline.md)（Opus）與 [證據索引](evidence/2026-09-07-phase-0/README.md)（Sonnet），各由獨立懷疑者逐條對照歸檔證據反駁、Opus 修正、再驗一次；同時 Sonnet 從零重建 `target/` 跑回歸（見 §8）；整合者裁決剩餘 findings（續開拒絕改以原始 probe 筆數計；H 輪 daemon binary 改標 uncertain）、修 `archive-evidence.py` 拒絕歸檔時只收回本次寫入的檔 | 主模型裁決；Opus 撰寫／修正；Sonnet 核對／回歸 |

真 Agent 的模型：gateway **無法指定也無法回報**模型（K-01／K-02／K-03 = absent）；requestedModel 一律「not-specifiable-via-gateway」，actualModel 只能事後從 provider 端本機紀錄唯讀取得——Claude Code 2.1.263 `claude-fable-5-1`（permissionMode plan）、Codex CLI 0.153.4 `gpt-6-astra`（reasoning_effort medium、sandbox read-only、approval untrusted）。逐次執行的模型／費用／token／耗時表見 [E2E 基線](phase-0-e2e-baseline.md)。

## 3. 測試基線（HEAD 78dcda1，2026-09-07，macOS 26.2 arm64；原始 log 在 [evidence/2026-09-07-phase-0/baseline](evidence/2026-09-07-phase-0/baseline/)）

| 命令 | 結果 | 耗時 | 證據層級 |
|---|---|---|---|
| `cargo fmt --all --check` | 0 diff | 1 s | static |
| `cargo clippy --workspace --all-targets -- -D warnings` | 0 warning | 1 s（快取） | static |
| `cargo test --workspace --no-fail-fast` | 1278 passed／0 failed／0 ignored，98 test binaries | 236 s | unit／integration／fixture |
| `cargo test --manifest-path apps/interaction-desktop/src-tauri/Cargo.toml` | 78／0 | 36 s | unit |
| `pnpm typecheck`／`pnpm test`／`pnpm build` | typecheck OK／1916 passed（92 files）／build OK | 4 s／23 s／6 s | unit（jsdom） |
| `scripts/tests/architecture-checks.sh --docs／--drill-lint／--ts／--rust／--swift／--drills` | 六組全部 PASS | 3／0／2／106／6／50 s | static／unit／native Swift／drills（fixture） |
| `scripts/v03-cli-e2e.sh` | 96 passed／0 failed | 13 s | integration（真 daemon＋mock device） |
| `pnpm test:e2e`（Playwright） | 92 passed | 214 s | browser（真 daemon＋fixture agents） |
| `scripts/tests/ios-simulator.sh` | 163／0 XCTest | 54 s | simulator（非真機） |
| `firmware/esp32-companion/compile.sh`／`--ble` | 兩者編譯成功 | 40 s／59 s | compile check（非真板） |

## 4. 交付文件索引

1. 本文件（進度入口）。
2. [Repository 真實狀態](phase-0-repository-state.md)。
3. [能力恢復矩陣](phase-0-capability-recovery-matrix.md)（A–G＋K；舊清單頂端已加指向）。
4. [架構依賴與資料流](phase-0-architecture.md)。
5. [測試責任矩陣與精簡處置表](phase-0-test-responsibility.md)（本階段只記錄處置，不刪測試）。
6. [已知問題可重現性](phase-0-known-issues-reproducibility.md)（L 維度 71 列＋本輪確認的 D1–D20）。
7. [雙平台與多 Session E2E 基線](phase-0-e2e-baseline.md)（E0-01～E0-11；Claude Code／Codex／fixture／原生桌面／真 iPhone 分欄）。
8. [文件與現況對照](phase-0-docs-vs-reality.md)（已套用的文件修正）。
9. [原始證據索引](evidence/2026-09-07-phase-0/README.md)（`artifact-manifest.json` 逐檔 sha256；home／token／SQLite 不歸檔）。
10. [後續階段與下一個實作切片](phase-0-roadmap.md)。

基線 runner 與真 Agent harness：[`scripts/tests/phase0/`](../../scripts/tests/phase0/README.md)。

## 5. 主要結論（摘要；細節在各文件）

- **電腦單獨跑通**：completed（安裝／啟動／首次設定／偏好／重啟／備份匯出還原）。
- **真 Claude Code**：派工、多輪、原生 `--resume` 續接、唯讀擋寫、授權寫入、human verify、撤銷、過期、重啟恢復、記憶送達與刪除——completed；**product-failed**：人類 interrupt 被記成 `failed` 而非 `cancelled`（D1，5/5 重現），close 的 SSE 投影把終局壓成 `closed`（D2），failed 沒有原因（D3）。
- **真 Codex**：同一組流程 completed，interrupt→cancelled 正確；**product-failed**：deny 送出的 wire 值 `"reject"` 不在 0.153.4 列舉內（D4；安全結果靠 provider fail-closed）、唯讀 session 仍啟動使用者設定的 MCP server 且任務全文出現在 process argv（D5）、`writable_roots` 併入使用者全域設定（D6）。
- **兩平台共同 not-implemented**：waiting-for-input 沒有任何連接器產生（D11；人工 `POST /report` 可達）；「停止這一輪但保留 session」不存在（D8）；模型／推理設定不可指定不可回報（K-01/02/03）。
- **多 Session**：同種×2、異種、單獨取消、隔離——completed；併發上限（max_sessions 8／max_parallel 4）只有 unit／fixture 層（隨 `cargo test` 基線），真 agent 執行期未驗（needs-environment）。
- **iPhone**：fixture 全部 completed；真機 needs-environment（Xcode 未選 Team）。
- **原生桌面**：綁定 2026-09-06 的 0.7.0 候選 App（非 HEAD 重建）；E0-01 completed；其餘三支走查在測試設備修復後的結果見 E2E 基線。
- 雙平台真 Agent 驗收**不宣稱「通過」**：Claude Code 與 Codex 各自的 completed／product-failed／not-implemented 清單在 E2E 基線 §「雙平台裁決」。

## 6. 下一動作

1. **階段 1 第一個切片：取消與失敗的誠實終態（D1＋D2＋D3）**——精確起點、紅燈測試、實作位置、真 agent 迴歸命令見 [roadmap §1.1](phase-0-roadmap.md#11-下一個實作切片最小完整可驗收)。起點檔案：`crates/interaction-agent-gateway/src/claude.rs:354`（interrupt）與 `:213-231`（stdout task）；測試：`crates/interaction-runtime/tests/gateway_loop.rs`（仿 `:1772`）。
2. 之後依序：D4（codex deny `decline`）＋D16（stderr 留痕）、K-01/02/03（model／effort 請求與實際值）、bundle 16 KiB／48 KiB 上限矛盾（D-06／E-04）、D5／D6 產品決策、D9 續開稽核。
3. 維護（不屬階段）：磁碟清理後從 HEAD 重建原生 App 並重跑四支 `scripts/tests/tauri-*.py`；用有斷言的 AX helper 重驗 2026-09-06 的 settings 綠燈。

## 7. Blockers

- 真 iPhone：Xcode → Settings → Accounts 未選 Team，`apps/interaction-ios/scripts/device-build.sh --check-only` 停在 3/5；需要人類在 Xcode 登入並選 Team，不得繞過。
- 原生桌面 App 未由 HEAD 重建：階段 0 期間磁碟可用空間在 3.4–12 GiB 之間波動，禁止 `pnpm tauri build`；native 證據綁在 2026-09-06 候選 bundle（sha256 `1103cda7…`）。收尾時（2026-09-14）`target/` 已被清除、可用空間 25 GiB，重建與四支 `scripts/tests/tauri-*.py` 重跑留給 §6 第 3 項的維護工作，本輪未做（AX 走查會接管鍵盤與剪貼簿，需人在場）。
- Codex 連接器邊界（D5／D6）需要產品決策後才能修。
- 額度：本階段兩次因 Claude Code session 額度中斷（2026-09-07 12:00、2026-09-07 21:40 前）；所有中斷都保留了原始輸出，續接時未重跑已完成的基線與真 agent 煙霧測試。

## 8. 提交、合併與 tag（整合者填寫，2026-09-14）

### 8.1 分支 `phase-0/state-recovery-baseline` 的 commits（自 `78dcda1`）

| commit | 內容 |
|---|---|
| `17d78e7` | `test: add phase-0 baseline runner and real-agent harness`（`scripts/tests/phase0/`；`archive-evidence.py` 拒絕歸檔時只收回本次寫入的檔） |
| `88181c0` | `fix(fixture): keep fake_iphone alive when reconnect fails (D17)`（只動測試 fixture） |
| `9384fd5` | `test(desktop): make the AX walkthrough helper assert instead of assume (D18, D19)`（只動 harness；native 重跑未做） |
| （本 commit） | `docs: phase-0 state recovery, capability matrix, E2E baseline and evidence`（十份交付文件、證據目錄 588 檔、CHANGELOG／README／AGENTS／docs 修正） |

### 8.2 提交前回歸（2026-09-14，`target/` 已被清除、從零重建；工作樹含上列全部變更）

| 命令 | 結果 | 耗時 |
|---|---|---|
| `cargo fmt --all --check` | 0 diff | 1 s |
| `cargo clippy --workspace --all-targets -- -D warnings` | 0 warning | 213 s（含與其他 cargo 工作的鎖等待） |
| `cargo build -p interaction-runtime --example fake_iphone` | exit 0 | 182 s（同上） |
| `cargo test --workspace --no-fail-fast` | 1278 passed／0 failed／0 ignored，98 個 `test result:` 行 | 361 s（同上） |
| `python3 -m py_compile`（三支 tauri driver＋`phase0/*.py`） | exit 0 | <1 s |
| `osacompile scripts/lib/tauri-ax.applescript` | 編譯通過（只檢查語法，未執行） | <1 s |
| `architecture-checks.sh --drill-lint`／`--ts` | 6 支腳本靜態檢查 PASS／vitest 230 passed | 1 s／11 s |
| `scripts/v03-cli-e2e.sh` | 96 passed／0 failed | 14 s |
| `pnpm typecheck`／`pnpm test` | tsc 乾淨／1916 passed（92 files） | 16 s／22 s |
| `scripts/tests/docs-claims.sh`（commit 前） | 236 passed／1 failed——唯一失敗是「`78dcda1..HEAD` 有提交但 progress 檔沒被更新」，是 commit 順序造成的預期失敗 | <1 s |
| `scripts/tests/docs-claims.sh`（docs commit 後） | 見 §8.3 | |

**未重跑**（沿用 §3 在 `78dcda1` 的基線；理由：本分支的產品程式碼零變更，改動只在 example fixture、AppleScript／Python harness 與文件）：Playwright、Tauri `cargo test`、iOS 模擬器、ESP32 compile、`architecture-checks.sh --rust／--swift／--drills`。
**未做**：從 HEAD 重建原生 App 並以修過的 AX helper 重跑四支 `tauri-*.py`（需人在場，見 §7）。

### 8.3 PR、CI、合併與 checkpoint tag

- 待填：PR 編號／main CI 結果／合併 commit／`phase-0-baseline-20260914` tag。
