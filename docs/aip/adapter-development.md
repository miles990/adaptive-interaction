# 怎麼加一個角色／Renderer／裝置／Transport（v0.6.0 之後）

> 證據等級用字：本文件是操作導覽，多數規則引用既有契約與 `docs/aip/reference-character.md`
> 已經走過一次的實例（`ref-shape`）；沒有測試支持的通用建議標「建議做法」，不寫成強制規則。

## 1. 加一個新角色

完整步驟與「沒改核心」的證明見 `docs/aip/reference-character.md`（`ref-shape` 是這條路徑唯一
走完整趟的實例）。摘要三個擴充點：

1. **Manifest**：`apps/interaction-desktop/public/characters/<id>/manifest.json`＋在
   `public/characters/index.json` 多一列。
2. **Host 白名單**（兩處，不可能漂移地各自維護）：
   - Rust：`interaction_runtime::character::character_host_registry()` 的
     `CHARACTER_BUILTIN_ENTRYPOINTS` 多一個 id；若角色有專屬能力集／遷移邏輯，
     像小樞一樣獨立成一個 `interaction-character-*` crate（純資料＋純函式，不依賴 tokio／I/O，
     見 `crates/interaction-character-shu` 的形狀）。
   - TS：`character/adapterRegistry.ts` 的 `BUILTIN_ADAPTER_IDS` 多一個宣告；
     `registerBuiltinAdapter(id, factory, meta)` 在 `character/adapters/index.ts` 註冊工廠
     （宣告與工廠分離：宣告了卻沒註冊工廠由 `adapter-contract.test.ts` 擋，不是靜默失敗）。
3. **只有兩個不同 Adapter 都需要相同新語意才改 Core** ——這是任務書要求的規則，來源是
   `docs/aip/architecture-boundaries.md` §1 的依賴方向與 §4 的 strangler 精神：協定核心
   （`interaction-character`／`interaction-aip`／`interaction-session`）不認識任何具名角色、
   不含任何角色專屬字串（`rg -n -i 'shu|maid' crates/interaction-character/src` 必須是 0 命中，
   `docs/aip/reference-character.md` §5 的驗收方式）。單一角色需要的新能力，一律先在
   角色自己的 crate／adapter 模組實作；只有當**第二個**獨立角色也需要同一段語意（例如
   一種新的 canonical capability id、一種新的 intent 詞彙）時，才提升進協定核心，且要走
   `docs/aip/compatibility.md` §2 的 minor 演進規則（新增而非修改既有語意）。

測試清單（新角色至少要有）：`adapter-contract.test.ts` 加入這個新 id 的 contract 案例
（見 `docs/aip/renderer-adapter.md` §5）；一份角色專屬測試（manifest 與 adapter 定義一致、
CPP intent 逐一送、輸入能力）；`architecture-no-entrypoint-switch.test.ts` 之類的守門測試不需要
為新角色新增案例——它讀的是「host 端有沒有字面分岔」，新角色如果照 registry 模式加入，
不會觸發任何新的字面分岔。

## 2. 加一個新 Renderer（外部 adapter，非 in-process builtin）

外部 transport 目前只有一種具體實作：WebSocket（`GET /v1/character/ws?token=<adapter token>`，
`crates/interaction-api/src/character_ws.rs::character_ws_loop`）。加一個新的外部 renderer
**不需要改任何協定程式碼**——它是這條既有管道的一個新客戶端：

1. 用 `POST /v1/character/adapters {displayName, manifest}`（human token）註冊，拿到
   `adapterId`／`token`。
2. 連 `GET /v1/character/ws?token=<token>`，依 CPP §3.3 握手（`hello`→`negotiate`→`negotiated`）。
3. 收 `intent{envelope}`（Behavior Intent 的 CPP 投影），回 `receipt{receipt}`。
4. 若也要參與 AIP Character Session（收語意 state／發互動事件），需要另外走
   `docs/aip/transport-bindings.md` 描述的路徑之一——**目前只有 iPhone wss 這一條路走過
   AIP frame**；外部 WebSocket adapter（`GET /v1/character/ws`）**不**承載 AIP
   （`docs/aip/README.md` §9 表格最後一列：「不承載 AIP；外部 renderer 仍走 CPP wire」）。

Contract test：`src/test/adapter-contract.test.ts`（見 `docs/aip/renderer-adapter.md` §5）針對
in-process builtin adapter；外部 transport 的等價驗收是
`scripts/v03-cli-e2e.sh`「Character Protocol」段（Node fixture 走 WebSocket，標示模擬 adapter，
`docs/character-protocol/README.md` §13）。

## 3. 加一個新裝置（Device Profile）

見 `docs/aip/device-profile.md` §6：1.0 只有 iPhone 一個實作，ESP32／BLE／Serial 尚未接上 AIP。
若要加：

1. 在對應的 Transport 層（例如 `crates/interaction-adapter-declarative` 或它的 Runtime 接線）
   仿照 `crates/interaction-runtime/src/mobile.rs` 的 `Some("aip") if authed.is_some()` 分支
   （:3491-3510）加一個 AIP frame 分派點：只在該 Transport 自己的認證完成後才接受，
   身分綁定用該 Transport 自己的配對／認證機制。
2. 呼叫 `Runtime::character_session_device_frame`（或對應的等價入口）把已驗證身分與 envelope
   交給 `interaction-session`（不得繞過它自己實作一套安全檢查——安全管線只有一份，在
   `crates/interaction-session/src/session.rs::gate`）。
3. 依 Device Profile 宣告 `capability`（`role`／`inputs`／`intents`／`features`），
   不新增協定層級的欄位（除非兩個裝置都需要，見 §1 的提升規則）。
4. 補 fixture（仿照 `crates/interaction-runtime/examples/fake_iphone.rs` 的 `aip-*` op，
   `docs/aip/transport-bindings.md` §6）與端到端測試（仿照
   `crates/interaction-runtime/tests/character_session_loop.rs` 的 helper 與案例）。

## 4. 加一個新 Transport（AIP binding 規則）

`docs/aip/README.md` §9 定義「什麼算一個 Transport binding」；新增一個時：

- **語意不變**：framing、重連、退避、速率窗是 Transport 自己的事；訊息語意、錯誤碼、
  outcome 階梯、revision／sequence 規則一律由 AIP 決定，不得為了適配新 Transport 而修改
  `crates/interaction-aip` 或 `crates/interaction-session` 的規則。
- **沒有可插拔的 `CharacterTransport` trait**：`interaction-character`／`interaction-aip`／
  `interaction-session` 都是純函式 crate，沒有 tokio、沒有 I/O（`docs/character-protocol/README.md`
  §8.1：「本版沒有可插拔的 `CharacterTransport` trait」，這句話對 AIP 同樣成立）。新 Transport
  要在 `interaction-runtime`／`interaction-api` 層仿照既有迴圈實作（`character_ws_loop`
  或 `mobile.rs` 的連線迴圈），不是在協定核心裡定義 trait。
- **更新 `docs/aip/transport-bindings.md`**：新增一列到 §0（身分對照表）與相應的訊息表；
  更新 `docs/aip/compatibility.md` §1「協定相容矩陣」如果新 Transport 引入了新的實作分佈。

## 5. 刪除／停用的 lifecycle 與 deprecation 週期

- **角色／Adapter 停用**：CPP `Removed` lifecycle 狀態（`docs/character-protocol/README.md` §7
  的 adapter lifecycle 圖：「另有 `crashed`／`reconnecting`」，`docs/aip/architecture-boundaries.md`
  §3 補充了外層穩定狀態機 `Registered → Initializing → Starting → Ready ⇄ Degraded → Stopping
  → Disposed`，另 `Cancelled`／`Failed`／`Unavailable`／`Removed`）；移除時要求：停新工作 →
  取消進行中 → 關 subscription／timer／listener／rAF／physics → 釋放資產 → 關連線 → 更新
  capability registry → 撤銷失效權限 → 清 presence → 使用者資料保留或明確處理 →
  不留幽靈 UI／假可用 capability。這套契約由 `adapter-contract.test.ts` 的
  「資源清理：dispose 後 timer／rAF／DOM listener 都回到原本水位」（:333，見
  `docs/aip/renderer-adapter.md` §5）驗證，但那是**內建** adapter 的 contract test；
  外部／第三方 adapter 沒有自動化工具強制它遵守同一份 lifecycle 契約，只有文件約束
  （§9 分級：可執行 adapter 必須顯示來源、作者、版本、能力、網路需求、資料範圍，
  並提供停用／撤銷／移除／fallback，`docs/character-protocol/README.md` §9）。
- **協定欄位／name／capability 的 deprecation**（`docs/aip/README.md` §4.1）：標 `deprecated` →
  至少跨一個公開 minor → 提供 compatibility adapter 與 `aip.deprecated-used` diagnostics
  warning → 更新 `docs/aip/compatibility.md` §3 表 → 才可移除。**目前該表是空的**——1.0 還沒有
  任何欄位、name 或 capability被標記為即將移除。
- **Rust 函式層級的 deprecation 先例**：`migrate_legacy_pack`
  （`crates/interaction-character/src/manifest.rs:1719`，`#[deprecated(since="0.6.0", …)]`）
  是「新函式＋舊函式繼續可用但標記」模式的範例——舊呼叫端不會編譯失敗，只會在建置時看到
  deprecation 警告；這與協定層級的 deprecation（欄位／name／capability）是兩件不同的事，
  前者是程式碼 API 的演進紀律，後者是 wire 協定的演進紀律，本文件把兩者分開說明避免混淆。
