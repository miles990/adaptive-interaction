# 架構總覽（v0.5.1 已發布；v0.6.0 開發中；v0.3 基線＋v0.4／v0.5／v0.6 增量）

跨 AI 自適應互動平台的核心是一個 **Rust runtime**。CLI、HTTP API 與 Tauri 桌面
控制中心都是它的 client，共用同一套 application service 與同一個 deterministic
Safety Governor——沒有任何入口能繞過授權。**不使用 MCP**。

```
任何 AI／Agent／人類
   │  Skill＋CLI ·  HTTP API＋SSE ·  Tool Schema ·  Tauri IPC
   ▼
Rust Adaptive Interaction Runtime
   ├── Capability Registry（receptor / actuator / tool operation）
   ├── Provider Registry（device / service / ai-session / companion…）
   ├── Recipe Engine（YAML/JSON，句子式編輯器的無損模型）
   ├── Adaptive Orchestrator（候選→評分→最小有效行動；允許不介入）
   ├── deterministic Policy / Consent / Safety Governor
   ├── Action State Machine + Receipt Store（SQLite）
   ├── Agent Session Manager（lease / mailbox / delegation safety）
   ├── Sensor Manager（麥克風 listen 視窗；always-visible 指示）
   └── Adapter SDK ＋ Declarative Adapter Engine（HTTP/SSE）
```

## Crate 責任

| Crate | 責任 |
|---|---|
| `interaction-core` | 領域模型：manifest、observation、action/receipt 狀態機、policy、human 語意層、**provider**、**agent session／delegation** |
| `interaction-registry` | Capability Registry、Common Capability Catalog、human-view resolver、**Provider Registry** |
| `interaction-policy` | deterministic Governor：min() 限界鏈、consent、quiet hours、allowlist、預算、**delegation limits** |
| `interaction-recipe` | Recipe 模型／驗證／摘要／AI 決策閘門；YAML↔JSON 無損 round-trip |
| `interaction-runtime` | Tokio runtime：orchestrator、executor、watchdog、human 層、**providers**、**agents**、**sensors** |
| `interaction-storage` | SQLite：receipts / plans / sessions / audit / ai_descriptions / **providers** / **agent_sessions**（schema v3） |
| `interaction-events` | EventBus（broadcast）＋ bounded replay |
| `interaction-api` | Axum HTTP API＋SSE（Last-Event-ID）；loopback-only、human/restricted-agent Bearer tokens |
| `interaction-cli` | `interact-ai`：40＋子指令，`--json` 潔淨輸出 |
| `interaction-tool-schema` | Canonical Tool Manifest → OpenAI/Anthropic/Gemini/OpenAPI/JSON-Schema（golden tests） |
| `interaction-adapter-sdk` | Receptor/Actuator manifest builder、DriverReceipt 協定、human meta helper |
| `interaction-adapter-declarative` | **宣告式 adapter**：YAML spec → 真 HTTP/SSE receptor/actuator（policy-bounded、secret://、SSRF 防護） |
| `interaction-character` | **Character Presentation Protocol 1.0**（純函式）：manifest 驗證／migration、能力協商、intent／truthState／priority floor、input 正規化、回執狀態機、wire messages、`Gateway`；JSON Schema 來源。v0.6.0：不再含任何小樞字串，`builtin_whitelist` 改為空、由 host 注入 |
| `interaction-aip` | **v0.6.0 進行中** — Adaptive Interaction Protocol 1.0（純函式，無 tokio／I/O）：跨裝置語意訊息 envelope、12 種 message type、12 值 Outcome 誠實階梯、19 個穩定錯誤碼、版本協商、確定性能力協商、身分綁定決策、離線事件政策、證據分類、canonical JSON hash；JSON Schema 來源（`schemas/aip-1.0.schema.json`）。契約：`docs/aip/README.md` |
| `interaction-session` | **v0.6.0 進行中** — 權威 Character Session（純函式）：語意狀態（mood／activity／attention／truth／members）唯一 owner、確定性 Director、revision／sequence／snapshot／patch／delta replay、有界安全管線、ports（Clock／SessionStore／IdentityVerifier／ConsentVerifier／RendererPort／DevicePort）。契約：`docs/aip/character-session.md` |
| `interaction-character-shu` | **v0.6.0 進行中** — 小樞（`shu-rig`）專屬內容從 `interaction-character` 核心抽出：variants、能力集、`character-rig` 2.0 遷移（`ShuRigPack`）。核心 crate 不再含任何小樞字串 |
| `adapters-builtin` | 內建 receptor/actuator（conversation/web-ui/log/notification/webhook/mock/companion/agent…） |
| `adapters-media` | **高敏感媒體 receptor**：麥克風 listen（feature-gated cpal；預設關；記憶體內只留 level 事實） |
| `interaction-desktop` | Tauri 2：狀態列 runtime、RuntimeSupervisor、可信 host overlay、角色視窗（TS `CharacterGateway`＋小樞／sprite／text adapters）、角色匯入、控制中心 |

核心 crate 不依賴 Tauri／Axum／cpal／特定硬體；設備專屬 adapter 各自獨立。

## 桌面 App 生命週期（狀態列常駐）

```
啟動 → RuntimeSupervisor 偵測
   ├── 已有 interact-ai daemon → ConnectedToExternal（前端切 HTTP transport，token 走 IPC）
   └── 沒有 → EmbeddedOwned（app 擁有內嵌 runtime＋HTTP API）

關閉控制中心視窗 → 首次顯示說明對話框 → keep-running / hide-companion / quit
   · keep-running：只隱藏視窗，狀態列＋小樞＋runtime 續跑
   · 完全結束（tray / Cmd+Q）→ RunEvent::Exit → runtime.shutdown()
      （取消未完成動作 · 停止感測器 · 關閉 agent session · 保存 receipt · 釋放鎖）
   · 外部 daemon 模式：完全結束只關 app，不動外部 daemon
```

狀態列 icon 反映：正常／暫停／AI 工作中／需同意／麥克風使用中／攝影機使用中／
部分離線／緊急停止／Runtime 離線——**永遠有文字，不只靠顏色**。

## 桌面角色小樞（Companion）

第二個透明無邊框 Tauri 視窗。它是**呈現層與輸入入口**，不持有任何權限：

- **素材管線**：參數化 SVG rig → 確定性 sprite sheet（72 幀×3 變體×18 動畫，
  anchor 固定、可循環）。原創、程序化生成、無 AI 產圖。
- **確定性狀態機**：completed=點頭；綠色勾勾**只在 verified 時**出現；
  emergency 凍結一切動畫並固定安全姿勢；pack fallback 不會把安全狀態演成慶祝。
- **輸入**：點擊→快捷操作（確定性，不呼叫 AI）；文字輸入＋資料去向預覽；
  拖放先預覽再確認。**原始游標座標只在 renderer 記憶體，不持久化、不傳 AI**；
  只送語意事件（`desktop.companion.interaction`）。
- **Persona / World / Story packs**：純資料、有界、無可執行內容。
  **安全語句（emergency/blocked/unknown/sensor-in-use）固定不可覆寫**——
  驗證器標記覆寫企圖，resolver 也直接無視。

## 誠實階梯（貫穿全系統）

```
requested → sent → acknowledged → completed → verified
```

- 裝置 actuator 的 200 回應 = **acknowledged**，不會自動升級成 completed
  （只有 actuator 正式宣告 ack 即送達的本機通道，如對話/紀錄，才在 best-effort
  下完成；設備需 observation 才能 completed）。
- Agent 的「聲稱完成」= observation 的 **inference**，永遠不是 receipt、
  不是驗證證據；偷渡的 `actionId` 會被改名成 `claimActionId`。
- 委派入信箱 = dispatched；session 實際領取才 = acknowledged。

## Provider 生命週期

```
discovered → unpaired → paired → installed → disabled → available
                                                  ↕
                                    busy / degraded / disconnected
   （任何 live 狀態）→ expired / revoked → closed
```

配對／安裝／啟用／同意是**分開的步驟**，registry 拒絕捷徑轉換；revoked 是黏性狀態
（無法回到 available）。配對用 sha256 指紋（**IP 不是身分**）。

硬體發現另由 `HardwareDiscoveryAdapter::scan_metadata` 處理；掃描只讀名稱、
類型、穩定身分來源、權限需求與可用狀態，**不開啟感測**。目前有
17 類覆蓋結果、Linux `/dev/*/by-id` 與 macOS serial metadata；無法穩定識別的
裝置留 `stableId:null`，不可直接配對。

## Agent Session

```
AI Provider → Agent Profile → Agent Session（有租約、有預算、有範圍）
```

Session 透過 runtime mailbox 溝通（不互讀對話）；委派攜帶防循環信封
（depth／cycle／budget／session-count，policy 強制）；estop 取消所有 open session
並阻擋新建；open session 不跨重啟存活（誠實標記 expired）。

HTTP 信任面有兩個 0600 token：`state/api-token` 屬人類控制面，
`state/api-agent-token` 只能讀狀態、呼叫 canonical tools 與往安全方向停止。
agent token 無法開／授權 session、發布知識、修改 policy 或 clear estop；
Codex／Claude 子程序啟動時也會移除所有 Runtime token 環境變數。

## 感測隱私

麥克風預設關＋Intimate＋consent-gated。`begin_mic_listen` 在 Rust 強制三重閘門：
no-estop ＋ receptor enabled ＋ 該 receptor 的明確 session consent。listen 視窗有
30 秒硬上限（watchdog 每 tick 掃描）。**無靜默擷取路徑**——擷取狀態同步到
`status.activeSensors`、`sensor.started/stopped` 事件、狀態列 glyph、控制中心橫幅
與小樞標籤。原始音訊只在記憶體、只導出 level 事實，不存不傳。攝影機**誠實未實作**
（不做假 driver）。

## v0.4 增量架構

```
┌─ 控制中心（新 IA：首頁/小樞/AI/能力/記憶知識/自動/活動/安全/設定 ＋ ⌘K）
│    └─ GlobalSearch・Activity Inbox・Consent Sheet・狀態預覽（真素材）
├─ 桌面角色 小樞 v2（貓系 rig：scripts/shu；Behavior Runtime：companion/behavior.ts）
│    └─ presentation.command SSE → 渲染 → /v1/presentation/ack（誠實 receipt）
├─ interaction-runtime 新模組
│    ├─ presentation.rs   Presentation Provider（7 receptors＋7 actuators、bridge、TTL sweep）
│    ├─ proactive.rs      主動對話政策（五模式、確定性頻率、跨重啟）
│    ├─ gateway.rs        Agent Gateway 接線（attach/pump/deliver/approval/kill-tree）
│    ├─ hardware.rs       metadata-only 跨平台發現（17 類覆蓋報告）
│    ├─ memory.rs         記憶分層＋保存期限＋確定性 Context Bundle
│    ├─ knowledge.rs      CAS 素材＋知識圖譜＋FTS5＋lexical-vector 候選
│    └─ curator.rs        更新決策器＋經驗轉知識＋Knowledge Receipt
├─ crates/interaction-agent-gateway（新 crate）
│    ├─ claude.rs  claude -p stream-json（plan 模式；panic-proof 解析器）
│    ├─ codex.rs   codex app-server JSON-RPC（schema 鎖定；approval→人類）
│    ├─ codex_exec.rs  codex exec --json／resume 相容 fallback
│    └─ process.rs process-group spawn＋SIGTERM→SIGKILL 樹終止
└─ storage v4→v6：memory_items／assets／knowledge_nodes+edges+FTS5／knowledge_receipts
```

信任面不變：唯一 governor、claims 永為 inference、human/agent token 分權、
estop 全鏈（含子程序樹與
presentation 佇列）、重啟不自動恢復任何可行動狀態。


## v0.5 Character Presentation Protocol（角色無關的呈現層）

唯一契約：`docs/character-protocol/README.md`；權威實作與 JSON Schema：`crates/interaction-character`
（`schemas/character-protocol.schema.json` golden）；TS 鏡射：`apps/interaction-desktop/src/character/`。

```
Runtime 真相（agent.session.state／action.*／plan.blocked／emergency／proactive／provider／observation／AI state-present）
   │  crates/interaction-runtime/src/character.rs：投影成 IntentEnvelope（intent＋truthState＋priority floor＋correlationId）
   ▼
CharacterHub（Rust Gateway）── character.intent（SSE／Tauri IPC）──▶ 桌面視窗 CharacterGateway（TS）──▶ shu-rig／sprite／text adapter
   │                          ── WebSocket /v1/character/ws（adapter token）──▶ 外部程式（examples/character-adapters）
   │  ◀── receipts（accepted→started→completed｜cancelled｜failed｜uncertain）、input events（正規化、節流、只 metadata）
   ▼
receipt 誠實結算（AI presentation command：completed→Completed AcknowledgedOnly；永不 verified）、audit、character.receipt 事件
```

- **呈現層無權限主權**：adapter 只收授權後的 intent、只送受限 event；truthState／verified 只由 Runtime 決定；
  adapter token 打不到任何人類路由。
- **誠實降級**：exact／substituted／reduced／unsupported／failed；安全 intent 沒有任何能力時落到 `system.text`
  （事件＋可信 host overlay），不會遺失。
- **可信 host 層**：estop／感測使用中／Runtime 離線由 Tauri `overlay` 視窗＋tray 顯示，內容只來自 Rust
  （renderer 視窗沒有 emit 權限）。
- **小樞＝Reference Adapter**：`src/character/adapters/shu.ts`＋`shuTables.ts` 是唯一知道耳朵／尾巴／36 表情的地方；
  Runtime 只有語意投影表。文字角色（`plain-text`）與舊 sprite pack 走同一條路。

## v0.6.0 Foundation：AIP 1.0、權威 Character Session、小樞脫核心

> 證據等級用字：本節只依 `git show <commit>` 讀取原始碼與已提交的測試檔名判斷「哪些程式碼與測試存在」，
> 本次任務未執行 `cargo`／`pnpm`／daemon／Playwright，因此**不宣稱任何測試「通過」**——通過與否以
> `docs/releases/v0.6.0-baseline.md`（修改前基線）與 CHANGELOG 之後補上的實跑回歸為準。「已落地（HEAD）」
> 只代表「對應程式碼與測試函式已提交在 `edb1682`」，不是「已驗證可運作」；真機一律「未驗證」；
> `examples/fake_iphone` 產生的一切一律標「模擬 iPhone（fixture）」。

### 1. 分層與依賴方向

```
Presentation / UI / Renderer     apps/interaction-desktop/src（React、companion 視窗、character/adapters/*）
                                  apps/interaction-ios（SwiftUI；AIP 型別已鏡射，Session client 進行中）
            ↓
Application Use Cases            crates/interaction-runtime/src/character_session.rs（Session Host：
                                  join／leave／presence／submit／resume／snapshot／diagnostics／tick）
            ↓                    crates/interaction-runtime/src/{character,mobile,agents,executor}.rs（既有真相來源）
Domain Core / Character Session  crates/interaction-session（純函式）、crates/interaction-aip（純函式）、
                                  crates/interaction-character（純函式，CPP）、crates/interaction-core
            ↓
Ports / Stable Interfaces        crates/interaction-session/src/ports.rs
                                  （Clock／SessionStore／IdentityVerifier／ConsentVerifier／EventLog／RendererPort／DevicePort）
            ↓
Adapters / Transport / Platform  mobile.rs（iPhone wss `aip` frame）、character.rs＋character_ws.rs（CPP）、
                                  interaction-api（HTTP／SSE）、Tauri src-tauri、iOS ConnectionManager、
                                  TS `character/adapters/*`（shu／sprite／text／shape）
```

依賴只能朝向核心：`interaction-session` 依賴 `interaction-aip`＋`interaction-character`（truth 詞彙）＋serde，
不依賴 tokio／axum／Tauri／SwiftUI／WebSocket 函式庫；`tests/e2e/tests/dependency_boundaries.rs` 釘住。
完整分層說明與 Ports 清單見 `docs/aip/architecture-boundaries.md`。

### 2. 新 crate

| Crate | 型態 | 責任 |
|---|---|---|
| `crates/interaction-aip` | 純函式，無 tokio／I/O | AIP 1.0：跨裝置語意訊息 envelope、12 種 message type、12 值 Outcome 誠實階梯、19 個穩定錯誤碼、版本協商、能力協商（交集＋min）、身分綁定決策、離線事件政策、canonical JSON hash、上限常數。契約：`docs/aip/README.md` |
| `crates/interaction-session` | 純函式，無 tokio／I/O | 權威 Character Session：語意狀態（mood／activity／attention／truth／members）唯一 owner、確定性 Director、revision／sequence／snapshot／patch（RFC 7396）／delta replay、有界安全管線、6 個 port trait。契約：`docs/aip/character-session.md` |
| `crates/interaction-character-shu` | 純函式 | 小樞（`shu-rig`）專屬內容：`ShuRigPack`（variants、能力集、`character-rig` 2.0 遷移）。`crates/interaction-character` 核心不再含任何小樞字串（`ValidationLimits::default().builtin_whitelist` 改為空，由 host 注入） |

### 3. Session Host 接線點（`crates/interaction-runtime/src/character_session.rs`）

Runtime 只有一個權威 `Character Session Host`（桌面 Runtime 擔任，介面不寫死）。對外方法：
`character_session_{enabled_from_env:62, join:374, leave:395, presence:410, submit:425, submit_runtime:442,
resume:476, snapshot_envelope:492, diagnostics_value:327, peek:322, tick_at:537}`（行號對應 `edb1682`）。

Runtime 事件 → Session 事件（真相唯一入口是 `submit_runtime`）：

| Runtime 事件 | Session 事件 |
|---|---|
| `agent.session.state` | `TaskState{truth}`；`verified` → `TaskVerified` |
| `emergency.stop` engage／clear | `Emergency{engaged}` |
| `POST /v1/character/hello` 協商成功 | `join(human-surface:desktop, role host-renderer)`＋`ReducedMotion` |
| watchdog tick | reacting 逾時、presence 逾時、過期 intent、離線逾時成員清除、到期持久化 |
| iPhone 斷線／撤銷 | `presence(device, offline)`／`leave(device)` |

HTTP／SSE／CLI 接線：`GET /v1/character-session`、`POST /v1/character-session/resume`、
`POST /v1/character-session/events`、`GET /v1/character-session/diagnostics`（`interaction-api/src/routes.rs`）；
SSE `character.session.state`；CLI `interact-ai character session status|diagnostics|resume`。細節見
`docs/aip/transport-bindings.md`。

### 4. iPhone `aip` frame

iPhone 線協定 v1（`mobile.rs`）新增一種 frame `{"type":"aip","envelope":{…}}`，只在 `auth-ok` 之後接受，
共用既有 v1 的 128 KiB frame 上限與 30 msg/s 連線速率窗。**沒送過 `capability` 的舊 App 永遠不會收到任何
`aip` frame**（回歸測試：`crates/interaction-runtime/tests/mobile_loop.rs::a_legacy_phone_that_never_negotiates_receives_no_aip_frames`），
`character.present` 動器與 `iphone.touch` observation 路徑完全不變。細節見 `docs/aip/transport-bindings.md` §1。

### 5. Feature flag

`INTERACT_AI_CHARACTER_SESSION`（env，預設 `1`；`character_session_enabled_from_env()`，`character_session.rs:62`）：
`0` 時 Runtime 不啟動 Session Host，四條 `/v1/character-session/*` 路由回 `503 session-disabled`，iPhone 的
`aip` frame 回 `error{unsupported-capability}`；其餘行為與 v0.5.1 相同（回退路徑）。

### 6. 已落地（HEAD `edb1682`）vs 進行中

**已落地**（程式碼與對應測試函式已提交；未在本次任務內重跑驗證）：

- `interaction-aip`、`interaction-session`、`interaction-character-shu` 三個新 crate＋golden schema＋codegen＋conformance fixture。
- 小樞脫離 `interaction-character` 核心（strangler）＋TS `character/adapterRegistry.ts`＋第二個 Reference Character `ref-shape`。
- Runtime Session Host 接線（`character_session.rs`）＋HTTP／SSE／CLI（`e71ab45 feat(runtime): host the authoritative
  Character Session and bind it to the iPhone wire protocol, HTTP, SSE and CLI`）。
- iPhone `aip` frame（同一個 commit `e71ab45`，`mobile.rs`）。
- 發布流程拆分（`release-prepare.sh`／`release-verify.sh`／`release-tag.sh`）。
- `docs/aip/*` 契約文件、`docs/releases/v0.6.0-baseline.md`（修改前基線）、`v0.6.0-recovery-matrix.md`（恢復矩陣）、
  `.claude/workflows/adversarial-review-v06.js`（**已加入，尚未執行**）。

**進行中，未驗證**（本次任務在 `apps/interaction-desktop/src`、`apps/interaction-ios` 搜尋不到對應實作，
`rg -n "characterSession|CharacterSession" apps/interaction-desktop/src apps/interaction-ios` 除 `aip/generated.ts`
的型別定義外無其他命中）：

- **桌面同步狀態文案**：`docs/aip/character-session.md` §11 定義的一般模式人話（「iPhone 已連接，角色狀態已同步」等）
  尚未出現在 `statusProjection.ts` 或任何桌面頁面；`aip/envelope.ts`／`generated.ts`（AIP 型別鏡射）與
  `aip-conformance.test.ts`／`aip-envelope.test.ts` 已存在，但沒有把 session state 接上 UI 的程式碼。
- **iOS Session client**：`apps/interaction-ios` 目前只有 AIP 型別鏡射與 conformance（`AIPEnvelope.swift`／
  `AIPGenerated.swift`／`AIPConformanceTests.swift`／`AIPFixtures.swift`），沒有加入 session、渲染 semantic state
  的 client 程式碼。
- **對抗審查**：workflow 檔已提交，尚未執行（無 finding 記錄）。

**文件落後程式碼的已知落差**：`CHANGELOG.md` 的 `[Unreleased]` 段最後一次更新（`336a6b6 docs(changelog): open
the v0.6.0 Foundation section with the landed wave-1 facts`）早於 `94abf5c`（TS migrator registry 鏡射收尾）、
`e71ab45`（Session Host＋iPhone `aip` frame＋HTTP／SSE／CLI）與 `edb1682`（桌面 lockfile 更新）三個 commit，
因此目前 CHANGELOG 正文仍寫「Runtime Session Host、iPhone `aip` frame……在落地前不會出現在這裡」，但這兩項
的程式碼與測試函式**已經**在 HEAD 上；這是文件更新滯後於程式碼，不是功能缺失，記錄於此供補寫 CHANGELOG 時參考
（CHANGELOG.md 不在本輪文件任務的可修改清單內，未逕行修改）。
