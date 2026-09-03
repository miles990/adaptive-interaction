# 架構總覽（v0.5 開發中；v0.3 基線＋v0.4／v0.5 增量）

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
| `interaction-character` | **Character Presentation Protocol 1.0**（純函式）：manifest 驗證／migration、能力協商、intent／truthState／priority floor、input 正規化、回執狀態機、wire messages、`Gateway`；JSON Schema 來源 |
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
