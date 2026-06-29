//! WebSocket client to the kura-chan server. Connects (with auto-reconnect),
//! sends Hello + outgoing JSON (TextInput), and forwards server messages to the
//! frontend as Tauri events. Audio frames are received but not yet played (TODO:
//! voice stage). The same WS protocol as the firmware is reused.

use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tauri::{AppHandle, Emitter};
use tokio::sync::{mpsc, Mutex};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

/// An outgoing WS message: JSON text (hello / text input) or a binary audio frame.
pub enum WsOut {
    Text(String),
    Binary(Vec<u8>),
}

/// Managed Tauri state: outgoing-message sender, the server's HTTP base (for the
/// frontend to fetch portrait PNGs), and the current connection status (so the
/// frontend can query it on startup — events emitted before it subscribes are lost).
pub struct WsHandle {
    pub tx: mpsc::UnboundedSender<WsOut>,
    pub http_base: String,
    pub status: Arc<StdMutex<String>>,
}

fn set_status(app: &AppHandle, status: &Arc<StdMutex<String>>, value: &str) {
    if let Ok(mut s) = status.lock() {
        *s = value.to_string();
    }
    let _ = app.emit("ws-status", value);
}

/// Spawn the connect/reconnect loop and return the handle to send messages.
pub fn connect(
    app: AppHandle,
    ws_url: String,
    http_base: String,
    api_key: String,
    device_id: String,
) -> WsHandle {
    let (tx, rx) = mpsc::unbounded_channel::<WsOut>();
    let rx = Arc::new(Mutex::new(rx));
    let status = Arc::new(StdMutex::new("connecting".to_string()));
    let status2 = status.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            set_status(&app, &status2, "connecting");
            let mut guard = rx.lock().await;
            match run_conn(&app, &ws_url, &api_key, &device_id, &mut guard, &status2).await {
                Ok(()) => set_status(&app, &status2, "closed"),
                Err(e) => set_status(&app, &status2, &format!("disconnected: {e}")),
            }
            drop(guard);
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    });
    WsHandle { tx, http_base, status }
}

async fn run_conn(
    app: &AppHandle,
    ws_url: &str,
    api_key: &str,
    device_id: &str,
    rx: &mut mpsc::UnboundedReceiver<WsOut>,
    status: &Arc<StdMutex<String>>,
) -> Result<(), String> {
    let mut req = ws_url.into_client_request().map_err(|e| e.to_string())?;
    {
        let h = req.headers_mut();
        h.insert(
            "Authorization",
            format!("Bearer {api_key}").parse().map_err(|_| "bad auth header".to_string())?,
        );
        h.insert(
            "X-Device-Id",
            device_id.parse().map_err(|_| "bad device id".to_string())?,
        );
    }
    let (ws, _) = tokio_tungstenite::connect_async(req)
        .await
        .map_err(|e| e.to_string())?;
    let (mut write, mut read) = ws.split();
    set_status(app, status, "connected");

    // Announce ourselves (same Hello shape the firmware sends).
    let hello = json!({
        "type": "hello",
        "device_id": device_id,
        "firmware_version": "desktop-0.1",
        "audio": {
            "input_format": "pcm16",
            "input_sample_rate": 16000,
            "input_channels": 1,
            "input_frame_duration_ms": 20,
            "output_format": "pcm16",
            "output_sample_rate": 16000,
            "output_channels": 1
        },
        "capabilities": ["text"]
    });
    write
        .send(Message::Text(hello.to_string().into()))
        .await
        .map_err(|e| e.to_string())?;

    loop {
        tokio::select! {
            incoming = read.next() => match incoming {
                Some(Ok(Message::Text(t))) => handle_server_text(app, t.as_str()),
                Some(Ok(Message::Binary(b))) => handle_audio_frame(app, &b),
                Some(Ok(Message::Close(_))) | None => break,
                Some(Ok(_)) => {}
                Some(Err(e)) => return Err(e.to_string()),
            },
            outgoing = rx.recv() => match outgoing {
                Some(WsOut::Text(t)) => write
                    .send(Message::Text(t.into()))
                    .await
                    .map_err(|e| e.to_string())?,
                Some(WsOut::Binary(b)) => write
                    .send(Message::Binary(b.into()))
                    .await
                    .map_err(|e| e.to_string())?,
                None => break,
            }
        }
    }
    Ok(())
}

/// Parse a server JSON message and re-emit it to the frontend by `type`.
fn handle_server_text(app: &AppHandle, text: &str) {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else {
        return;
    };
    let typ = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
    match typ {
        "subtitle" => { let _ = app.emit("subtitle", &v); }
        "stt" => { let _ = app.emit("stt", &v); }
        "sync" => { let _ = app.emit("sync", &v); }
        "response" => { let _ = app.emit("response", &v); }
        "state" => { let _ = app.emit("state", &v); }
        "control" => { let _ = app.emit("control", &v); }
        "speak_done" => { let _ = app.emit("speak_done", ()); }
        _ => {}
    }
}

/// Decode an AUDIO_OUTPUT frame ([0x02, flags, len:u16, PCM16 payload]) and
/// forward the PCM to the frontend (base64) for Web Audio playback. The START
/// flag tells the frontend to reset its playback schedule (new reply).
fn handle_audio_frame(app: &AppHandle, data: &[u8]) {
    if data.len() < 4 || data[0] != 0x02 {
        return; // not an AUDIO_OUTPUT frame
    }
    let flags = data[1];
    if flags & 0x01 != 0 {
        let _ = app.emit("audio-start", ());
    }
    let payload = &data[4..];
    if payload.is_empty() {
        return;
    }
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(payload);
    let _ = app.emit("audio", b64);
}
