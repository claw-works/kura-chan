mod audio;
mod ws;

use audio::Recorder;
use serde_json::json;
use tauri::{Manager, State};
use ws::{WsHandle, WsOut};

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

/// Read a dropped file as UTF-8 text (small files only). Used by drag-and-drop:
/// the frontend reads the file then sends its content into the conversation.
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
            // Connection config from env for now (a tray settings UI comes later).
            let ws_url = std::env::var("KURA_WS_URL")
                .unwrap_or_else(|_| "ws://127.0.0.1:18099/ws/device".into());
            let http_base = std::env::var("KURA_HTTP_BASE")
                .unwrap_or_else(|_| "http://127.0.0.1:18099".into());
            let api_key = std::env::var("KURA_API_KEY").unwrap_or_default();
            let device_id =
                std::env::var("KURA_DEVICE_ID").unwrap_or_else(|_| "KURA_DESKTOP_001".into());
            let handle = ws::connect(app.handle().clone(), ws_url, http_base, api_key, device_id);
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
            read_dropped
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
