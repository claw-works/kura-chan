mod ws;

use serde_json::json;
use tauri::{Manager, State};
use ws::WsHandle;

/// Send a typed message to the agent (text input → server, bypasses STT).
#[tauri::command]
fn send_text(state: State<WsHandle>, text: String) -> Result<(), String> {
    let t = text.trim();
    if t.is_empty() {
        return Ok(());
    }
    let msg = json!({ "type": "text_input", "text": t });
    state.tx.send(msg.to_string()).map_err(|e| e.to_string())
}

/// Frontend config (server HTTP base for fetching portrait PNGs, etc.).
#[tauri::command]
fn get_config(state: State<WsHandle>) -> serde_json::Value {
    json!({ "httpBase": state.http_base })
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
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![send_text, get_config])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
