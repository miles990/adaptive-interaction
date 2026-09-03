//! RuntimeSupervisor: decides whether this desktop app OWNS an embedded
//! runtime or CONNECTS to an existing external `interact-ai serve` daemon.
//!
//! Rules (spec §6):
//! - Never start two runtimes (the runtime's instance lock is the backstop;
//!   the supervisor probes first so the normal path never even races).
//! - Fully quitting the desktop app must NOT kill an external daemon.
//! - Closing the control-center window hides it; only "完全結束" shuts the
//!   embedded runtime down.
//! - The app must never look healthy while the runtime is actually offline:
//!   the external-mode health loop demotes state and tells the UI.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

/// Supervisor lifecycle states (spec-mandated set).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SupervisorState {
    Starting,
    EmbeddedOwned,
    ConnectedToExternal,
    Ready,
    Degraded,
    Disconnected,
    Stopping,
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SupervisorMode {
    Embedded,
    External,
    Undecided,
}

/// What the frontend needs to pick its transport.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SupervisorInfo {
    pub mode: SupervisorMode,
    pub state: SupervisorState,
    pub api_base: String,
    /// Only set in external mode (the WebView then talks HTTP itself).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl SupervisorInfo {
    pub fn starting() -> Self {
        Self {
            mode: SupervisorMode::Undecided,
            state: SupervisorState::Starting,
            api_base: String::new(),
            token: None,
            detail: None,
        }
    }
}

/// Desktop-app-local preferences (close behavior, companion visibility…).
/// Lives in the interaction home as `state/desktop.json`: desktop lifecycle
/// is a desktop concern, so it works identically in embedded and external
/// modes without needing the runtime.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DesktopPrefs {
    /// `keep-running` | `hide-companion` | `quit`; None = not decided yet
    /// (first close shows the explanation dialog — this is also the upgrade
    /// notice: v0.2 quit on close, v0.3 keeps running only after the user
    /// confirms).
    pub close_behavior: Option<String>,
    /// Show the dialog on every close until the user opts out.
    pub ask_on_close: bool,
    /// Launch at login (mirrors the OS autostart entry; default off).
    pub launch_at_login: bool,
    pub show_companion_on_start: bool,
    pub open_control_center_on_start: bool,
    /// Desktop companion visibility (Phase 2).
    pub companion_visible: bool,
    /// Remembered companion window position (logical px), per display setups.
    pub companion_position: Option<(f64, f64)>,
    /// Companion surface size in logical pixels (bounded by the native bridge).
    pub companion_size: (f64, f64),
    /// Surface opacity, 0.2..=1.0. Applied inside the transparent WebView.
    pub companion_opacity: f64,
    /// 目前角色的 `characterId`（Character Presentation Protocol manifest 身分；
    /// 見 `docs/character-protocol/README.md` §2.2）。舊的 8 個 pack id
    /// （shu-maid／shu-maid-dusk／shu-maid-sakura／shu-standard／shu-minimal／
    /// shu-lively／shu-agile／shu-lazy）一律仍可用，與 `public/characters/index.json`
    /// 及 `state/characters/<characterId>/`（匯入角色）對應。預設維持 `shu-maid`
    /// 以相容既有設定檔。
    pub companion_pack: String,
    /// Persona pack id（純資料；`persona-shu`／`persona-navigator` 等）。
    /// manifest 以 `preferences.persona` 引用它，不隨角色引擎改變。
    pub companion_persona: String,
    /// Story chapters already shown (fire once; clearable by the user).
    pub story_progress: std::collections::BTreeMap<String, bool>,
    /// Expressiveness: `quiet` | `natural` | `lively`.
    pub companion_expressiveness: String,
    /// Companion stays above other windows.
    pub companion_always_on_top: bool,
    /// 角色名字（顯示用；不影響任何權限）。也是角色視窗的標題。
    pub companion_name: String,
    /// 遊玩場景：`none` | `nest` | `desk` | `sill` | `night`。
    pub companion_scene: String,
    /// 玩耍（玩具/追逐）開關。
    pub companion_play: bool,
    /// 游標互動（光點/逗貓棒跟隨）開關。
    pub companion_cursor_play: bool,
    /// 主動靠近／看向游標開關。
    pub companion_approach: bool,
    /// 桌面（遊玩場內）自主移動開關。
    pub companion_desk_move: bool,
    /// 小型使魔（最多 3 隻；純呈現，無任何權限）。
    pub companion_familiars: Vec<FamiliarPref>,
    /// 勿擾：角色進入安靜基態（不主動靠近、不主動說話）。預設關閉。
    pub companion_do_not_disturb: bool,
    /// 說話氣泡開關（關掉後只剩固定的安全文字）。預設開啟。
    pub companion_bubbles: bool,
    /// 角色音效開關。**預設關閉**（不主動出聲）。
    pub companion_sound: bool,
    /// 允許用滑鼠拖曳角色視窗。預設開啟。
    pub companion_drag_enabled: bool,
    /// 使用者要求的本機安靜期到期時間（epoch ms；0＝沒有）。
    /// 只擋角色的主動行為（隨口氣泡／hover 短句／ambient 表演）；
    /// 安全文字（緊急停止、被擋下、未知、失敗）永遠不受它影響。
    pub companion_proactive_quiet_until: f64,
    /// 角色互動記憶（最喜歡的玩具、近期玩耍、常關掉的反應、熟悉度）。
    /// 純呈現資料，有界（≤8 玩具、≤20 事件），不會升級成正式知識。
    pub companion_interaction_memory: CompanionInteractionMemory,
    /// 各角色由 manifest.preferencesSchema 宣告的偏好值（characterId → 鍵 → 值）。
    /// 純呈現資料、無任何權限語意。上限（≤16 個角色、每角色 ≤32 鍵、值只能是
    /// bool／有限數字／≤200 字的字串）由 `desktop_prefs_patch` 強制；patch 帶了
    /// 這個欄位就整張表取代（前端一律送完整的表）。
    pub companion_preferences:
        std::collections::BTreeMap<String, std::collections::BTreeMap<String, serde_json::Value>>,
    pub schema_version: u32,
}

/// 角色互動記憶（spec §11 第一類）。欄位與前端 interactionMemory.ts 對應；
/// 上限在兩邊都強制，避免偏好檔無限成長。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct CompanionInteractionMemory {
    pub toys: Vec<MemoryCount>,
    pub disabled_reactions: Vec<MemoryCount>,
    pub events: Vec<MemoryEvent>,
    pub days_seen: u32,
    pub last_day: i64,
    pub last_seen_at: f64,
}

/// 最多記幾種玩具/反應。
pub const MEMORY_MAX_COUNTS: usize = 8;
/// 最多記幾筆近期事件。
pub const MEMORY_MAX_EVENTS: usize = 20;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct MemoryCount {
    pub kind: String,
    pub count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct MemoryEvent {
    pub at: f64,
    pub kind: String,
    pub detail: String,
}

impl CompanionInteractionMemory {
    /// 有界化：超過上限就截斷（最近的事件、最常玩的玩具優先留下）。
    pub fn bounded(mut self) -> Self {
        self.toys.sort_by(|a, b| b.count.cmp(&a.count));
        self.toys.truncate(MEMORY_MAX_COUNTS);
        self.disabled_reactions
            .sort_by(|a, b| b.count.cmp(&a.count));
        self.disabled_reactions.truncate(MEMORY_MAX_COUNTS);
        if self.events.len() > MEMORY_MAX_EVENTS {
            let drop = self.events.len() - MEMORY_MAX_EVENTS;
            self.events.drain(0..drop);
        }
        for e in &mut self.events {
            // 以「字元」截斷，不是 byte：detail 來自 WebView（任意 UTF-8），
            // byte 48 落在 CJK／emoji 中間會讓 String::truncate panic。
            let cut = e
                .detail
                .char_indices()
                .nth(MEMORY_MAX_DETAIL_CHARS)
                .map(|(i, _)| i)
                .unwrap_or(e.detail.len());
            e.detail.truncate(cut);
        }
        self
    }
}

/// 每筆事件 detail 最多幾個字元（與前端 interactionMemory.ts 的 slice(0, 48) 對應）。
pub const MEMORY_MAX_DETAIL_CHARS: usize = 48;

/// 使魔設定（純呈現資料）。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct FamiliarPref {
    pub id: String,
    pub name: String,
    pub palette: String,
}

impl Default for DesktopPrefs {
    fn default() -> Self {
        Self {
            close_behavior: None,
            ask_on_close: true,
            launch_at_login: false,
            show_companion_on_start: true,
            open_control_center_on_start: false,
            companion_visible: true,
            companion_position: None,
            companion_size: (200.0, 210.0),
            companion_opacity: 1.0,
            // characterId；預設仍是 shu-maid（相容 v0.3–v0.5 的設定檔）。
            companion_pack: "shu-maid".into(),
            companion_persona: "persona-shu".into(),
            story_progress: std::collections::BTreeMap::new(),
            companion_expressiveness: "natural".into(),
            companion_always_on_top: false,
            companion_name: "小樞".into(),
            companion_scene: "none".into(),
            companion_play: true,
            companion_cursor_play: true,
            companion_approach: true,
            companion_desk_move: true,
            companion_familiars: Vec::new(),
            companion_do_not_disturb: false,
            companion_bubbles: true,
            // 預設不出聲：音效要使用者自己打開。
            companion_sound: false,
            companion_drag_enabled: true,
            companion_proactive_quiet_until: 0.0,
            companion_interaction_memory: CompanionInteractionMemory::default(),
            companion_preferences: std::collections::BTreeMap::new(),
            schema_version: 1,
        }
    }
}

pub fn interaction_home() -> PathBuf {
    std::env::var_os("INTERACT_AI_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs_home()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".adaptive-interaction")
        })
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn prefs_path() -> PathBuf {
    interaction_home().join("state").join("desktop.json")
}

pub fn load_prefs() -> DesktopPrefs {
    let path = prefs_path();
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_prefs(prefs: &DesktopPrefs) -> Result<(), String> {
    let path = prefs_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    // Atomic write: unique temp file + rename, so two concurrent savers never
    // clobber each other's half-written temp (each renames its own).
    let tmp = path.with_extension(format!("json.tmp.{}", std::process::id()));
    let body = serde_json::to_string_pretty(prefs).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, body).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
    Ok(())
}

/// Read the daemon's configured API address from the shared home config.
pub fn configured_api_base() -> String {
    let cfg = interaction_home().join("config").join("interaction.yaml");
    let (mut host, mut port) = ("127.0.0.1".to_string(), 8787u16);
    if let Ok(text) = std::fs::read_to_string(cfg) {
        if let Ok(v) = serde_yaml_value(&text) {
            if let Some(h) = v.get("apiHost").and_then(|x| x.as_str()) {
                host = h.to_string();
            }
            if let Some(p) = v.get("apiPort").and_then(|x| x.as_u64()) {
                port = p as u16;
            }
        }
    }
    format!("http://{host}:{port}")
}

fn serde_yaml_value(text: &str) -> Result<serde_json::Value, String> {
    // Minimal YAML → JSON via serde_json's YAML-ish subset is not available;
    // parse the two flat keys we need without a YAML dependency.
    let mut map = serde_json::Map::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some((k, v)) = line.split_once(':') {
            let key = k.trim();
            let val = v.trim().trim_matches('"').trim_matches('\'');
            if key == "apiHost" {
                map.insert(key.into(), serde_json::Value::String(val.into()));
            } else if key == "apiPort" {
                if let Ok(n) = val.parse::<u64>() {
                    map.insert(key.into(), serde_json::Value::Number(n.into()));
                }
            }
        }
    }
    Ok(serde_json::Value::Object(map))
}

pub fn read_api_token() -> Option<String> {
    std::fs::read_to_string(interaction_home().join("state").join("api-token"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Probe an existing daemon. Returns true only on a real HTTP 200 from /ready.
pub async fn daemon_ready(api_base: &str) -> bool {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_millis(1200))
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };
    matches!(
        client.get(format!("{api_base}/ready")).send().await,
        Ok(resp) if resp.status().is_success()
    )
}

/// Authorized GET against the daemon (external mode helpers for tray actions).
pub async fn daemon_get(
    api_base: &str,
    token: &str,
    path: &str,
) -> Result<serde_json::Value, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(format!("{api_base}{path}"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("{} on {path}", resp.status()));
    }
    resp.json().await.map_err(|e| e.to_string())
}

pub async fn daemon_post(
    api_base: &str,
    token: &str,
    path: &str,
    body: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .post(format!("{api_base}{path}"))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("{} on {path}", resp.status()));
    }
    Ok(resp.json().await.unwrap_or(serde_json::Value::Null))
}

/// DELETE 到外部 daemon（撤銷外部 character adapter 等）；非 2xx 誠實回 Err。
pub async fn daemon_delete(
    api_base: &str,
    token: &str,
    path: &str,
) -> Result<serde_json::Value, String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .delete(format!("{api_base}{path}"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("{} on {path}", resp.status()));
    }
    Ok(resp.json().await.unwrap_or(serde_json::Value::Null))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_prefs_default_is_conservative() {
        let p = DesktopPrefs::default();
        // Launch-at-login must be opt-in; first close must ask.
        assert!(!p.launch_at_login);
        assert!(p.ask_on_close);
        assert!(p.close_behavior.is_none());
        assert!(!p.open_control_center_on_start);
        // v0.5：音效預設關閉；勿擾預設關閉；氣泡與拖曳預設開啟。
        assert!(!p.companion_sound);
        assert!(!p.companion_do_not_disturb);
        assert!(p.companion_bubbles);
        assert!(p.companion_drag_enabled);
    }

    #[test]
    fn interaction_memory_is_bounded() {
        let mem = CompanionInteractionMemory {
            toys: (0..20)
                .map(|i| MemoryCount {
                    kind: format!("toy{i}"),
                    count: i,
                })
                .collect(),
            disabled_reactions: (0..12)
                .map(|i| MemoryCount {
                    kind: format!("r{i}"),
                    count: i,
                })
                .collect(),
            events: (0..50)
                .map(|i| MemoryEvent {
                    at: i as f64,
                    kind: "play".into(),
                    detail: "x".repeat(80),
                })
                .collect(),
            days_seen: 3,
            last_day: 20_000,
            last_seen_at: 1.0,
        }
        .bounded();
        assert_eq!(mem.toys.len(), MEMORY_MAX_COUNTS);
        assert_eq!(mem.disabled_reactions.len(), MEMORY_MAX_COUNTS);
        assert_eq!(mem.events.len(), MEMORY_MAX_EVENTS);
        // 留下的是最近的事件（最舊的被丟掉）。
        assert_eq!(mem.events[0].at, 30.0);
        assert!(mem.events[0].detail.len() <= 48);
        // 玩最多次的玩具排在最前面。
        assert_eq!(mem.toys[0].count, 19);
    }

    /// regression：detail 曾以 byte 索引 `truncate(48)`——多位元組字元跨
    /// 邊界就 panic，prefs_patch 整個中斷。截斷必須以字元為單位。
    #[test]
    fn interaction_memory_truncates_detail_on_char_boundaries() {
        let cjk = format!("x{}", "毛線球".repeat(8)); // 25 chars／73 bytes，byte 48 非邊界
        let mixed = format!("a{}", "あ".repeat(47)); // 48 chars／142 bytes
        let emoji = "🧶🐈🎀".repeat(20); // 60 chars，每個 4 bytes
        let mem = CompanionInteractionMemory {
            events: [cjk.clone(), mixed.clone(), emoji.clone()]
                .into_iter()
                .enumerate()
                .map(|(i, detail)| MemoryEvent {
                    at: i as f64,
                    kind: "play".into(),
                    detail,
                })
                .collect(),
            ..Default::default()
        }
        .bounded();
        assert_eq!(mem.events.len(), 3);
        for e in &mem.events {
            assert!(
                e.detail.chars().count() <= MEMORY_MAX_DETAIL_CHARS,
                "{:?}",
                e.detail
            );
            assert!(std::str::from_utf8(e.detail.as_bytes()).is_ok());
        }
        // 不超過上限的 CJK 原樣保留；超過的只留前 48 個「字」。
        assert_eq!(mem.events[0].detail, cjk);
        assert_eq!(mem.events[1].detail, mixed);
        assert_eq!(
            mem.events[2].detail,
            emoji.chars().take(48).collect::<String>()
        );
    }

    #[test]
    fn desktop_prefs_roundtrip_and_unknown_fields_tolerated() {
        let json = r#"{"closeBehavior":"keep-running","askOnClose":false,"futureField":1}"#;
        let p: DesktopPrefs = serde_json::from_str(json).unwrap();
        assert_eq!(p.close_behavior.as_deref(), Some("keep-running"));
        assert!(!p.ask_on_close);
        // Unknown fields must not break loading (forward compatibility).
        // 舊設定檔沒有 companionPreferences：預設空表，不是錯誤。
        assert!(p.companion_preferences.is_empty());
    }

    /// 角色偏好（manifest.preferencesSchema 的值）必須真的被 host 保存並以
    /// camelCase `companionPreferences` 回傳——否則角色頁會誠實退回 localStorage。
    #[test]
    fn companion_preferences_roundtrip_with_camel_case_key() {
        let json = r#"{"companionPreferences":{"my-char":{"variant":"dusk","volume":0.5,"bubbles":true}}}"#;
        let p: DesktopPrefs = serde_json::from_str(json).unwrap();
        let entry = p
            .companion_preferences
            .get("my-char")
            .expect("character map kept");
        assert_eq!(entry["variant"], serde_json::json!("dusk"));
        assert_eq!(entry["volume"], serde_json::json!(0.5));
        assert_eq!(entry["bubbles"], serde_json::json!(true));

        let out = serde_json::to_value(&p).unwrap();
        assert_eq!(out["companionPreferences"]["my-char"]["variant"], "dusk");
        assert!(out.get("companion_preferences").is_none(), "camelCase only");
        // 預設值序列化成空物件（前端讀到 {} 而不是 undefined）。
        let default = serde_json::to_value(DesktopPrefs::default()).unwrap();
        assert_eq!(default["companionPreferences"], serde_json::json!({}));
    }

    #[test]
    fn api_base_parses_flat_yaml_keys() {
        let v = serde_yaml_value("apiHost: 0.0.0.0\napiPort: 9000\nother: x\n").unwrap();
        assert_eq!(v.get("apiHost").unwrap().as_str().unwrap(), "0.0.0.0");
        assert_eq!(v.get("apiPort").unwrap().as_u64().unwrap(), 9000);
    }
}
