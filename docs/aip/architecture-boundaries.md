# v0.6.0 Foundation：架構邊界、Ports／Adapters、遷移計畫

## 1. 分層與依賴方向

```
Presentation / UI / Renderer     apps/interaction-desktop/src（React 頁面、companion 視窗、adapters/*）、apps/interaction-ios Views
            ↓
Application Use Cases            crates/interaction-runtime/src/character_session.rs（Join／Leave／Submit／Snapshot／Resume／Attach／Cancel／Diagnostics）
            ↓                    crates/interaction-runtime/src/{character,mobile,agents,executor}.rs（既有）
Domain Core / Character Session  crates/interaction-session（純）、crates/interaction-aip（純）、crates/interaction-character（純，CPP）、crates/interaction-core
            ↓
Ports / Stable Interfaces        crates/interaction-session/src/ports.rs（Clock、SessionStore、IdentityVerifier、ConsentVerifier、EventLog、RendererPort、DevicePort）
            ↓
Adapters / Transport / Platform  mobile.rs（iPhone wss）、character.rs＋character_ws.rs（CPP）、Tauri src-tauri、iOS ConnectionManager、TS adapters（shu／sprite／text／shape）
```

依賴只能朝向核心：`interaction-session` 依賴 `interaction-aip`＋`interaction-character`（truth 詞彙）＋serde；
**不得**依賴 tokio、axum、Tauri、SwiftUI、WebSocket／BLE／MQTT／Serial 函式庫、小樞專屬欄位、Live2D、sprite 引擎、OS 通知 API。
`cargo tree -p interaction-session` 與 `-p interaction-aip` 的結果由 `tests/e2e/tests/dependency_boundaries.rs` 釘住。

## 2. Ports（名稱依 repo 慣例）

| Port | 位置 | 責任 | 可選能力表達方式 |
|---|---|---|---|
| `CharacterAdapter` | CPP `CharacterManifest`＋TS `CharacterAdapter`（既有） | 角色 package → renderer 實例 | manifest capabilities |
| `RendererAdapter` | `interaction-session::ports::RendererPort`（Rust）；TS `CharacterAdapter`（既有） | 收 semantic state＋Behavior Intent、本地播放、降級 | AIP `capability` 協商 |
| `DeviceAdapter` | `interaction-session::ports::DevicePort` | 產生 event、接收 state／command、presence | AIP `capability` |
| `TransportAdapter` | 各 transport 自有（mobile.rs、character_ws.rs）；共同外層是 AIP binding（`docs/aip/README.md` §9） | framing、重連、退避 | — |
| `AgentAdapter` | `interaction-agent-gateway`（既有） | 真 Agent 連接 | — |
| `MemoryProvider` | `runtime/memory.rs`（既有） | 長期記憶 | — |
| `SessionStore` | `interaction-session::ports::SessionStore` | snapshot 持久化：`save → Result<SaveOutcome, PortError>`（`Written`／`SkippedStale`／`SkippedParked`），`(epoch, revision)` 不得倒退且檢查＋寫入原子；`load` 有界讀取並區分 `Corrupt`／`Unavailable`／`FutureFormat`。production：`JsonSessionStore`；測試：`MemoryStore`（守同一條 guard） | — |
| `Clock` | `interaction-session::ports::Clock` | 注入時間（millis） | — |
| `IdentityVerifier` | `interaction-session::ports::IdentityVerifier` | Transport 身分 vs `source` | — |
| `ConsentVerifier` | `interaction-session::ports::ConsentVerifier` | `consentGrantId` 有效性（1.0 只對帶 grant 的 command） | — |
| `EventLog` | `interaction-session::EventLog`（有界環，內建實作） | delta replay | — |

避免巨大 `Provider` 介面：每個 port 只有 3–6 個方法；可選能力一律靠 capability 協商而不是 optional method。

## 3. Adapter lifecycle（外層穩定語意）

`Registered → Initializing → Starting → Ready ⇄ Degraded → Stopping → Disposed`，另 `Cancelled`／`Failed`／`Unavailable`／`Removed`。
各 adapter 內部流程自訂，但必須映射到這些外層狀態（CPP 的 14 態 `AdapterLifecycleState` 由 `statusProjection.projectCharacterLifecycle` 映射，既有）。
移除時：停新工作 → 取消進行中 → 關 subscription／timer／listener／rAF／physics → 釋放資產 → 關連線 → 更新 capability registry →
撤銷失效權限 → 清 presence → 使用者資料保留或明確處理 → 不留幽靈 UI／假可用 capability。contract test：`src/test/adapter-contract.test.ts`
對 shu／sprite／text／shape 四個 adapter 跑同一套。

## 4. 小樞脫離核心（Strangler）

| 現況（v0.5.1） | v0.6.0 |
|---|---|
| `interaction-character/src/manifest.rs`：`BUILTIN_ENTRYPOINTS = ["shu-rig","sprite","text"]`、`SHU_RIG_VARIANTS`、`shu_rig_capabilities()`、`migrate_rig_pack()` | 移到新 crate `crates/interaction-character-shu`（`ShuRigPack`：variants、capabilities、`migrate_rig_pack`）。核心 `ValidationLimits.builtin_whitelist` 由 host 注入（Tauri／runtime／TS 各自從 registry 取）；核心 `migrate_pack_to_manifest` 改為 `PackMigrator` registry（`register_migrator`），shu 的 rig 2.0 遷移由 shu crate 註冊。核心 crate 內 `rg -w 'shu|maid'` 只允許出現在測試 fixture |
| TS `protocol.ts BUILTIN_ENTRYPOINT_IDS`、`CompanionApp.tsx` 依 entrypoint 三向 if | `character/adapterRegistry.ts`：`registerBuiltinAdapter(id, factory)`；shu／sprite／text／shape 各自在自己的模組註冊；CompanionApp 只呼叫 `createAdapter(entrypoint)`；白名單＝registry keys |
| Tauri `BUNDLED_CHARACTER_IDS` 硬編 9 個 | 從 `public/characters/index.json` 產生的常數（build script 或測試釘住一致），新增 `ref-shape` |
| 桌面主入口第二項標籤 | 已是「目前角色」動態名（`useCharacterName`），保留；預設仍顯示「小樞」 |
| （v0.6.x）`companion/settingsTransfer.ts` 以 `SHU_RIG_PALETTES`＋硬編 persona 清單做全域驗證；`CompanionPage.tsx` 以 `isShuRig` 字面分岔掛小樞遊玩場 UI | 驗證綁定**目標角色的 adapter meta**（`personas`／`variants`／`hasPlayfield`）；小樞遊玩場 UI 搬進 `character/adapters/shuPlayControls.tsx` 並由 `SHU_META.playfieldControls` 宣告；守門測試擴大到頁面層（`pages/CompanionPage.tsx`、`pages/character/**`，`CharacterPreview.tsx`／`CharacterLibrary.tsx` 暫列待收斂棘輪） |

新增第二 Reference Character：`ref-shape`（幾何形，`entrypoint builtin:shape`，只宣告 `visual.presence`／`visual.expression`（variants=四個 intent）／`input.click`，
無耳尾、無玩具）。驗收：加入它**不改** `interaction-character`、`character.rs`、`CompanionApp.tsx` 任何 switch-case（用 `git diff --stat` 證明）。

#### 實作註記（v0.6.0 `refactor(character)` 實作時補）

1. **函式名**：本表寫的 `migrate_pack_to_manifest` 在 v0.5.1 的實際名稱是
   `interaction_character::migrate_legacy_pack`。實作採「新函式＋舊函式 deprecated」：新增
   `migrate_pack_to_manifest(json, &MigrationRegistry)`；`migrate_legacy_pack` 標 `#[deprecated]`
   且只剩通用 sprite（核心不能依賴任何角色 crate）。
2. **Tauri 依賴**：`src-tauri` 沒有加 `interaction-character-shu` 依賴，改為直接用
   `interaction_runtime::character::character_host_registry()`。桌面 host 只有一份白名單／migrator
   registry，Tauri 匯入路徑與 Runtime 不可能漂移；`interaction-character-shu` 由 runtime 傳遞。
3. **TS 白名單與工廠分離**：`adapterRegistry.ts` 用 `BUILTIN_ADAPTER_IDS`（宣告）決定白名單，
   工廠由 `adapters/index.ts` 註冊。理由：`manifest.ts` 的驗證不能因為 adapter 模組有沒有被
   import 而給出不同答案。「宣告了卻沒註冊工廠」由 `adapter-contract.test.ts` 擋。
4. **`ref-shape` 的 variants**：本文寫「variants=四個 intent」，實作依本輪任務書採 10 個
   （idle／notice／play／rest／work／think／acknowledge／wait／greet／sleep）。其中 `wait` 是 CPP
   的**安全** intent（floor 60），所以它只出現在 `visual.expression.variants`，**沒有**列進 manifest
   的 `intents`；協商時仍落 `system.text`。細節見 `docs/aip/reference-character.md` §4。
5. **entrypoint 分岔的例外**：`CharacterSource.kind`（`index`／`legacy-pack`／`imported`／`text` 退路）
   是**來源**判別標籤，與有哪些 adapter 無關，架構守門測試不把它算成 entrypoint 分岔。

### 4.1 Runtime 核心去特定裝置耦合（provider 能力宣告）

`refactor(character)` 之後仍有第二條「核心只理解一種特定裝置」的耦合：Runtime 核心以 `iphone.*`／
`companion.` 這類**能力 id 字面前綴**判斷跨切面語意。v0.6.0 一併改成宣告驅動——provider 註冊時
自己說明，核心只查表。

| 現況（v0.5.1） | v0.6.0 |
|---|---|
| `character.rs::is_presentation_surface_actuator` 硬編 `starts_with("companion.")` ＋ `== "iphone.character"` | 改成 `is_presentation_surface_actuator(&ProviderCapabilityRegistry, actuator_id)`；呈現面由 provider 宣告（`presentation_surfaces`），沒宣告過的動器一律是一般動器 |
| `activity.rs::sensor_event_label` 以 `sensor.starts_with("iphone.")` 決定人話標題「iPhone」 | 由「宣告了這個受器的 provider」提供 `class_label`；沒有宣告就退回中性字樣（內建本機感測器仍走 `sensor_display_name`） |
| `sensors.rs::emit_stop_sensor_events` 直接寫 `"sensor": "iphone.mic-level"`，並以 `crate::mobile::StopOutcome` 比對 | 改成通用 trait `sensors::SensorStopOutcome`（`source_id`／`sensor_ids`／`outcome_label`／`waited_ms`／`confirmed_stopped`）＋純函式 `sensor_stop_uncertain_payloads`；受器 id 來自 provider 宣告的高風險受器清單 |
| `providers.rs` 自己組 `provider.mobile.<deviceId>` | id 命名規則移到 `mobile::mobile_provider_id`（字串格式不變，前端／API 測試把它當契約）；`MOBILE_PROVIDER_ID_PREFIX` 是唯一產生點 |

**Ports**：`crates/interaction-runtime/src/providers.rs` 新增 `CapabilitySelector`（`Exact`／`Prefix`）、
`ProviderCapabilityDeclaration`（`class_label`／`presentation_surfaces`／`receptors`／`high_risk_receptors`）、
`ProviderCapabilityRegistry`（同步 `RwLock`，每個 `declaration_id` 一筆）。內建宣告在
`Runtime::init_providers()` 一次登記完（`companion_capability_declaration()`＋
`mobile::mobile_capability_declaration()`）——**與伺服器有沒有起來、有沒有配對過裝置無關**：核心對
能力 id 的理解不能依賴某個 provider 剛好在線上。

驗收：`rg -n '"iphone[.-]|provider\.mobile\.' crates/interaction-runtime/src --glob '!mobile.rs'` 為 0 命中。

#### 實作註記

1. **宣告表的存放位置**：投影路徑（`character_project_action`）是同步的、沒有 await 點，所以宣告表
   必須同步可讀。v0.6.0 時暫掛在 `CharacterHub` 上；**v0.6.x 起已搬成 `RuntimeInner` 的欄位**（與 `registry`／
   `providers` 平行，仍是 `std::sync::RwLock`）。讀者（`character.rs`／`activity.rs`／`sensors.rs`）只拿
   `&dyn CapabilityDeclarationsView`（is_presentation_surface／class_label_of_receptor／high_risk_receptors／
   declaration_ids／declaration）；`declare`／`retract` 為 pub(crate)，寫入只經 `Runtime::declare_provider_capabilities`／
   `retract_provider_capabilities`。刻意**不**併進 `ProviderGate`：那是個體、可變、async 的運行期閘門；宣告表是家族、
   靜態、sync 的描述。單台裝置的 disable／revoke **不**動家族宣告（撤銷最後一支 iPhone 後 `provider.mobile` 的高風險受器
   宣告仍在，否則 stop-all 會從此不知道 `iphone.mic-level` 是高風險）。維運可讀：`GET /v1/providers/declarations`／
   `interact-ai providers declarations`（唯讀，無 HTTP 寫入入口）。命名提醒：`interaction_registry::CapabilityRegistry`
   （每個受器／動器的 enabled 旗標）與 `ProviderCapabilityRegistry`（provider 家族的語意宣告）是兩個不同的表。
2. **`StopAllSensorsReport.devices` 仍是 `Vec<MobileStopOutcome>`**（wire 形狀不動），**但停止已經只有一個協調器**
   （v0.6.x，`Runtime::stop_all_sensor_sources`）：`crates/interaction-runtime/src/sensor_source.rs` 的 `SensorSource` port
   （`source_id`／`declaration_id`／`active_captures`／`request_stop(target, deadline, reason)`／`release`）＋有界登記表
   （上限 32，超過拒絕並稽核）。本機麥克風（`LocalMicSensorSource`）與 iPhone（`MobileSensorSource`）都是登記進來的一般來源，
   核心沒有裝置特例分支；`emergency_stop` 與停止按鈕呼叫同一個協調器（X1：對「宣告了高風險受器卻沒有來源涵蓋」回同樣的
   `uncertain`＋no-stop-path）；通用 `revoke_provider`／`transition_provider(Disabled)` 對登記了來源的 provider 指名 target
   走同一條 `request_stop`＋`release`（X2，結果寫進稽核）；`Runtime::unregister_receptor` 對高風險受器先請來源停止再移除
   （S4）。結果五態 `stopped`／`already-stopped`／`unknown`／`unreachable`／`refused`，只有前兩者算確認；非 mobile 來源的逐筆
   結果進新增的 `sources[]`（非空才序列化），`stopped`／`uncertain` 同時涵蓋 `devices`＋`sources`＋無停止管道的受器；來源被
   移除時仍在擷取的感測以有界可見的 stop-unknown 留在 activeSensors（感測不靜默）。第三波的宣告式 adapter 實作同一個 port。
3. **宣告是動態的**：`Runtime::declare_provider_capabilities()` 是公開 API，測試與未來的動態 provider
   都可以在執行期登記自己的呈現面／高風險受器，不必改核心的任何分支。

## 5. Feature flags 與相容

- `INTERACT_AI_CHARACTER_SESSION`（env，預設 `1`）：`0` 時 Runtime 不啟動 Session Host，所有 `/v1/character-session/*` 回 503 `session-disabled`，
  iPhone 的 `aip` frame 回 `error{unsupported-capability}`；其餘行為與 v0.5.1 相同（回退路徑）。
- iPhone 舊 App：不送 `capability` 就永遠不收 `aip` frame；`character.present`／`iphone.touch` 路徑不變。
- 舊 Character Pack／manifest：遷移路徑不變（由 shu crate 註冊）。
- 舊 `DesktopPrefs.companion_pack`：不變。

## 6. 提交順序（每個可獨立驗收）

1. `test(baseline)` 基線文件＋恢復矩陣。
2. `docs(aip)` 契約（本目錄）。
3. `feat(protocol)` `interaction-aip` crate＋golden schema＋codegen＋conformance。
4. `feat(session)` `interaction-session` crate。
5. `refactor(character)` 小樞抽出核心＋adapter registry＋`ref-shape`。
6. `feat(runtime)` Session Host＋HTTP／SSE＋CharacterHub 投影。
7. `feat(mobile)` iPhone `aip` frame＋身分綁定＋resume＋fake_iphone ops。
8. `feat(desktop)` 同步狀態文案＋journey tests（390px／a11y）。
9. `feat(ios)` Session client＋CharacterView 語意呈現＋XCTest。
10. `test(adversarial)`／`fix(...)` 對抗審查與修復。
11. `docs(release)` 證據、已知限制、CHANGELOG；`chore(release)` release.sh 拆成 prepare／verify／tag。
