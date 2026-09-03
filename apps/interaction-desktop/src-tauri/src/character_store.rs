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
    BUILTIN_ENTRYPOINTS, MAX_ASSET_BYTES_CEILING,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::{Component, Path, PathBuf};

/// 一次匯入的資產總量上限。
pub const MAX_TOTAL_IMPORT_BYTES: u64 = 32 * 1024 * 1024;
/// `character_asset` 回傳 data URL 的單檔上限（WebView 記憶體考量）。
pub const MAX_ASSET_DATA_URL_BYTES: u64 = 8 * 1024 * 1024;
/// 內建角色 id（鏡射 `apps/interaction-desktop/public/characters/index.json`；
/// 該檔由前端擁有，host 不在編譯期讀它以免耦合建置）。匯入不得撞名、移除不得碰。
pub const BUNDLED_CHARACTER_IDS: [&str; 9] = [
    "shu-maid",
    "shu-maid-dusk",
    "shu-maid-sakura",
    "shu-standard",
    "shu-minimal",
    "shu-lively",
    "shu-agile",
    "shu-lazy",
    "plain-text",
];
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
    /// builtin entrypoint id（shu-rig／sprite／text）。
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
    BUNDLED_CHARACTER_IDS.contains(&character_id)
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

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

fn effective_max_asset_bytes(manifest: &CharacterManifest) -> u64 {
    manifest
        .resource_limits
        .max_asset_bytes
        .min(MAX_ASSET_BYTES_CEILING)
}

/// 純驗證（不碰檔案系統）：manifest 規則＋host 規則＋每個資產的大小／magic／sha256。
pub fn validate_import(
    manifest_text: &str,
    assets: &[ImportAssetInput],
) -> Result<ValidatedImport, String> {
    let (manifest, report) = parse_manifest(manifest_text.as_bytes(), &ValidationLimits::default())
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
    match &manifest.entrypoint {
        Entrypoint::Builtin { id } if BUILTIN_ENTRYPOINTS.contains(&id.as_str()) => {}
        Entrypoint::Builtin { .. } => {
            return Err(format!(
                "entrypoint must be a whitelisted builtin ({})",
                BUILTIN_ENTRYPOINTS.join("/")
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
            return Err(format!(
                "asset '{}' is {len} bytes; the manifest allows at most {max_asset}",
                decl.id
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
    parse_manifest(&bytes, &ValidationLimits::default())
        .map_err(|e| format!("manifest invalid: {e}"))
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

    /// `BUNDLED_CHARACTER_IDS` 是前端 `public/characters/index.json` 的鏡射：兩邊必須一致，
    /// 否則匯入撞名／移除保護會漏掉新加入的內建角色。
    #[test]
    fn bundled_ids_mirror_the_frontend_index() {
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
        let mut ours: Vec<String> = BUNDLED_CHARACTER_IDS
            .iter()
            .map(|s| s.to_string())
            .collect();
        ours.sort();
        assert_eq!(
            ours, ids,
            "update BUNDLED_CHARACTER_IDS when the bundled character index changes"
        );
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
