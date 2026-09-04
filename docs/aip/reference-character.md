# 加一個 Reference Character：`ref-shape` 的作法與「沒改核心」的證明

> 對應契約：`docs/aip/architecture-boundaries.md` §4（小樞脫離核心／strangler）、
> `docs/character-protocol/README.md` §2.1（host 注入白名單）、§2.2（migrator registry）、§12（reference adapters）。
> 本文說明 v0.6.0 之後「多一個角色」要碰哪些檔案，以及為什麼協定核心一行都不用動。

## 1. 為什麼要有第二個 Reference Character

v0.5.1 之前，「有哪些角色」這件事散在協定核心裡：`interaction-character` 直接寫著
`BUILTIN_ENTRYPOINTS = ["shu-rig", "sprite", "text"]`、小樞的三個配色、小樞的能力集與
`character-rig` 2.0 遷移；桌面前端則靠 `entrypoint === "shu-rig"` 之類的三向分岔決定要建哪個
adapter、用哪個 CSS class、要不要送 `palette`。那樣的架構下，「加一個角色」＝改協定核心＋改
CompanionApp，第三方永遠只能改我們的程式碼。

`ref-shape`（一個幾何圓形）存在的唯一理由，就是把這件事**證明**掉：它跟小樞沒有任何共用程式，
加它時協定核心、Runtime 的 `character.rs`、桌面的 `CompanionApp.tsx` 都沒有為它加過一行。

## 2. 加一個角色要碰哪些檔案

| 檔案 | 內容 |
|---|---|
| `apps/interaction-desktop/public/characters/<id>/manifest.json` | 角色的 CPP manifest（能力、intent、輸入、安全需求） |
| `apps/interaction-desktop/public/characters/index.json` | 多一列（`characterId`／`manifestPath`／`origin: builtin`） |
| `apps/interaction-desktop/src/character/adapters/<name>.ts` | adapter 實作（`CharacterAdapter` 介面） |
| `apps/interaction-desktop/src/character/adapters/index.ts` | 多一次 `registerBuiltinAdapter(id, factory, meta)` |
| `apps/interaction-desktop/src/character/adapterRegistry.ts` | 在 `BUILTIN_ADAPTER_IDS` 多一個宣告過的 id |
| `crates/interaction-runtime/src/character.rs` | 在 `CHARACTER_BUILTIN_ENTRYPOINTS` 多一個 id（host 白名單） |
| 測試 | adapter contract、角色專屬測試、bundled manifest 數量 |

`ref-shape` 實際動到的檔案清單見 §5。

## 3. 三個擴充點

### 3.1 Host 白名單（誰算 builtin）

協定核心不認識任何具名 adapter：`ValidationLimits::default().builtin_whitelist` 是**空的**。
host 必須注入：

- **Rust**：`interaction_runtime::character::character_host_registry().validation_limits()`
  （`CHARACTER_BUILTIN_ENTRYPOINTS` = `["shu-rig", "shape", "sprite", "text"]`；`shu-rig` 的字串來自
  `interaction_character_shu::ShuRigPack::ENTRYPOINT_ID`）。Tauri 的匯入路徑
  （`character_store.rs`）用同一個 registry，兩邊不可能漂移。
- **TS**：`character/adapterRegistry.ts` 的 `BUILTIN_ADAPTER_IDS`／`builtinEntrypointIds()`；
  `manifest.ts` 的預設白名單就是它。白名單**不**取決於 adapter 模組有沒有載入（否則 manifest
  驗證會因為匯入順序給出不同答案）；「宣告了但沒註冊工廠」由 contract test 擋。

### 3.2 Adapter meta（角色專屬的呈現細節）

host 不看 entrypoint 字串，只讀 `registerBuiltinAdapter` 註冊的 meta：

| meta 欄位 | 用途 |
|---|---|
| `cssClass` | 角色畫布的 CSS class（`companion-stage`／`companion-canvas`／`companion-text`） |
| `surface` | 掛在 `canvas` 還是 DOM 宿主（決定 canvas／text host 哪個顯示） |
| `hasPlayfield` | 有沒有遊玩場（有的話工廠回傳 `CompanionSurface`：角色表、玩具目錄、舞台、roll call） |
| `variants` | adapter 認得的 variant id（未知 variant 只原樣透傳，不猜） |
| `variantAliasKeys` | 選定 variant 時要一起送的別名鍵（rig 用 `palette`） |
| `requiresLegacyPackShape` | 需要 `x-legacy` 舊 pack 版型才建得出來（sprite） |
| `legacyPackKinds` | 這個 adapter 接手哪些舊 pack `kind`（host 用它把舊 pack 導到對的 adapter） |

`ref-shape` 只用到 `cssClass`／`surface`／`hasPlayfield`，其餘留空。

### 3.3 Migrator registry（舊 pack → manifest）

`interaction_character::MigrationRegistry` 依 (`kind`, `schemaVersion`) 分派，有界且不允許重複註冊。
核心只內建通用 sprite；小樞的 `character-rig` 2.0 由 `interaction-character-shu` 提供、由 host 註冊。
新角色如果沒有舊格式要遷移（`ref-shape` 就沒有），完全不必碰這裡。

## 4. `ref-shape` 是什麼

- `characterId` `ref-shape`、`entrypoint` `builtin:shape`、`adapterKind` `in-process`、**沒有資產**。
- 宣告 `visual.presence`、`visual.expression`（10 個變體）與 `input.click`。
- 顏色隨 intent 家族（安靜／注意／玩耍／工作）；`play` 縮放脈衝一次、`notice` 輕微位移、其餘靜止；
  Reduced Motion 時完全不動、只變色。
- 不支援 audio、gaze、particles、遊玩。
- **安全 intent 不宣告**：依 CPP §3.4 的解析規則，`wait`／`ask`／`request-consent`／`blocked`／
  `unknown`／`claim-completed`／`verified-success`／`failed`／`cancelled`／`offline`／`emergency`
  一律落到可信 `system.text`，由 host 的可信元素顯示，adapter 沒有否決權，也不會假裝演過。
- 沒有 timer、沒有 rAF：`durationHint` 由 Gateway 的 `tick(now)` 推進，dispose 後不留任何 handle。

### 實作註記：`wait` 出現在 `visual.expression.variants` 裡

CPP 的 `wait` 有 priority floor 60，屬於**安全** intent（`is_safety()`）。`ref-shape` 的 manifest 依照
本輪契約把 `wait` 列進 `visual.expression.variants`（變體名稱清單），但**沒有**把它列進 `intents`，
所以協商時它仍然走 §3.4 步驟 5 落到 `system.text`。變體名存在只是宣告「這個角色畫得出等待的樣子」，
不代表它可以代替安全文字——安全語意的權威永遠在 Runtime／host。

## 5. 「加它沒有改核心」的證明

`ref-shape` 專屬的檔案（新增或只為它多一列）：

1. `apps/interaction-desktop/public/characters/ref-shape/manifest.json`（新增）
2. `apps/interaction-desktop/public/characters/index.json`（多一列）
3. `apps/interaction-desktop/src/character/adapters/shape.ts`（新增）
4. `apps/interaction-desktop/src/character/adapters/index.ts`（多一次 `registerBuiltinAdapter("shape", …)`）
5. `apps/interaction-desktop/src/character/adapterRegistry.ts`（`BUILTIN_ADAPTER_IDS` 多一個 `"shape"`）
6. `crates/interaction-runtime/src/character.rs`（`CHARACTER_BUILTIN_ENTRYPOINTS` 多一個 `"shape"`）
7. `apps/interaction-desktop/src/test/character-ref-shape.test.ts`（新增）
8. `docs/aip/reference-character.md`（本文）、`docs/character-protocol/README.md` §12 多一列

這份清單以外：

- `crates/interaction-character/src/**` 沒有任何一行是為 `ref-shape` 加的。驗收：
  `rg -n -i 'shu|maid' crates/interaction-character/src` = **0 命中**；`ref-shape` 這個字串在核心
  `src/` 完全不存在（核心 conformance **測試**檔 `tests/conformance.rs` 裡的 `CORE_BUILTIN_IDS`
  含 `"shape"`，那是測試 host 注入什麼，不是核心程式碼）。
- `crates/interaction-runtime/src/character.rs` 只有白名單那一個字串；沒有任何 `if entrypoint == …`。
- `apps/interaction-desktop/src/companion/CompanionApp.tsx` 沒有為 `ref-shape` 加過任何分岔：
  它只呼叫 `createBuiltinAdapter(entrypoint, ctx)` 並讀 meta。
  `src/test/architecture-no-entrypoint-switch.test.ts` 讀原始碼把這件事釘死
  （`companion/*.ts(x)`、`character/gateway.ts`、`character/negotiate.ts` 內不得出現
  `=== "shu-rig"` 之類的字面分岔；`CharacterSource.kind` 是來源判別標籤，不算）。

其餘 strangler 造成的改動（把小樞搬出核心、registry 化）是 refactor，不算在這份清單裡。

## 6. 測試

| 測試 | 覆蓋 |
|---|---|
| `src/test/adapter-contract.test.ts` | shu-rig／sprite／text／shape 四個 adapter 跑同一套 contract：lifecycle、capability 註冊、unsupported 不回 completed、cancel 冪等、timeout 由 tick 推進、dispose 後不再回執／不再送輸入、重複訂閱不重複送、dispose 後 timer／rAF／DOM listener 歸零 |
| `src/test/character-ref-shape.test.ts` | manifest 與 adapter 定義一致、20 個 CPP intent 逐一送（非安全演出／安全落 system.text）、動作與顏色、Reduced Motion、input.click、切換 shu ↔ ref-shape、索引列 |
| `src/test/architecture-no-entrypoint-switch.test.ts` | host 端沒有 entrypoint 字面分岔；CompanionApp 不直接 `new` 任何角色類別 |
| `crates/interaction-character/tests/migration_registry.rs` | 空白名單預設、host 注入、migrator registry 分派／重複／有界 |
| `crates/interaction-character-shu/tests/{rig_pack,conformance}.rs` | 小樞的 variants／能力集／`character-rig` 2.0 遷移＋內建 rig 角色的 CPP conformance |
| `crates/interaction-runtime/tests/character_host_registry.rs` | Runtime 注入的白名單與 migrator registry |
| `apps/interaction-desktop/src-tauri/src/character_store.rs` 單元 | 內建角色 id 由 `index.json` 解析、壞索引降級、匯入撞名擋 `ref-shape` |
