//! 匯入角色（CPP manifest＋資產）的本機儲存：`<home>/state/characters/<characterId>/`。
//!
//! 佈局：
//! ```text
//! <home>/state/characters/<characterId>/manifest.json   （原文，已通過驗證）
//! <home>/state/characters/<characterId>/assets/<assetId>
//! ```
//!
//! 安全邊界（CPP README §2.1／§9）：
//! - 驗證交給 `interaction-character`（`parse_manifest` = 大小、JSON、§2.1）；這裡只加 host 規則。
//! - 只接受 `adapterKind: in-process` 且 builtin 白名單 entrypoint；外部 adapter
//!   （external-process／remote-device／web）一律走 `/v1/character/adapters`，匯入不執行、不連線。
//! - 資產以 magic bytes 核對宣告的 mediaType（MIME／副檔名不可作唯一信任依據）、
//!   單檔 ≤ `resourceLimits.maxAssetBytes`（≤ 32 MB 上限）、總量 ≤ 32 MB；
//!   manifest 若宣告 `bytes`／`sha256` 必須相符。
//! - 所有路徑都由 characterId／assetId 的白名單字元組成，寫入前再以
//!   `check_relative_path` 與 [`resolve_inside`] 確認落在角色資料夾內；永不寫到資料夾外。
//! - 內建角色（`public/characters/index.json`）不住在這裡，因此不可被覆蓋或移除。
//! - 錯誤訊息只含 id／mediaType，不回顯絕對路徑或長輸入。

use base64::Engine;
use interaction_character::{
    asset_magic_matches, check_relative_path, is_valid_character_id, parse_manifest, AdapterKind,
    AssetDecl, CharacterManifest, Entrypoint, LocalizedText, ManifestReport, ValidationLimits,
    MAX_ASSETS, MAX_ASSET_BYTES_CEILING,
};
use interaction_runtime::character::character_host_registry;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

/// 一次匯入的資產總量上限。
pub const MAX_TOTAL_IMPORT_BYTES: u64 = 32 * 1024 * 1024;
/// `character_asset` 回傳 data URL 的單檔上限（WebView 記憶體考量）。
pub const MAX_ASSET_DATA_URL_BYTES: u64 = 8 * 1024 * 1024;
/// 一次匯入的資產筆數上限（＝ CPP §2.1 的 `MAX_ASSETS`）。IPC 介面在解碼之前就用它
/// 擋掉超量呼叫，讓有界性由 host 而不是呼叫端決定（對抗審查 character-package-021）。
pub const MAX_IMPORT_ASSETS: usize = MAX_ASSETS;
/// 內建角色索引（前端擁有的 `public/characters/index.json`）。編譯期 include：
/// 索引改了就要重編，不會靜默漂移；執行期只解析一次。
const BUNDLED_CHARACTER_INDEX: &str = include_str!("../../public/characters/index.json");
/// 內建角色數量上限（有界集合；索引若異常膨脹一律截斷並在日誌留下記錄）。
pub const MAX_BUNDLED_CHARACTERS: usize = 64;

/// 內建角色 id（由 [`BUNDLED_CHARACTER_INDEX`] 解析而來）。匯入不得撞名、移除不得碰。
///
/// 索引壞掉時回空清單：那代表「host 目前不知道任何內建角色」，匯入撞名保護會失效，
/// 所以解析失敗不會被吞掉——`bundled_character_index_parses` 測試會擋在 CI。
pub fn bundled_character_ids() -> &'static [String] {
    static IDS: OnceLock<Vec<String>> = OnceLock::new();
    IDS.get_or_init(|| parse_bundled_ids(BUNDLED_CHARACTER_INDEX))
}

fn parse_bundled_ids(text: &str) -> Vec<String> {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(text) else {
        return Vec::new();
    };
    let Some(items) = json.get("characters").and_then(|c| c.as_array()) else {
        return Vec::new();
    };
    let mut out: Vec<String> = Vec::new();
    for item in items.iter().take(MAX_BUNDLED_CHARACTERS) {
        let Some(id) = item.get("characterId").and_then(|v| v.as_str()) else {
            continue;
        };
        if !is_valid_character_id(id) || out.iter().any(|seen| seen == id) {
            continue;
        }
        out.push(id.to_string());
    }
    out
}
const MANIFEST_FILE: &str = "manifest.json";
const ASSETS_DIR: &str = "assets";

/// 已 base64 解碼的匯入資產。
#[derive(Debug, Clone)]
pub struct ImportAssetInput {
    pub id: String,
    pub bytes: Vec<u8>,
}

/// 匯入結果（回給 UI）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportOutcome {
    pub character_id: String,
    pub display_name: LocalizedText,
    pub report: ManifestReport,
    pub assets: Vec<String>,
}

/// 已匯入角色清單項目。`valid: false` 代表資料夾存在但 manifest 讀不到／驗證失敗
/// （誠實列出，讓使用者可以移除，而不是默默消失）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedCharacter {
    pub character_id: String,
    pub valid: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub display_name: LocalizedText,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adapter_kind: Option<AdapterKind>,
    /// builtin entrypoint id（host adapter registry 的白名單之一）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entrypoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// UI 標示旗標（§2.1：第三方／外部／需要網路／有可執行程式）。
    pub executable: bool,
    pub network: bool,
    pub external: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report: Option<ManifestReport>,
    /// 已通過驗證的完整 manifest（角色視窗用它建 adapter；`valid: false` 時為 None）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manifest: Option<CharacterManifest>,
    pub assets: Vec<String>,
    pub origin: &'static str,
}

/// 驗證通過、尚未落地的匯入。
#[derive(Debug)]
pub struct ValidatedImport {
    pub manifest: CharacterManifest,
    pub report: ManifestReport,
    /// (宣告, 對應輸入索引)，依 manifest 宣告順序。
    pub assets: Vec<(AssetDecl, usize)>,
}

pub fn characters_root(home: &Path) -> PathBuf {
    home.join("state").join("characters")
}

pub fn is_bundled(character_id: &str) -> bool {
    bundled_character_ids().iter().any(|id| id == character_id)
}

/// host 注入給 CPP 驗證的限制（builtin 白名單來自桌面 host 的 adapter registry；
/// 核心自己沒有預設值）。
fn host_limits() -> ValidationLimits {
    character_host_registry().validation_limits()
}

/// host 白名單（錯誤訊息與 entrypoint 檢查共用）。
fn host_builtin_ids() -> Vec<String> {
    host_limits().builtin_whitelist
}

/// 單一路徑片段白名單：非空、≤ 64 字、不以 `.` 開頭、只含英數與 `._-`。
/// 這比 `check_relative_path` 更嚴：片段裡連 `/` 都不允許。
pub fn is_safe_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment.len() <= 64
        && !segment.starts_with('.')
        && segment
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// 把 `segments` 接到 `base` 底下，並保證結果仍在 `base` 內。每個片段都要通過
/// [`is_safe_segment`]、`check_relative_path` 與「只能是 Normal component」三道檢查。
pub fn resolve_inside(base: &Path, segments: &[&str]) -> Result<PathBuf, String> {
    let mut out = base.to_path_buf();
    for segment in segments {
        if !is_safe_segment(segment) {
            return Err("path segment contains characters that are not allowed".into());
        }
        check_relative_path(segment).map_err(|reason| reason.to_string())?;
        if Path::new(segment)
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err("path segment must be a plain file name".into());
        }
        out.push(segment);
    }
    if !out.starts_with(base) {
        return Err("resolved path escapes the character folder".into());
    }
    Ok(out)
}

/// 解碼 IPC 送來的 base64 資產；先以字串長度擋掉超過總量上限的輸入，再解碼。
/// 錯誤不回顯內容。
pub fn decode_asset_base64(text: &str) -> Result<Vec<u8>, String> {
    let trimmed = text.trim();
    let max_encoded = (MAX_TOTAL_IMPORT_BYTES / 3 + 1) * 4;
    if trimmed.len() as u64 > max_encoded {
        return Err(format!(
            "asset exceeds the {} MB import limit",
            MAX_TOTAL_IMPORT_BYTES / (1024 * 1024)
        ));
    }
    base64::engine::general_purpose::STANDARD
        .decode(trimmed)
        .map_err(|_| "asset is not valid base64".to_string())
}

/// IPC 介面的**有界**解碼：筆數上限先擋，再逐筆解碼並累計總量。
///
/// 以前 `character_import` 是「先把每一筆都解碼進 Vec，之後才在 `validate_import` 檢查
/// ≤64 筆／≤32 MB」，記憶體用量由呼叫端說了算（對抗審查 character-package-021）。
/// 有界性必須由 host 決定，而且要在解碼**之前**生效。
pub fn decode_import_assets(
    assets: impl IntoIterator<Item = (String, String)>,
) -> Result<Vec<ImportAssetInput>, String> {
    let assets: Vec<(String, String)> = assets.into_iter().collect();
    if assets.len() > MAX_IMPORT_ASSETS {
        return Err(format!(
            "too many assets: this host imports at most {MAX_IMPORT_ASSETS} per character"
        ));
    }
    let mut decoded = Vec::with_capacity(assets.len());
    let mut total: u64 = 0;
    for (id, base64) in assets {
        let bytes = decode_asset_base64(&base64)?;
        total = total.saturating_add(bytes.len() as u64);
        if total > MAX_TOTAL_IMPORT_BYTES {
            return Err(format!(
                "assets exceed the {} MB total import limit",
                MAX_TOTAL_IMPORT_BYTES / (1024 * 1024)
            ));
        }
        decoded.push(ImportAssetInput { id, bytes });
    }
    Ok(decoded)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// 單一資產的實際上限＝min(manifest 宣告, host 天花板, **讀得回來的上限**)。
///
/// 讀回資產的 `asset_data_url()` 硬性拒絕超過 [`MAX_ASSET_DATA_URL_BYTES`]，所以匯入時
/// 若只擋到 32 MB，8–32 MB 的資產會「匯入成功」卻永遠載不起來，使用者只看得到固定的
/// 「角色載入失敗，改用文字顯示」——accepted 被當成可用（對抗審查 character-package-019）。
/// 兩個上限在這裡交叉檢查，匯入當下就誠實拒絕。
fn effective_max_asset_bytes(manifest: &CharacterManifest) -> u64 {
    manifest
        .resource_limits
        .max_asset_bytes
        .min(MAX_ASSET_BYTES_CEILING)
        .min(MAX_ASSET_DATA_URL_BYTES)
}

/// 純驗證（不碰檔案系統）：manifest 規則＋host 規則＋每個資產的大小／magic／sha256。
pub fn validate_import(
    manifest_text: &str,
    assets: &[ImportAssetInput],
) -> Result<ValidatedImport, String> {
    let (manifest, report) = parse_manifest(manifest_text.as_bytes(), &host_limits())
        .map_err(|e| format!("manifest invalid: {e}"))?;
    let character_id = manifest.character_id.clone();
    if !is_valid_character_id(&character_id) || !is_safe_segment(&character_id) {
        return Err("characterId is not a safe identifier".into());
    }
    if is_bundled(&character_id) {
        return Err(format!(
            "characterId '{character_id}' is a bundled character and cannot be replaced"
        ));
    }
    if manifest.adapter_kind != AdapterKind::InProcess {
        return Err(
            "only in-process characters can be imported; external adapters must be registered \
             through /v1/character/adapters"
                .into(),
        );
    }
    let builtin_ids = host_builtin_ids();
    match &manifest.entrypoint {
        Entrypoint::Builtin { id } if builtin_ids.iter().any(|w| w == id) => {}
        Entrypoint::Builtin { .. } => {
            return Err(format!(
                "entrypoint must be a whitelisted builtin ({})",
                builtin_ids.join("/")
            ))
        }
        _ => {
            return Err(
                "in-process characters must use a builtin entrypoint; module/process/url \
                 entrypoints are never executed by the host"
                    .into(),
            )
        }
    }

    // 宣告端：id 安全、路徑合法、id 不重複（大小寫不敏感：macOS 檔案系統會撞）。
    let mut seen: Vec<String> = Vec::new();
    for decl in &manifest.assets {
        if !is_safe_segment(&decl.id) {
            return Err(format!(
                "asset id '{}' is not a safe identifier",
                short(&decl.id)
            ));
        }
        check_relative_path(&decl.path)
            .map_err(|reason| format!("asset '{}' path rejected: {reason}", decl.id))?;
        let lower = decl.id.to_ascii_lowercase();
        if seen.contains(&lower) {
            return Err(format!("asset id '{}' is declared twice", decl.id));
        }
        seen.push(lower);
    }
    // 輸入端：id 安全、必須是宣告過的、不重複。
    let mut seen_inputs: Vec<String> = Vec::new();
    for input in assets {
        if !is_safe_segment(&input.id) {
            return Err(format!(
                "asset id '{}' is not a safe identifier",
                short(&input.id)
            ));
        }
        if !manifest.assets.iter().any(|d| d.id == input.id) {
            return Err(format!(
                "asset '{}' is not declared in the manifest",
                input.id
            ));
        }
        if seen_inputs.contains(&input.id) {
            return Err(format!("asset '{}' was provided twice", input.id));
        }
        seen_inputs.push(input.id.clone());
    }

    let max_asset = effective_max_asset_bytes(&manifest);
    let mut total: u64 = 0;
    let mut resolved = Vec::with_capacity(manifest.assets.len());
    for decl in &manifest.assets {
        let Some(index) = assets.iter().position(|a| a.id == decl.id) else {
            return Err(format!("declared asset '{}' was not provided", decl.id));
        };
        let bytes = &assets[index].bytes;
        let len = bytes.len() as u64;
        if len == 0 {
            return Err(format!("asset '{}' is empty", decl.id));
        }
        if len > max_asset {
            // 上限可能來自 manifest 也可能來自 host（讀回上限）；訊息只講數字，不回顯輸入。
            return Err(format!(
                "asset '{}' is {len} bytes; this host imports at most {max_asset} bytes per asset \
                 (it must also stay under the {} MB read-back limit)",
                decl.id,
                MAX_ASSET_DATA_URL_BYTES / (1024 * 1024)
            ));
        }
        total = total.saturating_add(len);
        if total > MAX_TOTAL_IMPORT_BYTES {
            return Err(format!(
                "assets exceed the {} MB total import limit",
                MAX_TOTAL_IMPORT_BYTES / (1024 * 1024)
            ));
        }
        if !asset_magic_matches(&decl.media_type, bytes) {
            return Err(format!(
                "asset '{}' content does not match its declared mediaType {}",
                decl.id, decl.media_type
            ));
        }
        if let Some(declared) = decl.bytes {
            if declared != len {
                return Err(format!(
                    "asset '{}' is {len} bytes but the manifest declares {declared}",
                    decl.id
                ));
            }
        }
        if let Some(expected) = &decl.sha256 {
            let actual = sha256_hex(bytes);
            if !actual.eq_ignore_ascii_case(expected) {
                return Err(format!(
                    "asset '{}' sha256 does not match the manifest",
                    decl.id
                ));
            }
        }
        resolved.push((decl.clone(), index));
    }
    Ok(ValidatedImport {
        manifest,
        report,
        assets: resolved,
    })
}

fn short(id: &str) -> String {
    id.chars().take(64).collect()
}

fn write_tree(
    tmp: &Path,
    manifest_text: &str,
    validated: &ValidatedImport,
    assets: &[ImportAssetInput],
) -> Result<(), String> {
    let assets_dir = resolve_inside(tmp, &[ASSETS_DIR])?;
    std::fs::create_dir_all(&assets_dir).map_err(|e| format!("create character folder: {e}"))?;
    let manifest_path = resolve_inside(tmp, &[MANIFEST_FILE])?;
    std::fs::write(&manifest_path, manifest_text.as_bytes())
        .map_err(|e| format!("write manifest: {e}"))?;
    for (decl, index) in &validated.assets {
        // 寫入前再檢查一次：id 必須是安全片段、落在 assets/ 之內。
        let target = resolve_inside(tmp, &[ASSETS_DIR, decl.id.as_str()])?;
        let Some(input) = assets.get(*index) else {
            return Err(format!("asset '{}' vanished during import", decl.id));
        };
        std::fs::write(&target, &input.bytes)
            .map_err(|e| format!("write asset '{}': {e}", decl.id))?;
    }
    Ok(())
}

/// 驗證並落地。先寫到 `.tmp-<id>-<pid>`，全部成功後才換入正式資料夾；
/// 任何錯誤都清掉暫存，不留半套角色。
pub fn import(
    root: &Path,
    manifest_text: &str,
    assets: &[ImportAssetInput],
) -> Result<ImportOutcome, String> {
    let validated = validate_import(manifest_text, assets)?;
    let character_id = validated.manifest.character_id.clone();
    std::fs::create_dir_all(root).map_err(|e| format!("create characters root: {e}"))?;
    let final_dir = resolve_inside(root, &[character_id.as_str()])?;
    let tmp = root.join(format!(".tmp-{character_id}-{}", std::process::id()));
    if tmp.exists() {
        let _ = std::fs::remove_dir_all(&tmp);
    }
    let staged = write_tree(&tmp, manifest_text, &validated, assets).and_then(|_| {
        // 符號連結防線：暫存資料夾的真實路徑必須仍在 root 之下。
        let root_real = root
            .canonicalize()
            .map_err(|e| format!("resolve characters root: {e}"))?;
        let tmp_real = tmp
            .canonicalize()
            .map_err(|e| format!("resolve staging folder: {e}"))?;
        if !tmp_real.starts_with(&root_real) {
            return Err("staging folder escaped the characters root".into());
        }
        Ok(())
    });
    if let Err(e) = staged {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(e);
    }
    if final_dir.exists() {
        if let Err(e) = std::fs::remove_dir_all(&final_dir) {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(format!("replace existing character: {e}"));
        }
    }
    if let Err(e) = std::fs::rename(&tmp, &final_dir) {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(format!("move character into place: {e}"));
    }
    Ok(ImportOutcome {
        character_id,
        display_name: validated.manifest.display_name.clone(),
        report: validated.report,
        assets: validated
            .assets
            .iter()
            .map(|(decl, _)| decl.id.clone())
            .collect(),
    })
}

fn read_manifest(dir: &Path) -> Result<(CharacterManifest, ManifestReport), String> {
    let path = resolve_inside(dir, &[MANIFEST_FILE])?;
    let bytes = std::fs::read(&path).map_err(|e| format!("read manifest: {e}"))?;
    parse_manifest(&bytes, &host_limits()).map_err(|e| format!("manifest invalid: {e}"))
}

fn list_asset_files(dir: &Path) -> Vec<String> {
    let Ok(assets_dir) = resolve_inside(dir, &[ASSETS_DIR]) else {
        return Vec::new();
    };
    let mut ids: Vec<String> = std::fs::read_dir(assets_dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter(|e| e.path().is_file())
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|name| is_safe_segment(name))
                .collect()
        })
        .unwrap_or_default();
    ids.sort();
    ids
}

fn load_entry(root: &Path, character_id: &str) -> ImportedCharacter {
    let invalid = |error: String| ImportedCharacter {
        character_id: character_id.to_string(),
        valid: false,
        error: Some(error),
        display_name: LocalizedText::new(),
        adapter_kind: None,
        entrypoint: None,
        version: None,
        executable: false,
        network: false,
        external: false,
        report: None,
        manifest: None,
        assets: Vec::new(),
        origin: "imported",
    };
    let dir = match resolve_inside(root, &[character_id]) {
        Ok(d) => d,
        Err(e) => return invalid(e),
    };
    let (manifest, report) = match read_manifest(&dir) {
        Ok(v) => v,
        Err(e) => return invalid(e),
    };
    if manifest.character_id != character_id {
        return invalid("manifest characterId does not match its folder".into());
    }
    let entrypoint = match &manifest.entrypoint {
        Entrypoint::Builtin { id } => Some(id.clone()),
        _ => None,
    };
    ImportedCharacter {
        character_id: character_id.to_string(),
        valid: true,
        error: None,
        display_name: manifest.display_name.clone(),
        adapter_kind: Some(manifest.adapter_kind),
        entrypoint,
        version: Some(manifest.version.clone()),
        executable: report.executable,
        network: report.needs_network,
        external: report.external,
        report: Some(report),
        assets: list_asset_files(&dir),
        manifest: Some(manifest),
        origin: "imported",
    }
}

/// 列出已匯入角色（依 id 排序）。root 不存在＝空清單。
pub fn list(root: &Path) -> Vec<ImportedCharacter> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut ids: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|name| is_valid_character_id(name) && is_safe_segment(name))
        .collect();
    ids.sort();
    ids.iter().map(|id| load_entry(root, id)).collect()
}

/// 讀一個已匯入資產並以 data URL 回傳（≤ 8 MB；讀出後再核對一次 magic bytes）。
pub fn asset_data_url(root: &Path, character_id: &str, asset_id: &str) -> Result<String, String> {
    if !is_valid_character_id(character_id) {
        return Err("characterId is not a valid identifier".into());
    }
    let dir = resolve_inside(root, &[character_id])?;
    if !dir.is_dir() {
        return Err(format!("'{character_id}' is not an imported character"));
    }
    let (manifest, _) = read_manifest(&dir)?;
    let Some(decl) = manifest.assets.iter().find(|d| d.id == asset_id) else {
        return Err(format!(
            "asset '{}' is not declared by '{character_id}'",
            short(asset_id)
        ));
    };
    let path = resolve_inside(&dir, &[ASSETS_DIR, asset_id])?;
    let meta = std::fs::metadata(&path).map_err(|e| format!("read asset '{asset_id}': {e}"))?;
    if !meta.is_file() {
        return Err(format!("asset '{asset_id}' is not a file"));
    }
    if meta.len() > MAX_ASSET_DATA_URL_BYTES {
        return Err(format!(
            "asset '{asset_id}' exceeds the {} MB data URL limit",
            MAX_ASSET_DATA_URL_BYTES / (1024 * 1024)
        ));
    }
    let bytes = std::fs::read(&path).map_err(|e| format!("read asset '{asset_id}': {e}"))?;
    if !asset_magic_matches(&decl.media_type, &bytes) {
        return Err(format!(
            "asset '{asset_id}' on disk does not match its declared mediaType {}",
            decl.media_type
        ));
    }
    Ok(format!(
        "data:{};base64,{}",
        decl.media_type,
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

/// 移除已匯入角色。內建角色不在此資料夾，且一律拒絕。
pub fn remove(root: &Path, character_id: &str) -> Result<(), String> {
    if !is_valid_character_id(character_id) {
        return Err("characterId is not a valid identifier".into());
    }
    if is_bundled(character_id) {
        return Err(format!(
            "'{character_id}' is a bundled character and cannot be removed"
        ));
    }
    let dir = resolve_inside(root, &[character_id])?;
    if !dir.is_dir() {
        return Err(format!("'{character_id}' is not an imported character"));
    }
    std::fs::remove_dir_all(&dir).map_err(|e| format!("remove character: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    const JPEG_MAGIC: [u8; 3] = [0xFF, 0xD8, 0xFF];

    fn png(len: usize) -> Vec<u8> {
        let mut v = PNG_MAGIC.to_vec();
        v.resize(len.max(PNG_MAGIC.len()), 7);
        v
    }

    fn manifest_json(character_id: &str, extra: &str) -> String {
        format!(
            r#"{{
  "schemaVersion": "1.0",
  "characterId": "{character_id}",
  "displayName": {{ "zh-TW": "測試角色", "en": "Test" }},
  "version": "1.0.0",
  "adapterKind": "in-process",
  "entrypoint": {{ "kind": "builtin", "id": "sprite" }},
  "assets": [ {{ "id": "sheet", "path": "sheet.png", "mediaType": "image/png" }} ],
  "capabilities": {{ "visual.presence": {{ "supported": true }} }},
  "inputCapabilities": {{ "input.click": {{ "supported": true }} }},
  "channels": ["transform"],
  "intents": ["idle", "notice"],
  "locales": ["zh-TW"],
  "compatibility": {{ "protocol": "1.x" }}{extra}
}}"#
        )
    }

    fn sheet(bytes: Vec<u8>) -> Vec<ImportAssetInput> {
        vec![ImportAssetInput {
            id: "sheet".into(),
            bytes,
        }]
    }

    fn root() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    #[test]
    fn imports_a_valid_sprite_character_and_lists_it() {
        let dir = root();
        let out =
            import(dir.path(), &manifest_json("my-char", ""), &sheet(png(64))).expect("import ok");
        assert_eq!(out.character_id, "my-char");
        assert_eq!(
            out.display_name.get("zh-TW").map(String::as_str),
            Some("測試角色")
        );
        assert_eq!(out.assets, vec!["sheet".to_string()]);
        assert!(!out.report.executable && !out.report.needs_network && !out.report.external);
        assert!(dir.path().join("my-char/manifest.json").is_file());
        assert!(dir.path().join("my-char/assets/sheet").is_file());
        // 暫存資料夾不得殘留。
        assert!(!dir
            .path()
            .join(format!(".tmp-my-char-{}", std::process::id()))
            .exists());

        let listed = list(dir.path());
        assert_eq!(listed.len(), 1);
        let entry = &listed[0];
        assert!(entry.valid);
        assert_eq!(entry.character_id, "my-char");
        assert_eq!(entry.entrypoint.as_deref(), Some("sprite"));
        assert_eq!(entry.adapter_kind, Some(AdapterKind::InProcess));
        assert_eq!(entry.assets, vec!["sheet".to_string()]);
        assert_eq!(entry.origin, "imported");
        assert!(!entry.executable && !entry.network && !entry.external);

        // 重複匯入＝覆蓋，不是累加。
        import(dir.path(), &manifest_json("my-char", ""), &sheet(png(128))).expect("re-import");
        assert_eq!(list(dir.path()).len(), 1);
        assert_eq!(
            std::fs::metadata(dir.path().join("my-char/assets/sheet"))
                .expect("meta")
                .len(),
            128
        );
    }

    #[test]
    fn rejects_traversal_in_asset_ids_and_paths() {
        let dir = root();
        // 輸入端 id 帶 `..`。
        let bad = vec![ImportAssetInput {
            id: "../evil".into(),
            bytes: png(32),
        }];
        let err = import(dir.path(), &manifest_json("trav", ""), &bad).unwrap_err();
        assert!(err.contains("not a safe identifier"), "{err}");
        // 宣告端 path 帶 `..`（crate 驗證擋下）。
        let m = manifest_json("trav", "").replace("\"sheet.png\"", "\"../../sheet.png\"");
        let err = import(dir.path(), &m, &sheet(png(32))).unwrap_err();
        assert!(err.contains("manifest invalid"), "{err}");
        // 宣告端 id 帶斜線。
        let m = manifest_json("trav", "").replace("\"id\": \"sheet\"", "\"id\": \"a/b\"");
        let err = import(dir.path(), &m, &sheet(png(32))).unwrap_err();
        assert!(
            err.contains("not a safe identifier") || err.contains("manifest invalid"),
            "{err}"
        );
        assert!(list(dir.path()).is_empty());
        // 錯誤訊息不得回顯絕對路徑。
        assert!(!err.contains(dir.path().to_string_lossy().as_ref()));
    }

    #[test]
    fn resolve_inside_never_escapes() {
        let base = Path::new("/base/characters");
        assert!(resolve_inside(base, &[".."]).is_err());
        assert!(resolve_inside(base, &["a", ".."]).is_err());
        assert!(resolve_inside(base, &["../x"]).is_err());
        assert!(resolve_inside(base, &["/etc/passwd"]).is_err());
        assert!(resolve_inside(base, &["a\\b"]).is_err());
        assert!(resolve_inside(base, &[".hidden"]).is_err());
        assert!(resolve_inside(base, &[""]).is_err());
        assert!(resolve_inside(base, &["C:"]).is_err());
        let ok = resolve_inside(base, &["shu", "assets", "sheet.png"]).expect("ok");
        assert_eq!(ok, PathBuf::from("/base/characters/shu/assets/sheet.png"));
        assert!(is_safe_segment("sheet.png"));
        assert!(!is_safe_segment(&"a".repeat(65)));
        assert!(!is_safe_segment("a b"));
    }

    #[test]
    fn rejects_oversize_assets_per_manifest_limit_and_total() {
        let dir = root();
        let m = manifest_json(
            "big",
            r#",
  "resourceLimits": { "maxAssetBytes": 1024, "maxConcurrentCommands": 4, "maxQueue": 32, "maxFps": 60 }"#,
        );
        let err = import(dir.path(), &m, &sheet(png(2048))).unwrap_err();
        assert!(err.contains("at most 1024"), "{err}");
        assert!(import(dir.path(), &m, &sheet(png(1024))).is_ok());
        // 空資產也不收。
        let err = import(dir.path(), &manifest_json("empty", ""), &sheet(Vec::new())).unwrap_err();
        assert!(err.contains("empty"), "{err}");
        // 總量上限是常數且不超過協定的 32 MB 天花板。
        assert_eq!(MAX_TOTAL_IMPORT_BYTES, MAX_ASSET_BYTES_CEILING);
        // base64 長度守門：超過上限的字串在解碼前就被擋下。
        let huge = "A".repeat(((MAX_TOTAL_IMPORT_BYTES / 3 + 2) * 4) as usize);
        assert!(decode_asset_base64(&huge).is_err());
        assert!(decode_asset_base64("not base64!!").is_err());
        assert_eq!(decode_asset_base64("aGVsbG8=").expect("decode"), b"hello");
    }

    /// character-package-019：匯入上限不得高於「讀得回來」的上限。
    ///
    /// 舊行為：manifest 宣告 `maxAssetBytes: 16 MB` 時，一個 8–32 MB 的資產匯入成功
    /// （對話框顯示完成、清單列為 valid），之後每次選用都在 `asset_data_url` 失敗、
    /// 退回文字角色，使用者只看得到固定的「角色載入失敗」——accepted 被當成可用。
    #[test]
    fn import_never_accepts_an_asset_it_can_never_read_back() {
        let dir = root();
        let m = manifest_json(
            "toobig",
            r#",
  "resourceLimits": { "maxAssetBytes": 16000000, "maxConcurrentCommands": 4, "maxQueue": 32, "maxFps": 60 }"#,
        );
        // 8 MB + 1：在 manifest 宣告與 32 MB 天花板之內，但超過讀回上限。
        let oversize = (MAX_ASSET_DATA_URL_BYTES + 1) as usize;
        let err = import(dir.path(), &m, &sheet(png(oversize))).unwrap_err();
        assert!(err.contains("8 MB"), "{err}");
        assert!(
            !dir.path().join("toobig").exists(),
            "失敗的匯入不得留下資料夾"
        );
        // 反向不變量：匯入成功的資產一定讀得回來。
        import(dir.path(), &m, &sheet(png(4096))).expect("import ok");
        assert!(asset_data_url(dir.path(), "toobig", "sheet").is_ok());
        assert!(
            effective_max_asset_bytes(
                &parse_manifest(m.as_bytes(), &host_limits())
                    .expect("parse")
                    .0
            ) <= MAX_ASSET_DATA_URL_BYTES
        );
    }

    /// character-package-021：IPC 解碼要先擋筆數、邊解碼邊累計總量。
    #[test]
    fn ipc_asset_decoding_is_bounded_by_the_host_not_the_caller() {
        // 筆數上限在**解碼之前**生效：每一筆都是合法 base64，但筆數超量就直接回錯。
        let one = base64::engine::general_purpose::STANDARD.encode(png(64));
        let too_many: Vec<(String, String)> = (0..=MAX_IMPORT_ASSETS)
            .map(|i| (format!("a{i}"), one.clone()))
            .collect();
        let err = decode_import_assets(too_many).unwrap_err();
        assert!(err.contains("too many assets"), "{err}");
        // 筆數合法但總量超過 32 MB：解碼中途就中止，不會把全部收進記憶體。
        let chunk = base64::engine::general_purpose::STANDARD
            .encode(png((MAX_TOTAL_IMPORT_BYTES / 8) as usize));
        let heavy: Vec<(String, String)> = (0..MAX_IMPORT_ASSETS)
            .map(|i| (format!("a{i}"), chunk.clone()))
            .collect();
        let err = decode_import_assets(heavy).unwrap_err();
        assert!(err.contains("total import limit"), "{err}");
        // 正常情況照樣解得出來。
        let ok = decode_import_assets(vec![("sheet".to_string(), one)]).expect("decode");
        assert_eq!(ok.len(), 1);
        assert_eq!(ok[0].id, "sheet");
    }

    #[test]
    fn rejects_spoofed_magic_bytes_and_wrong_hashes() {
        let dir = root();
        // 宣告 image/png 但內容是 JPEG。
        let mut jpeg = JPEG_MAGIC.to_vec();
        jpeg.resize(64, 1);
        let err = import(dir.path(), &manifest_json("spoof", ""), &sheet(jpeg)).unwrap_err();
        assert!(
            err.contains("does not match its declared mediaType"),
            "{err}"
        );
        // 宣告 bytes 不符。
        let m = manifest_json("spoof", "").replace(
            "\"mediaType\": \"image/png\"",
            "\"mediaType\": \"image/png\", \"bytes\": 99",
        );
        let err = import(dir.path(), &m, &sheet(png(64))).unwrap_err();
        assert!(err.contains("declares 99"), "{err}");
        // 宣告 sha256 不符。
        let m = manifest_json("spoof", "").replace(
            "\"mediaType\": \"image/png\"",
            &format!(
                "\"mediaType\": \"image/png\", \"sha256\": \"{}\"",
                "0".repeat(64)
            ),
        );
        let err = import(dir.path(), &m, &sheet(png(64))).unwrap_err();
        assert!(err.contains("sha256"), "{err}");
        // 宣告 sha256 相符則通過。
        let bytes = png(64);
        let m = manifest_json("hashed", "").replace(
            "\"mediaType\": \"image/png\"",
            &format!(
                "\"mediaType\": \"image/png\", \"sha256\": \"{}\"",
                sha256_hex(&bytes)
            ),
        );
        assert!(import(dir.path(), &m, &sheet(bytes)).is_ok());
    }

    #[test]
    fn rejects_external_kinds_non_whitelisted_builtins_and_bundled_ids() {
        let dir = root();
        let external = manifest_json("ext", "")
            .replace(
                "\"adapterKind\": \"in-process\"",
                "\"adapterKind\": \"external-process\"",
            )
            .replace(
                "{ \"kind\": \"builtin\", \"id\": \"sprite\" }",
                "{ \"kind\": \"process\", \"command\": [\"node\", \"adapter.mjs\"] }",
            );
        let err = import(dir.path(), &external, &sheet(png(32))).unwrap_err();
        assert!(err.contains("/v1/character/adapters"), "{err}");
        assert!(
            !dir.path().join("ext").exists(),
            "nothing may be written for a rejected import"
        );

        let evil =
            manifest_json("evil", "").replace("\"id\": \"sprite\"", "\"id\": \"evil-plugin\"");
        let err = import(dir.path(), &evil, &sheet(png(32))).unwrap_err();
        assert!(
            err.contains("manifest invalid") || err.contains("whitelisted"),
            "{err}"
        );

        let err = import(dir.path(), &manifest_json("shu-maid", ""), &sheet(png(32))).unwrap_err();
        assert!(err.contains("bundled"), "{err}");
        assert!(list(dir.path()).is_empty());
    }

    #[test]
    fn declared_and_provided_assets_must_match_exactly() {
        let dir = root();
        let err = import(dir.path(), &manifest_json("m", ""), &[]).unwrap_err();
        assert!(err.contains("was not provided"), "{err}");
        let extra = vec![
            ImportAssetInput {
                id: "sheet".into(),
                bytes: png(32),
            },
            ImportAssetInput {
                id: "undeclared".into(),
                bytes: png(32),
            },
        ];
        let err = import(dir.path(), &manifest_json("m", ""), &extra).unwrap_err();
        assert!(err.contains("not declared"), "{err}");
    }

    #[test]
    fn asset_data_url_rechecks_path_size_and_magic() {
        let dir = root();
        import(dir.path(), &manifest_json("dl", ""), &sheet(png(64))).expect("import");
        let url = asset_data_url(dir.path(), "dl", "sheet").expect("data url");
        assert!(url.starts_with("data:image/png;base64,"), "{url}");
        // 路徑穿越／未宣告／不存在角色。
        assert!(asset_data_url(dir.path(), "dl", "../manifest.json").is_err());
        assert!(asset_data_url(dir.path(), "dl", "manifest.json").is_err());
        assert!(asset_data_url(dir.path(), "nope", "sheet").is_err());
        assert!(asset_data_url(dir.path(), "../dl", "sheet").is_err());
        // 磁碟上的內容被換成 JPEG → 拒絕（magic 重新核對）。
        let mut jpeg = JPEG_MAGIC.to_vec();
        jpeg.resize(64, 1);
        std::fs::write(dir.path().join("dl/assets/sheet"), jpeg).expect("swap");
        let err = asset_data_url(dir.path(), "dl", "sheet").unwrap_err();
        assert!(err.contains("does not match"), "{err}");
        // 超過 8 MB → 拒絕，而且不讀進記憶體再說（先看 metadata）。
        std::fs::write(
            dir.path().join("dl/assets/sheet"),
            png(MAX_ASSET_DATA_URL_BYTES as usize + 1),
        )
        .expect("write big");
        let err = asset_data_url(dir.path(), "dl", "sheet").unwrap_err();
        assert!(err.contains("8 MB"), "{err}");
    }

    #[test]
    fn remove_only_touches_imported_characters() {
        let dir = root();
        assert!(remove(dir.path(), "shu-maid")
            .unwrap_err()
            .contains("bundled"));
        assert!(remove(dir.path(), "plain-text")
            .unwrap_err()
            .contains("bundled"));
        assert!(remove(dir.path(), "ghost")
            .unwrap_err()
            .contains("not an imported"));
        assert!(remove(dir.path(), "../ghost").is_err());
        import(dir.path(), &manifest_json("gone", ""), &sheet(png(32))).expect("import");
        remove(dir.path(), "gone").expect("remove");
        assert!(!dir.path().join("gone").exists());
        assert!(list(dir.path()).is_empty());
    }

    /// 內建角色 id 由前端擁有的 `public/characters/index.json` 在編譯期 include 後解析而來：
    /// 這個測試釘住「解析結果 == 索引檔內容」，避免解析失敗被靜默吞成空清單。
    #[test]
    fn bundled_character_index_parses_and_matches_the_frontend_index() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../public/characters/index.json");
        let text = std::fs::read_to_string(path)
            .expect("public/characters/index.json (frontend-owned) must exist");
        let json: serde_json::Value = serde_json::from_str(&text).expect("index.json parses");
        let mut ids: Vec<String> = json["characters"]
            .as_array()
            .expect("characters array")
            .iter()
            .filter_map(|c| c["characterId"].as_str().map(String::from))
            .collect();
        ids.sort();
        let mut ours: Vec<String> = bundled_character_ids().to_vec();
        ours.sort();
        assert_eq!(ours, ids, "bundled ids must come from the character index");
        assert!(!ours.is_empty(), "index must not parse to an empty list");
        assert!(ours.len() <= MAX_BUNDLED_CHARACTERS);
        // 第二個 reference character 也在內建清單裡（匯入撞名保護涵蓋它）。
        assert!(ours.iter().any(|id| id == "ref-shape"));
        assert!(is_bundled("ref-shape"));
    }

    /// 壞索引不得 panic、不得產生無界清單，也不得混進不合法 id。
    #[test]
    fn a_broken_or_oversized_index_degrades_to_a_bounded_list() {
        assert!(parse_bundled_ids("{not json").is_empty());
        assert!(parse_bundled_ids("{}").is_empty());
        assert!(parse_bundled_ids(r#"{"characters": {}}"#).is_empty());
        assert_eq!(
            parse_bundled_ids(
                r#"{"characters": [{"characterId": "ok"}, {"characterId": "Bad Id"},
                     {"characterId": "ok"}, {"nope": 1}]}"#
            ),
            vec!["ok".to_string()]
        );
        let many: Vec<String> = (0..(MAX_BUNDLED_CHARACTERS + 20))
            .map(|i| format!(r#"{{"characterId": "c{i}"}}"#))
            .collect();
        let json = format!(r#"{{"characters": [{}]}}"#, many.join(","));
        assert_eq!(parse_bundled_ids(&json).len(), MAX_BUNDLED_CHARACTERS);
    }

    /// 匯入不得撞到第二個 reference character 的 id。
    #[test]
    fn importing_a_bundled_reference_character_id_is_refused() {
        let dir = root();
        let err = import(dir.path(), &manifest_json("ref-shape", ""), &sheet(png(32))).unwrap_err();
        assert!(err.contains("bundled"), "{err}");
        assert!(!dir.path().join("ref-shape").exists());
    }

    #[test]
    fn list_reports_corrupt_folders_honestly() {
        let dir = root();
        std::fs::create_dir_all(dir.path().join("broken")).expect("mkdir");
        std::fs::write(dir.path().join("broken/manifest.json"), b"{not json").expect("write");
        // 暫存與非 id 名稱的資料夾不列出。
        std::fs::create_dir_all(dir.path().join(".tmp-x-1")).expect("mkdir");
        std::fs::create_dir_all(dir.path().join("Not Valid")).expect("mkdir");
        let listed = list(dir.path());
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].character_id, "broken");
        assert!(!listed[0].valid);
        assert!(listed[0].error.is_some());
        // 資料夾名稱與 manifest characterId 不一致 → invalid。
        std::fs::create_dir_all(dir.path().join("renamed/assets")).expect("mkdir");
        std::fs::write(
            dir.path().join("renamed/manifest.json"),
            manifest_json("original", "").replace(
                "\"assets\": [ { \"id\": \"sheet\", \"path\": \"sheet.png\", \"mediaType\": \"image/png\" } ]",
                "\"assets\": []",
            ),
        )
        .expect("write");
        let listed = list(dir.path());
        let renamed = listed
            .iter()
            .find(|e| e.character_id == "renamed")
            .expect("renamed listed");
        assert!(!renamed.valid);
    }
}
