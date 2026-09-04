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
| `SessionStore` | `interaction-session::ports::SessionStore` | snapshot 持久化 | — |
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

新增第二 Reference Character：`ref-shape`（幾何形，`entrypoint builtin:shape`，只宣告 `visual.presence`／`visual.expression`（variants=四個 intent）／`input.click`，
無耳尾、無玩具）。驗收：加入它**不改** `interaction-character`、`character.rs`、`CompanionApp.tsx` 任何 switch-case（用 `git diff --stat` 證明）。

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
