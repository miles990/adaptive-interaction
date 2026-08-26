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
    /// Selected character pack id (bundled: shu-standard/lively/minimal).
    pub companion_pack: String,
    /// Expressiveness: `quiet` | `natural` | `lively`.
    pub companion_expressiveness: String,
    /// Companion stays above other windows.
    pub companion_always_on_top: bool,
    pub schema_version: u32,
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
            companion_pack: "shu-standard".into(),
            companion_expressiveness: "natural".into(),
            companion_always_on_top: false,
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
    // Atomic write: temp file + rename.
    let tmp = path.with_extension("json.tmp");
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
    }

    #[test]
    fn desktop_prefs_roundtrip_and_unknown_fields_tolerated() {
        let json = r#"{"closeBehavior":"keep-running","askOnClose":false,"futureField":1}"#;
        let p: DesktopPrefs = serde_json::from_str(json).unwrap();
        assert_eq!(p.close_behavior.as_deref(), Some("keep-running"));
        assert!(!p.ask_on_close);
        // Unknown fields must not break loading (forward compatibility).
    }

    #[test]
    fn api_base_parses_flat_yaml_keys() {
        let v = serde_yaml_value("apiHost: 0.0.0.0\napiPort: 9000\nother: x\n").unwrap();
        assert_eq!(v.get("apiHost").unwrap().as_str().unwrap(), "0.0.0.0");
        assert_eq!(v.get("apiPort").unwrap().as_u64().unwrap(), 9000);
    }
}
