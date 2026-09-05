# 怎麼加一個角色／Renderer／裝置／Transport（v0.6.0 之後）

> 證據等級用字：本文件是操作導覽，多數規則引用既有契約與 `docs/aip/reference-character.md`
> 已經走過一次的實例（`ref-shape`）；沒有測試支持的通用建議標「建議做法」，不寫成強制規則。

## 1. 加一個角色（1a）／加一個 adapter（1b）

這是**兩件事**，代價差很多。它們過去寫成同一步，是因為 `ref-shape` 當初「新角色＋新 adapter」
一起加（`docs/aip/reference-character.md` §5 的 8 項清單）。分界線只有一句話：
**host 白名單的鍵是 entrypoint，不是角色 id**——重用既有 adapter 的角色不需要任何白名單成員。
實測見 `docs/releases/v0.7.0-drills.md` §1（加一個重用 `shape` adapter 的角色，兩份白名單都不在 diff 裡）。

### 1a. 加一個角色（重用既有的 in-process adapter）

三個檔，**兩份 host 白名單一個都不用動**：

1. **Manifest**：`apps/interaction-desktop/public/characters/<id>/manifest.json`。
   `entrypoint` 指向既有的 builtin id（`shu-rig`／`sprite`／`text`／`shape`）；能力集只能宣告
   那個 adapter 真的做得到的事。
2. **索引**：`public/characters/index.json` 多一列（沒有 sprite sheet 的角色不帶 `assetBase`）。
3. **一支角色專屬測試**：manifest 通過 §2.1 驗證、能力集與 adapter 一致、CPP intent 逐一送、
   輸入能力。順帶釘住「兩份白名單沒被動過」，讓「加角色不必動白名單」變成可執行的事實。

不需要動的：`adapter-contract.test.ts`（它測的是 **adapter** 的生命週期契約，不是角色）、
`character/adapters/index.ts`、`CompanionApp.tsx`、任何 Rust。角色身分由 adapter 從 ctx 取
（manifest ＞ 清單摘要），所以新角色以自己的 `characterId` 上線，不會被同一個 adapter 的
其他角色冒名。出貨角色的數量只寫在 `src/test/character-manifests.test.ts` 的
`SHIPPED_CHARACTER_IDS` 一處。

### 1b. 加一個新 adapter（新的 entrypoint）

只有這一種才會動到 host 白名單：

1. **Host 白名單**（兩處，不可能漂移地各自維護）：
   - Rust：`interaction_runtime::character::character_host_registry()` 的
     `CHARACTER_BUILTIN_ENTRYPOINTS` 多一個 id；若角色有專屬能力集／遷移邏輯，
     像小樞一樣獨立成一個 `interaction-character-*` crate（純資料＋純函式，不依賴 tokio／I/O，
     見 `crates/interaction-character-shu` 的形狀）。
   - TS：`character/adapterRegistry.ts` 的 `BUILTIN_ADAPTER_IDS` 多一個宣告；
     `registerBuiltinAdapter(id, factory, meta)` 在 `character/adapters/index.ts` 註冊工廠
     （宣告與工廠分離：宣告了卻沒註冊工廠由 `adapter-contract.test.ts` 擋，不是靜默失敗）。
2. **Contract 案例**：`adapter-contract.test.ts` 加入這個新 id 的案例
   （見 `docs/aip/renderer-adapter.md` §5）——四個內建 adapter 共用同一套生命週期與資源清理契約。
3. 通常還會**順便**加一個用它的角色，那部分照 1a 走。

### 1a／1b 共同的核心規則

**只有兩個不同 Adapter 都需要相同新語意才改 Core** ——這是任務書要求的規則，來源是
`docs/aip/architecture-boundaries.md` §1 的依賴方向與 §4 的 strangler 精神：協定核心
（`interaction-character`／`interaction-aip`／`interaction-session`）不認識任何具名角色、
不含任何角色專屬字串（`rg -n -i 'shu|maid' crates/interaction-character/src` 必須是 0 命中，
`docs/aip/reference-character.md` §5 的驗收方式）。單一角色需要的新能力，一律先在
角色自己的 crate／adapter 模組實作；只有當**第二個**獨立角色也需要同一段語意（例如
一種新的 canonical capability id、一種新的 intent 詞彙）時，才提升進協定核心，且要走
`docs/aip/compatibility.md` §2 的 minor 演進規則（新增而非修改既有語意）。

`architecture-no-entrypoint-switch.test.ts` 之類的守門測試兩種情況都不需要新增案例——它讀的是
「host 端有沒有字面分岔」，照 registry 模式加入的角色或 adapter 不會觸發任何新的字面分岔。

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

見 `docs/aip/device-profile.md` §5–§6：現在有兩條裝置 binding——iPhone wss v1 與宣告式裝置線
v1.1／v1.2（Serial／MQTT／BLE；Serial 經 pty 模擬器驗證，真板為零）。

**如果你的裝置走宣告式 adapter**（YAML spec，transport 是 serial／mqtt／ble）：什麼都不用寫。
`crates/interaction-adapter-declarative` 對每個 `DeviceLink<L>` 自動建一個 `AipChannel<L>`
（型別抹除的 `DeviceAipChannel`），`register_declarative_spec` 會宣告能力、登記 `SensorSource`、
綁定收送迴圈（`crates/interaction-runtime/src/declarative_session.rs`）。裝置端只要在 hello＋配對之後
收發一行 `{"type":"aip","envelope":{…}}`（參考韌體 `firmware/esp32-companion/` 目前只忽略入站 aip，
要真的成為成員得自己實作 capability／touch）。它的身分強度是 `transport-hello+device-side-pairing`
（`device-profile.md` §3）——寫文件、稽核、UI 時用這個字串，不要寫成「已驗證身分」。

**大訊息（裝置線 v1.2 的分片）**：一則 `state{kind:"snapshot"}` 實測 814–1019 bytes，放不進 serial／
MQTT 的 639 bytes、更放不進 BLE 的 480 bytes（`device-profile.md` §6.2 的實測表）。裝置若**真的**做得到
重組，就在 `hello.caps` 加一項 `"aip.frag/1"`，host 會自動把超長的 envelope 切成 `aip-frag` 送過去
（`device-profile.md` §6.3）。

誠實地不宣告它是完全可以的——**參考韌體就沒有宣告**（它沒有那塊緩衝）。此時 host 的行為與 v1.1
完全相同：放不進行上限的 envelope 一個位元組都不寫，稽核 `over-line-limit-no-fragmentation`，
而這個成員在 diagnostics／status 上是 `intent-only`（或 `event-source`），**介面不得顯示「已同步」**。
反過來，宣告一項自己做不到的能力是最糟的選擇：host 會把訊息切好送出去、對端全部丟掉，
而收據上寫著「已送出」。

重組必須跟 host 同一套規則（`crates/interaction-adapter-declarative/src/fragment.rs`；
`scripts/esp32-serial-sim.py` 是一份可以照抄的參考實作）：每條線 1 筆進行中、重組上限 8 KiB、
片數上限 64、自最後一片起 2 秒逾時、crc32 核對，缺片／重片／亂序／截斷一律**整筆丟棄並留痕**——
半份 envelope 交給上層，會把「傳輸壞了」演成「對方說了一句沒有意義的話」。

**如果是第三種 Transport**：

1. 實作 `interaction_adapter_declarative::protocol::DeviceAipChannel`（或仿照
   `crates/interaction-runtime/src/mobile.rs` 的 `Some("aip") if authed.is_some()` 分支）：**只在該
   Transport 自己的認證完成後才放行** aip，身分綁定用它自己的配對／認證機制；被擋下的要計數＋稽核。
   出站 envelope 也要過 AIP profile 驗證與大小上限（`MAX_AIP_ENVELOPE_BYTES` 之下再守傳輸自己的上限），
   送不出去要稽核 `aip.outbound-undeliverable`，不能只落 log。若這個 Transport 有單則上限，
   實作 `DeviceOutbound::max_line_bytes()`／`supports_fragmentation()`（預設是「沒有上限、不會重組」）——
   Runtime 用它們推導成員的 `syncProfile`（`device-profile.md` §3.1）。回錯了不會壞掉，
   但介面會替一台其實什麼都沒收到的裝置顯示「已同步」。
2. 呼叫 `Runtime::character_session_device_frame` 把已綁定的身分與 envelope 交給
   `interaction-session`（不得繞過它自己實作一套安全檢查——安全管線只有一份，在
   `crates/interaction-session/src/session.rs::gate`）；斷線→`Presence::Reconnecting`（既有 tick 轉
   Offline／leave）；provider 撤銷→leave＋retract＋unregister。
   **綁定不成立時要拒收入站 frame**（AIP 1.0 澄清／v0.7.0）：abort 一條 task 只在下一個 await 點
   生效，一則已經在處理中的 frame 會照樣跑完，於是裝置會在 `character.session.leave` 之後又
   join 回來。宣告式那一條線的做法是在 `on_aip` 開頭問一次綁定生命週期，不是 `Bound` 就拒收並
   稽核 `aip.rejected{stage:"provider-binding"}`（`device-profile.md` §6.1 步驟 1）。
3. 依 Device Profile 宣告 `capability`（`role`／`inputs`／`intents`／`features`），
   不新增協定層級的欄位（除非兩個裝置都需要，見 §1 的提升規則）。
4. 補 fixture（`crates/interaction-runtime/examples/fake_iphone.rs` 或 `scripts/esp32-serial-sim.py`
   的 `aip-*` 控制指令，`docs/aip/transport-bindings.md` §6／§8）與端到端測試（仿照
   `crates/interaction-runtime/tests/character_session_loop.rs`／`declarative_session_loop.rs`）。
   結果一律標「模擬器／fixture」；真機為零就寫零。

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

- **宣告式裝置的停用不是終局**（AIP 1.0 澄清／v0.7.0）：`Disabled` 與「連線沒了」都是可以
  重新綁定的原因，`Revoked`／`Removed` 不是。把 provider 轉回一個活著的狀態只代表「允許再試
  一次」——它會啟動一條有界的背景重新綁定，期間 `ProviderState` 誠實地是 `Disconnected`，
  握手成功之後才收斂成 `Available`。八個步驟與稽核事件見 `docs/aip/device-profile.md` §6.1。
  寫 adapter 時要記得兩件事：(a) 重新綁定會**整份重新註冊 spec**，所以能力回到剛啟動時的預設
  ——需要 consent 的受器仍然是關的；(b) 停用／撤銷的順序是「先問來源停止 → 再關連線 →
  最後翻旗標」，先關連線就再也問不出「你停了嗎」。

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
