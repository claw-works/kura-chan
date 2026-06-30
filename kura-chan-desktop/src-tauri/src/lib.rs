mod audio;
mod ws;

use std::collections::HashMap;
use std::path::PathBuf;

use audio::Recorder;
use serde_json::json;
use tauri::{Manager, State};
use ws::{WsHandle, WsOut};

/// ~/.kura — agent data dir (config, later: skills, chat history, etc.)
fn kura_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".kura"))
}
/// Parse ~/.kura/.env (KEY=VALUE lines) into a map.
fn load_kura_env() -> HashMap<String, String> {
    let mut map = HashMap::new();
    if let Some(path) = kura_dir().map(|d| d.join(".env")) {
        if let Ok(content) = std::fs::read_to_string(path) {
            for line in content.lines() {
                let l = line.trim();
                if l.is_empty() || l.starts_with('#') {
                    continue;
                }
                if let Some((k, v)) = l.split_once('=') {
                    map.insert(k.trim().to_string(), v.trim().trim_matches('"').to_string());
                }
            }
        }
    }
    map
}

#[derive(Clone)]
struct Settings {
    ws_url: String,
    http_base: String,
    api_key: String,
    device_id: String,
}
/// Resolve settings: ~/.kura/.env > process env > built-in default.
fn load_settings() -> Settings {
    let kenv = load_kura_env();
    let get = |k: &str, d: &str| {
        kenv.get(k)
            .cloned()
            .or_else(|| std::env::var(k).ok())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| d.to_string())
    };
    Settings {
        ws_url: get("KURA_WS_URL", "ws://127.0.0.1:18099/ws/device"),
        http_base: get("KURA_HTTP_BASE", "http://127.0.0.1:18099"),
        api_key: get("KURA_API_KEY", ""),
        device_id: get("KURA_DEVICE_ID", "KURA_DESKTOP_001"),
    }
}

/// Most recent sync (gender/appearance/growth) — frontend queries on startup
/// in case it missed the connect-time sync event.
#[tauri::command]
fn get_last_sync(handle: State<WsHandle>) -> Option<serde_json::Value> {
    handle
        .last_sync
        .lock()
        .ok()
        .and_then(|s| s.clone())
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
}

/// Persist the floating-window position to ~/.kura/window.json.
#[tauri::command]
fn save_window_pos(x: i32, y: i32) {
    if let Some(dir) = kura_dir() {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(
            dir.join("window.json"),
            serde_json::json!({ "x": x, "y": y }).to_string(),
        );
    }
}

/// Read the saved window position, if any.
#[tauri::command]
fn get_window_pos() -> Option<serde_json::Value> {
    let p = kura_dir()?.join("window.json");
    let c = std::fs::read_to_string(p).ok()?;
    serde_json::from_str::<serde_json::Value>(&c).ok()
}

/// Current settings for the settings UI.
#[tauri::command]
fn get_settings() -> serde_json::Value {
    let s = load_settings();
    json!({
        "wsUrl": s.ws_url,
        "httpBase": s.http_base,
        "apiKey": s.api_key,
        "deviceId": s.device_id,
    })
}

/// Write settings to ~/.kura/.env. Takes effect on next launch (reconnect TBD).
#[tauri::command]
fn save_settings(
    ws_url: String,
    http_base: String,
    api_key: String,
    device_id: String,
) -> Result<(), String> {
    let dir = kura_dir().ok_or_else(|| "no HOME".to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let content = format!(
        "# Kura-chan 桌面端配置（设置面板写入）\n\
         KURA_WS_URL={ws_url}\n\
         KURA_HTTP_BASE={http_base}\n\
         KURA_API_KEY={api_key}\n\
         KURA_DEVICE_ID={device_id}\n"
    );
    std::fs::write(dir.join(".env"), content).map_err(|e| e.to_string())?;
    Ok(())
}

/// Send a typed message to the agent (text input → server, bypasses STT).
#[tauri::command]
fn send_text(state: State<WsHandle>, text: String) -> Result<(), String> {
    let t = text.trim();
    if t.is_empty() {
        return Ok(());
    }
    let msg = json!({ "type": "text_input", "text": t });
    state.tx.send(WsOut::Text(msg.to_string())).map_err(|e| e.to_string())
}

/// Frontend config: server HTTP base (for portrait PNGs) + current WS status.
#[tauri::command]
fn get_config(state: State<WsHandle>) -> serde_json::Value {
    let status = state.status.lock().map(|s| s.clone()).unwrap_or_default();
    json!({ "httpBase": state.http_base, "status": status })
}

/// Begin microphone capture.
#[tauri::command]
fn start_recording(recorder: State<Recorder>) {
    recorder.start();
}

/// Stop capture and stream the recorded PCM to the server as audio_input frames.
#[tauri::command]
fn stop_recording(recorder: State<Recorder>, ws: State<WsHandle>) {
    let pcm = recorder.stop();
    audio::send_pcm(&ws.tx, &pcm);
}

/// VAD: has the user stopped talking (trailing silence)? Frontend polls this
/// while recording to auto-send without a second button press.
#[tauri::command]
fn is_voice_done(recorder: State<Recorder>) -> bool {
    recorder.is_done()
}

/// Fetch conversation history from the server (Bearer api_key), cache it to
/// ~/.kura/history.json, and return it. Falls back to the cache when offline.
#[tauri::command]
async fn get_history() -> Result<serde_json::Value, String> {
    let s = load_settings();
    let cache = kura_dir().map(|d| d.join("history.json"));
    let url = format!("{}/history", s.http_base.trim_end_matches('/'));
    let fetched: Result<serde_json::Value, String> = async {
        let resp = reqwest::Client::new()
            .get(&url)
            .bearer_auth(&s.api_key)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("HTTP {}", resp.status()));
        }
        resp.json::<serde_json::Value>().await.map_err(|e| e.to_string())
    }
    .await;

    match fetched {
        Ok(json) => {
            if let Some(p) = &cache {
                if let Some(parent) = p.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(p, serde_json::to_string(&json).unwrap_or_default());
            }
            Ok(json)
        }
        Err(e) => {
            // offline: fall back to the cached copy if present
            if let Some(p) = &cache {
                if let Ok(c) = std::fs::read_to_string(p) {
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&c) {
                        return Ok(json);
                    }
                }
            }
            Err(e)
        }
    }
}
#[tauri::command]
fn read_dropped(path: String) -> Result<String, String> {
    let meta = std::fs::metadata(&path).map_err(|e| e.to_string())?;
    if meta.len() > 200_000 {
        return Err("文件太大(>200KB)".into());
    }
    std::fs::read_to_string(&path).map_err(|_| "不是文本文件或无法读取".to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // Connection config from ~/.kura/.env (falls back to env / defaults).
            let s = load_settings();
            let handle = ws::connect(
                app.handle().clone(),
                s.ws_url,
                s.http_base,
                s.api_key,
                s.device_id,
            );
            app.manage(handle);
            app.manage(Recorder::new());

            // status-bar tray icon with a quit menu
            let quit = tauri::menu::MenuItem::with_id(app, "quit", "退出小爪", true, None::<&str>)?;
            let menu = tauri::menu::Menu::with_items(app, &[&quit])?;
            let _tray = tauri::tray::TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .menu(&menu)
                .on_menu_event(|app, event| {
                    if event.id.as_ref() == "quit" {
                        app.exit(0);
                    }
                })
                .build(app)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            send_text,
            get_config,
            start_recording,
            stop_recording,
            is_voice_done,
            read_dropped,
            get_settings,
            save_settings,
            get_history,
            get_last_sync,
            save_window_pos,
            get_window_pos
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
