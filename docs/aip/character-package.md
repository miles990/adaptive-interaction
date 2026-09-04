# Character Package／Manifest：版本化欄位、最小範例、匯入安全、Migration

> 證據等級用字同系列慣例：只有附測試名的敘述才算「有測試」；沒有對應測試的規則只是「契約文字」或
> 「程式碼存在但未見專屬測試」，不得寫成「已驗證」。本文件不重複定義 Character Presentation
> Protocol（CPP）——權威規格是 `docs/character-protocol/README.md`，這裡只做「版本化欄位對照」
> 「最小範例」「匯入安全」「migration」四件事的操作導覽，供要寫或審 Character Package 的人用。

## 1. Manifest 版本化欄位對照

`CharacterManifest`（`crates/interaction-character` 權威、`apps/interaction-desktop/src/character/`
鏡射，JSON Schema golden：`schemas/character-protocol.schema.json`）的版本化欄位：

| 欄位 | 版本化方式 | 規則 |
|---|---|---|
| `schemaVersion` | `major.minor` 字串 | major 必須等於 1；minor 大於實作者時允許並記錄 `newerMinor:true`（未知欄位保留、不崩潰）——`docs/character-protocol/README.md` §2.1 |
| `characterId` | 不隨版本變（穩定身分） | `^[a-z0-9][a-z0-9._-]{0,63}$`；**不等於**顯示名稱 |
| `version` | 角色本身的 semver 字串 | 資訊性，供 UI 顯示與相容判斷；協定不解析語意 |
| `compatibility.protocol` | 對 CPP 版本的宣稱（例如 `"1.x"`） | 供 UI／host 顯示；協定實際相容判斷仍由 `protocolVersion` major 握手決定 |
| `compatibility.runtime` | 對 Runtime 版本的宣稱（例如 `">=0.5.0"`） | 資訊性；host 不強制執行語意化版本比對 |
| `assets[].sha256` | 每個資產的完整性雜湊 | 匯入與讀取時都重新核對（§3） |
| `securityRequirements` | 不隨版本變，但決定 UI 標示 | `{network,executable,fileAccess,audioOutput,microphone,camera}`；`executable:true`／`network:true`／`adapterKind≠in-process` 必須在 UI 標「第三方／外部／需要網路／有可執行程式」 |
| `resourceLimits` | 上限值，host 可再收斂但不能放寬超過協定天花板 | `maxAssetBytes`（協定天花板 32 MB，`MAX_ASSET_BYTES_CEILING`，`crates/interaction-character/src/manifest.rs:18`）、`maxConcurrentCommands`、`maxQueue`、`maxFps` |
| `fallbacks` | 每個角色自訂，但安全 intent 只能 fallback 到安全 intent（§3.4 步驟 2 的限制） | `capabilities`／`intents` 兩張映射表 |

以上除 `schemaVersion` 外都不是 AIP／CPP 協定本身的版本號——**協定版本只有 `protocolVersion`
（CPP 握手，`docs/character-protocol/README.md` §3.3）與 `specVersion`（AIP envelope，`aip/1.0`）**，
manifest 裡的欄位都是「這個角色宣稱什麼」，不是「協定本身是第幾版」。

## 2. `ref-shape` 最小範例（形狀見 `docs/aip/reference-character.md` §4）

```jsonc
{
  "schemaVersion": "1.0",
  "characterId": "ref-shape",
  "displayName": { "en": "Reference Shape" },
  "version": "1.0.0",
  "adapterKind": "in-process",
  "entrypoint": { "kind": "builtin", "id": "shape" },
  "assets": [],
  "capabilities": {
    "visual.presence": { "supported": true },
    "visual.expression": {
      "supported": true,
      "variants": ["idle","notice","play","rest","work","think","acknowledge","wait","greet","sleep"]
    }
  },
  "inputCapabilities": { "input.click": { "supported": true } },
  "intents": ["idle","notice","play","rest","work","think","acknowledge","greet","sleep"],
  "securityRequirements": { "network": false, "executable": false, "fileAccess": "none",
                             "audioOutput": false, "microphone": false, "camera": false },
  "resourceLimits": { "maxAssetBytes": 0, "maxConcurrentCommands": 4, "maxQueue": 32, "maxFps": 60 },
  "fallbacks": {},
  "compatibility": { "protocol": "1.x", "runtime": ">=0.6.0" }
}
```

這是**形狀範例**，不是逐位元組複製 repo 內實際檔案的內容；實際檔案在
`apps/interaction-desktop/public/characters/ref-shape/manifest.json`
（本文件任務範圍不含讀取／改動該路徑下的檔案，因為它屬於目前有其他 agent 並行修改的
`apps/interaction-desktop`）。**注意** `wait` 出現在 `visual.expression.variants`（宣稱畫得出等待的
樣子）但**沒有**列進 `intents`——依 `docs/aip/reference-character.md` §4 的實作註記，這是刻意的：
安全 intent 的權威落地方式永遠是 `system.text`，manifest 宣告變體名不代表可以取代它。

## 3. 匯入安全

實作：`apps/interaction-desktop/src-tauri/src/character_store.rs`（Tauri 匯入路徑）；純驗證邏輯
（`validate_import`，:212）與檔案系統操作（`import`，:376）分開，前者可離線單測。

| 防線 | 位置 | 規則 |
|---|---|---|
| 路徑片段白名單 | `is_safe_segment`（:151） | 非空、≤64 字、不以 `.` 開頭、只允許英數字與 `.`／`_`／`-` |
| 路徑解析不逃逸 | `resolve_inside`（:162-180） | 逐段檢查 `is_safe_segment`＋`check_relative_path`＋只能是 `Component::Normal`，最終結果必須 `starts_with(base)` |
| Symlink 逃逸 | `import`（:376-424，符號連結防線段） | 寫入暫存資料夾後 `canonicalize()` 兩邊路徑，`tmp_real.starts_with(&root_real)` 才視為合法；不符則整個匯入失敗並清掉暫存資料夾。**本次核實沒有找到以真實 symlink 建構的專屬測試**（測試清單裡沒有 `symlink` 字樣命中），這條防線目前只有程式碼存在，未見專屬回歸測試——誠實標記「未直接驗證」 |
| Magic bytes | `asset_magic_matches`（`crates/interaction-character` 匯出，`character_store.rs` 呼叫端在 :315 與 :545 兩處：匯入時與之後每次讀取時都重新核對） | MIME／副檔名不可作唯一信任依據；核對 png/jpg/gif/webp/svg/json/mp3/wav/ogg/webm 的實際位元組 |
| 大小上限 | `MAX_TOTAL_IMPORT_BYTES`＝32 MB（:34）、`MAX_ASSET_DATA_URL_BYTES`＝8 MB（:36）、`effective_max_asset_bytes`（:204，取 manifest 宣告與 `MAX_ASSET_BYTES_CEILING` 的較小值） | 單一匯入總量與單次讀取（`asset_data_url`）分開限制 |
| sha256 完整性 | `sha256_hex`（:199）＋ manifest `assets[].sha256` | 匯入與讀取都核對，見 §1 |
| 內建角色名稱碰撞 | `is_bundled`（:134）＋ `import` 對 `bundled_character_ids()` 的檢查 | 不能匯入一個跟內建角色同 `characterId` 的第三方角色（含 `ref-shape`） |

已知測試（`character_store.rs` 內嵌 `#[cfg(test)]`）：`imports_a_valid_sprite_character_and_lists_it`
(:620)、`rejects_traversal_in_asset_ids_and_paths`(:662)、`resolve_inside_never_escapes`(:688)、
`rejects_oversize_assets_per_manifest_limit_and_total`(:706)、
`rejects_spoofed_magic_bytes_and_wrong_hashes`(:729)、
`rejects_external_kinds_non_whitelisted_builtins_and_bundled_ids`(:769)、
`declared_and_provided_assets_must_match_exactly`(:801)、
`asset_data_url_rechecks_path_size_and_magic`(:820)、`remove_only_touches_imported_characters`(:847)、
`bundled_character_index_parses_and_matches_the_frontend_index`(:868)、
`a_broken_or_oversized_index_degrades_to_a_bounded_list`(:893)、
`importing_a_bundled_reference_character_id_is_refused`(:913，含 `ref-shape` 碰撞情境)、
`list_reports_corrupt_folders_honestly`(:921)。證據等級：unit（Rust，本次未重新執行，只核對測試
函式與其對應邏輯是否存在）。

CPP 核心層另有一套獨立於 Tauri 匯入路徑的 manifest 驗證（`crates/interaction-character/tests/manifest.rs`，
`docs/character-protocol/README.md` §13 記載 18 個測試，涵蓋惡意 manifest／路徑穿越），本文件未重新
逐一核對每個測試函式名，引用既有文件記載的數字，標記為「沿用既有文件」而非本次獨立驗證。

## 4. Migration（`MigrationRegistry`）

見 `crates/interaction-character/src/manifest.rs`：`PackMigrator` trait（:1580）、
`MigrationRegistry`（struct :1609，`register`：:1638）。規則（`docs/character-protocol/README.md` §2.2）：

- 依 `(kind, schemaVersion)` 分派；有界（≤32 個 migrator、每個 ≤8 個 schemaVersion）；同一組
  `(kind, version)` 不得註冊兩次。
- 核心只內建通用 sprite（`character-pack` 1.0／1.1）；`character-rig` 2.0 由
  `interaction-character-shu::RigPackMigrator` 提供（`crates/interaction-character-shu/src/lib.rs`，
  `impl PackMigrator for RigPackMigrator`，`kind()`→`"character-rig"`、
  `schema_versions()`→`["2.0"]`、`migrate()`→呼叫 `ShuRigPack::migrate`）。
- 入口 `migrate_pack_to_manifest(json, &registry)`；舊的 `migrate_legacy_pack`
  （`crates/interaction-character/src/manifest.rs:1719`）已標 `#[deprecated(since="0.6.0", note="use
  migrate_pack_to_manifest with a host MigrationRegistry; this path only migrates sprite packs")]`，
  內部改呼叫 `migrate_pack_to_manifest(json, &MigrationRegistry::with_core_migrators())`（只剩 sprite
  一條路，因為核心不能依賴任何角色 crate）。
- 沒有 migrator 的格式一律 `ManifestErrorCode::Legacy`，不猜、不執行。

TS 鏡射（`apps/interaction-desktop/src/character/manifest.ts`）同一套：`PackMigrator`（:946）、
`MigrationRegistry`（:970）、`coreMigrationRegistry()`（:1012，只有核心 sprite）、
`setDefaultMigrationRegistry`／`defaultMigrationRegistry`（:1022,1027）、
`migratePackToManifest(json, opts)`（:1149，`opts.registry ?? defaultMigrationRegistry()`）。
`character-rig` 2.0 的 `rigPackMigrator` 住在 `character/adapters/shu.ts`，由
`character/adapterRegistry.ts::registerHostMigrator`／`hostMigrationRegistry()`
（`adapterRegistry.ts`:232-253）組成桌面 host 的完整 registry。

## 5. Integrity 與 minimum runtime version

- **完整性**：`assets[].sha256` 是唯一的完整性欄位；沒有整份 manifest 或整個 package 的簽章機制
  （§2.1「未簽章、`securityRequirements.executable=true`／`network=true`／`adapterKind≠in-process`
  的 manifest 在 UI 必須標示」——這是揭露義務，不是簽章驗證）。
- **Minimum runtime version**：`compatibility.runtime`（例如 `">=0.5.0"`）是**資訊性欄位**，本次核實
  沒有找到 Rust 或 TS 端對這個字串做語意化版本比較並據以拒絕載入的程式碼（`rg` 只找到它作為
  manifest 結構的一個欄位被讀寫，沒有找到比較邏輯）。誠實記錄：**目前只是宣稱，不是強制**——
  host 不會因為 `compatibility.runtime` 宣稱的版本高於自己就拒絕載入這個角色。真正的相容把關是
  `schemaVersion` major 檢查（§1）與 `protocolVersion` major 握手（CPP §3.3），不是這個欄位。
