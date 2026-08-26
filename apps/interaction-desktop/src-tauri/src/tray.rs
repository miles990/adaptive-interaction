//! Status-bar (tray / menu-bar) presence: state display, quick actions and an
//! always-available emergency stop that goes straight through Rust — never
//! through the WebView.
//!
//! State is never conveyed by color alone: the menu carries text for every
//! state, and on macOS the icon gets a text glyph title as a secondary cue.

use crate::DesktopState;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, Wry};

pub struct TrayHandles {
    pub tray: tauri::tray::TrayIcon<Wry>,
    pub info_status: MenuItem<Wry>,
    pub info_pause: MenuItem<Wry>,
    pub info_sessions: MenuItem<Wry>,
    pub toggle_pause: MenuItem<Wry>,
    pub toggle_companion: MenuItem<Wry>,
}

pub fn build(app: &AppHandle) -> tauri::Result<TrayHandles> {
    let info_status =
        MenuItem::with_id(app, "info_status", "系統狀態：啟動中…", false, None::<&str>)?;
    let info_pause = MenuItem::with_id(app, "info_pause", "主動互動：－", false, None::<&str>)?;
    let info_sessions =
        MenuItem::with_id(app, "info_sessions", "AI 工作階段：－", false, None::<&str>)?;
    let open = MenuItem::with_id(app, "open", "開啟控制中心", true, None::<&str>)?;
    let toggle_companion =
        MenuItem::with_id(app, "toggle_companion", "顯示桌面角色", true, None::<&str>)?;
    let toggle_pause = MenuItem::with_id(app, "toggle_pause", "暫停主動互動", true, None::<&str>)?;
    let pause_hour = MenuItem::with_id(app, "pause_hour", "暫停一小時", true, None::<&str>)?;
    let estop = MenuItem::with_id(app, "estop", "緊急停止", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "設定…", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "完全結束", true, None::<&str>)?;

    let menu = Menu::with_items(
        app,
        &[
            &info_status,
            &info_pause,
            &info_sessions,
            &PredefinedMenuItem::separator(app)?,
            &open,
            &toggle_companion,
            &toggle_pause,
            &pause_hour,
            &PredefinedMenuItem::separator(app)?,
            &estop,
            &PredefinedMenuItem::separator(app)?,
            &settings,
            &quit,
        ],
    )?;

    let tray = TrayIconBuilder::with_id("main-tray")
        .icon(app.default_window_icon().cloned().expect("bundled icon"))
        .icon_as_template(true)
        .menu(&menu)
        .show_menu_on_left_click(true)
        .tooltip("Adaptive Interaction")
        .on_menu_event(|app, event| on_menu_event(app, event.id().as_ref()))
        .build(app)?;

    Ok(TrayHandles {
        tray,
        info_status,
        info_pause,
        info_sessions,
        toggle_pause,
        toggle_companion,
    })
}

fn on_menu_event(app: &AppHandle, id: &str) {
    match id {
        "open" | "settings" => {
            crate::show_main_window(app, id == "settings");
        }
        "toggle_companion" => {
            crate::toggle_companion_window(app);
        }
        "toggle_pause" => {
            dispatch(app, TrayAction::TogglePause);
        }
        "pause_hour" => {
            dispatch(app, TrayAction::PauseOneHour);
        }
        "estop" => {
            // Straight to the backend; no WebView involved, never blocked by UI.
            dispatch(app, TrayAction::EmergencyStop);
        }
        "quit" => {
            crate::full_quit_from_tray(app);
        }
        _ => {}
    }
}

enum TrayAction {
    TogglePause,
    PauseOneHour,
    EmergencyStop,
}

fn dispatch(app: &AppHandle, action: TrayAction) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<DesktopState>();
        let backend = state.backend();
        let result = match (&backend, action) {
            (Some(b), TrayAction::TogglePause) => match b.pause_status().await {
                Ok(paused) if paused => b.resume().await,
                Ok(_) => b.pause(None).await,
                Err(e) => Err(e),
            },
            (Some(b), TrayAction::PauseOneHour) => b.pause(Some(60)).await,
            (Some(b), TrayAction::EmergencyStop) => b.emergency_stop("tray").await,
            (None, _) => Err("runtime not available".to_string()),
        };
        if let Err(e) = result {
            tracing::error!(error = %e, "tray action failed");
            // Surface the failure: honesty over silence.
            let _ = tauri::Emitter::emit(&app, "tray-action-error", e);
        }
        crate::refresh_tray(&app).await;
    });
}

/// Compose the status line texts + macOS title glyph for the current state.
pub struct TrayView {
    pub status_text: String,
    pub pause_text: String,
    pub sessions_text: String,
    pub pause_action_text: String,
    pub title_glyph: Option<&'static str>,
}

pub fn tray_view(
    supervisor_ready: bool,
    external: bool,
    estop: bool,
    paused: bool,
    ai_sessions: usize,
    mic_active: bool,
    camera_active: bool,
) -> TrayView {
    let status_text = if !supervisor_ready {
        "系統狀態：離線（無法連線 Runtime）".to_string()
    } else if estop {
        "系統狀態：緊急停止中".to_string()
    } else if external {
        "系統狀態：已連線外部 Runtime".to_string()
    } else {
        "系統狀態：正常（內嵌 Runtime）".to_string()
    };
    let pause_text = if paused {
        "主動互動：已暫停".to_string()
    } else {
        "主動互動：進行中".to_string()
    };
    let sessions_text = format!("AI 工作階段：{ai_sessions}");
    let pause_action_text = if paused {
        "恢復主動互動".to_string()
    } else {
        "暫停主動互動".to_string()
    };
    let title_glyph = if !supervisor_ready {
        Some("⚠")
    } else if estop {
        Some("⛔")
    } else if mic_active {
        Some("🎙")
    } else if camera_active {
        Some("📷")
    } else if paused {
        Some("⏸")
    } else {
        None
    };
    TrayView {
        status_text,
        pause_text,
        sessions_text,
        pause_action_text,
        title_glyph,
    }
}

#[cfg(test)]
mod tests {
    use super::tray_view;

    #[test]
    fn tray_states_always_carry_text_not_only_color() {
        let offline = tray_view(false, false, false, false, 0, false, false);
        assert!(offline.status_text.contains("離線"));
        assert_eq!(offline.title_glyph, Some("⚠"));

        let estop = tray_view(true, false, true, false, 0, false, false);
        assert!(estop.status_text.contains("緊急停止"));
        assert_eq!(estop.title_glyph, Some("⛔"));

        let paused = tray_view(true, false, false, true, 2, false, false);
        assert!(paused.pause_text.contains("已暫停"));
        assert!(paused.sessions_text.contains('2'));
        assert_eq!(paused.pause_action_text, "恢復主動互動");
        assert_eq!(paused.title_glyph, Some("⏸"));

        // Estop outranks pause and sensor glyphs.
        let both = tray_view(true, false, true, true, 0, true, true);
        assert_eq!(both.title_glyph, Some("⛔"));

        // Sensor use is visible even while otherwise normal.
        let mic = tray_view(true, true, false, false, 0, true, false);
        assert_eq!(mic.title_glyph, Some("🎙"));
        assert!(mic.status_text.contains("外部"));
    }
}
