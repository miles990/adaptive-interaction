# AGENTS.md — 任何 AI 在此 repo 的入口地圖

跨 AI「能力感知型自適應互動平台」：Rust runtime＋`interact-ai` CLI＋HTTP API（127.0.0.1:8787，Bearer token）
＋SSE＋Canonical Tool Manifest＋跨 AI Skill＋Tauri 2 控制中心＋真硬體 adapter（Serial／MQTT／BLE）
＋iPhone Mobile Provider＋AIP 1.0 與權威 Character Session。最新已發布版本 v0.6.0（tag `v0.6.0`）。

這份檔案只回答「我該從哪裡開始、不能踩什麼」。**不可違反的不變量**（嚴禁 MCP、Policy Governor
確定性強制、誠實階梯、感測不靜默、角色呈現層沒有權限主權…）在 `CLAUDE.md`「不可違反的不變量」段，
那一段對所有 AI 一體適用，動手前必讀。

## 1. 分層與禁止依賴

契約：`docs/aip/architecture-boundaries.md` §1（分層圖與 §2 的 ports 表）。

```
Presentation / UI / Renderer   apps/interaction-desktop/src、apps/interaction-ios
Application Use Cases          crates/interaction-runtime/src/{character_session,character,mobile,agents,executor}.rs
Domain Core                    crates/interaction-{session,aip,character,core}（純函式，無 tokio／I/O）
Ports                          crates/interaction-session/src/ports.rs
Adapters / Transport           mobile.rs、declarative_session.rs、character_ws.rs、src-tauri、iOS ConnectionManager、TS character/adapters/*
```

依賴只能朝向核心。`interaction-aip` 與 `interaction-session` **不得**（直接或遞移）依賴
tokio／axum／tauri／tungstenite／rumqttc／serialport／btleplug／reqwest／hyper——這條線由
`tests/e2e/tests/dependency_boundaries.rs`（`pure_crates_declare_no_transport_or_runtime_dependencies`、
`pure_crates_do_not_pull_transport_or_runtime_crates_transitively`）釘在 CI 裡，不是慣例。
桌面端同一條線的對應物是「host 不依 entrypoint 字串分岔」：
`apps/interaction-desktop/src/test/architecture-no-entrypoint-switch.test.ts`。

## 2. Canonical source 對照（契約文件 ↔ 權威實作）

| 領域 | 唯一契約文件 | 權威 crate／模組 |
|---|---|---|
| AIP 1.0（跨裝置語意訊息） | `docs/aip/README.md` | `crates/interaction-aip`（schema 來源：`schemas/aip-1.0.schema.json`） |
| CPP 1.0（角色呈現） | `docs/character-protocol/README.md` | `crates/interaction-character`（schema：`schemas/character-protocol.schema.json`） |
| Character Session（權威語意狀態） | `docs/aip/character-session.md` | `crates/interaction-session`；接收端決策表 `src/receive.rs` |
| 裝置線協定 v1.x（Serial／MQTT／BLE） | `docs/aip/device-profile.md` | `crates/interaction-adapter-declarative/src/protocol.rs` |
| Runtime 接線（Session Host／transport 綁定） | `docs/aip/transport-bindings.md` | `crates/interaction-runtime/src/{character_session,declarative_session,declarative_lifecycle,mobile}.rs` |
| 桌面（一般模式 UX、同步卡） | `docs/aip/general-mode-ux.md` | `apps/interaction-desktop/src/{aip,statusProjection,character}` |
| iOS companion | `docs/aip/iphone-companion.md` | `apps/interaction-ios/InteractionCompanion/Services/{SessionReceive,SessionClient,ConnectionManager,SocketTransport}.swift` |

三端（Rust／TypeScript／Swift）永遠讀**同一份** fixture 對答案：
`crates/interaction-aip/tests/fixtures/manifest.json`。操作手冊在 `docs/aip/conformance.md`。

## 3. 動手前必讀（依你要改的領域）

- **角色（manifest／adapter／pack）**：`docs/character-protocol/README.md`＋`adapter-authoring.md`；
  `docs/aip/character-package.md`。核心 crate 不得出現任何角色專屬字串。
- **協定（AIP／CPP wire）**：`docs/aip/README.md` §4（版本與能力協商）、`docs/aip/compatibility.md`
  §2（minor 只能怎麼長）。改型別要重生 golden＋codegen（見 §4）。
- **裝置（宣告式 adapter）**：`docs/aip/device-profile.md`、`docs/aip/adapter-development.md`、
  `docs/aip/pairing-security.md`。身分強度用文件既有字串，不寫「已驗證身分」。
- **儲存（快照／遷移）**：`docs/aip/character-session.md` §6；`crates/interaction-session/src/types.rs`
  的 `SNAPSHOT_FORMAT`、`ports.rs` 的 `SaveOutcome`／`PortError::FutureFormat`。
- **設定（桌面偏好／匯入匯出）**：`apps/interaction-desktop/src/desktop.ts`、
  `src/companion/settingsTransfer.ts`（角色專屬欄位只由 adapter meta 宣告）。
- **一般模式 UI**：`docs/aip/general-mode-ux.md`；主入口恰好五個、不外洩技術詞（守門測試見 §4）。
- **威脅面**：`docs/aip/threat-model.md`、`docs/aip/privacy.md`。

能力歸屬（誰擁有什麼、擴充點在哪、必要測試是哪幾支）查 `docs/MAINTAINERS-MAP.md`。

## 4. 測試與生成命令

日常命令（fmt／clippy／`cargo test --workspace`／`pnpm typecheck|test|build`／`pnpm test:e2e`／
`./scripts/v03-cli-e2e.sh`／`pnpm perf`／iOS typecheck）一律以 `CLAUDE.md`「常用命令」段為準，
不在這裡重抄一份會漂移的副本。

架構層的檢查另有單一入口：

```bash
bash scripts/tests/architecture-checks.sh --list        # 只列出檢查項目（零成本）
bash scripts/tests/architecture-checks.sh --docs        # 文件誠實度／發布腳本自測
bash scripts/tests/architecture-checks.sh --ts          # 桌面守門測試（指定檔）
bash scripts/tests/architecture-checks.sh --rust        # 依賴邊界、schema 漂移、決策表、生命週期
```

協定型別**只能**由 `scripts/aip-codegen.mjs` 產生（`pnpm aip:check` 擋手改與忘記重生）；
golden schema 由 `GOLDEN_UPDATE=1 cargo test -p interaction-e2e --test golden` 重生。

## 5. 資料遷移與相容

- 每一條相容路徑（deprecated 函式、舊 id、舊快照格式、線協定追加訊息、feature flag）都必須登記在
  `docs/aip/deprecation-ledger.md`：為什麼存在、適用版本、移除前需要的證據、資料遷移、回退方式、
  下一個檢查里程碑、owner。**沒有登記的相容路徑等於沒有退場計畫**。
- 版本之間能不能對話、minor 可以怎麼長：`docs/aip/compatibility.md` §1／§2。
- 舊 Character Package 與舊 iPhone App 不得因為 deprecation 而無法啟動（`compatibility.md` §3）。
- 快照檔：舊版本 → 遷移並備份原檔；更新版本 → **不隔離、不覆寫**，store parked、以記憶體跑完這一輪。

## 6. 提交、合併與發布

- Conventional commits。不自行 push／發布／部署／開 PR，除非使用者明確要求。
- 發布固定三步：`./scripts/release-prepare.sh X.Y.Z` → `./scripts/release-verify.sh X.Y.Z`
  → `./scripts/release-tag.sh X.Y.Z --push`。`scripts/release.sh` 只印流程。
- PR 走 rebase merge；tag 只從已通過 `release-verify.sh` 的 commit 打，且不移動既有 tag。
- 交付前跑測試並回報**實際數字**，未完成項列明原因；已知限制寫進 `CHANGELOG.md` 與
  `docs/acceptance-evidence.md`，修掉時同步更新。
- 模擬器／fixture／程序內 client 的結果一律標示「模擬器」，不得寫成真機驗收。

## 7. 接續進度

新 session 先讀進度文件的 §4（Blockers）與 §5，再決定動什麼：

- 上一輪（可維護性收斂）：`docs/releases/v0.6.x-maintainability-progress.md`。
- 本輪：`docs/releases/v0.7.0-progress.md`（由整合者建立；若檔案還不在，以上一份為準）。

進度文件、`CHANGELOG.md`、`docs/acceptance-evidence.md` 與 `docs/releases/evidence-index.json`
由整合者統一維護——平行工作的 agent 不要各自改這幾個檔，避免衝突與重複記錄。
