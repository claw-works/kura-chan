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
    /// Most recent `sync` message JSON, so the frontend can query it on startup
    /// (the connect-time sync may arrive before the frontend's listener is ready).
    pub last_sync: Arc<StdMutex<Option<String>>>,
}

fn set_status(app: &AppHandle, status: &Arc<StdMutex<String>>, value: &str) {
    eprintln!("[ws] {value}");
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
    let last_sync = Arc::new(StdMutex::new(None));
    let last_sync2 = last_sync.clone();
    let tx_tool = tx.clone(); // for sending ToolResults from tool execution
    tauri::async_runtime::spawn(async move {
        loop {
            set_status(&app, &status2, "connecting");
            let mut guard = rx.lock().await;
            match run_conn(&app, &ws_url, &api_key, &device_id, &mut guard, &status2, &tx_tool, &last_sync2).await {
                Ok(()) => set_status(&app, &status2, "closed"),
                Err(e) => set_status(&app, &status2, &format!("disconnected: {e}")),
            }
            drop(guard);
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    });
    WsHandle { tx, http_base, status, last_sync }
}

async fn run_conn(
    app: &AppHandle,
    ws_url: &str,
    api_key: &str,
    device_id: &str,
    rx: &mut mpsc::UnboundedReceiver<WsOut>,
    status: &Arc<StdMutex<String>>,
    tx: &mpsc::UnboundedSender<WsOut>,
    last_sync: &Arc<StdMutex<Option<String>>>,
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
        "capabilities": ["text", "notify", "read_file", "list_dir", "search_files", "get_clipboard", "set_clipboard", "system_info", "write_file", "open_url", "open_path", "run_command"]
    });
    write
        .send(Message::Text(hello.to_string().into()))
        .await
        .map_err(|e| e.to_string())?;

    loop {
        tokio::select! {
            incoming = read.next() => match incoming {
                Some(Ok(Message::Text(t))) => {
                    let s = t.as_str();
                    eprintln!("[ws←] {}", s.chars().take(400).collect::<String>());
                    if let Some(call) = parse_tool_call(s) {
                        eprintln!("[tool] got tool_call: tool={} call_id={} params={}", call.tool, call.call_id, call.params);
                        // execute device tool off-thread, send ToolResult back via tx
                        let tx2 = tx.clone();
                        tauri::async_runtime::spawn(execute_tool(tx2, call));
                    } else {
                        handle_server_text(app, s, last_sync);
                    }
                }
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
fn handle_server_text(app: &AppHandle, text: &str, last_sync: &Arc<StdMutex<Option<String>>>) {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else {
        return;
    };
    let typ = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
    match typ {
        "subtitle" => { let _ = app.emit("subtitle", &v); }
        "stt" => { let _ = app.emit("stt", &v); }
        "sync" => {
            // cache so the frontend can fetch it on startup if it missed the event
            if let Ok(mut ls) = last_sync.lock() {
                *ls = Some(text.to_string());
            }
            eprintln!("[sync] {}", &text[..text.len().min(400)]);
            let _ = app.emit("sync", &v);
        }
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


// ===== device tools (executed locally, results sent back as ToolResult) =====

struct DeviceToolCall {
    call_id: String,
    tool: String,
    params: serde_json::Value,
}

/// Parse a `tool_call` server message; returns None for any other message type.
fn parse_tool_call(s: &str) -> Option<DeviceToolCall> {
    let v: serde_json::Value = serde_json::from_str(s).ok()?;
    if v.get("type")?.as_str()? != "tool_call" {
        return None;
    }
    Some(DeviceToolCall {
        call_id: v.get("call_id")?.as_str()?.to_string(),
        tool: v.get("tool")?.as_str()?.to_string(),
        params: v.get("params").cloned().unwrap_or_else(|| json!({})),
    })
}

/// Run a device tool and send the ToolResult back to the server.
async fn execute_tool(tx: mpsc::UnboundedSender<WsOut>, call: DeviceToolCall) {
    let (status, result) = run_tool(&call.tool, &call.params).await;
    eprintln!(
        "[tool] {} → status={} result={}",
        call.tool,
        status,
        result.to_string().chars().take(200).collect::<String>()
    );
    let msg = json!({
        "type": "tool_result",
        "call_id": call.call_id,
        "status": status,
        "result": result,
    });
    match tx.send(WsOut::Text(msg.to_string())) {
        Ok(()) => eprintln!("[tool→] sent tool_result call_id={}", call.call_id),
        Err(e) => eprintln!("[tool→] FAILED to send tool_result: {e}"),
    }
}

/// Expand a leading `~` / `~/` to the user's home directory.
fn expand_path(p: &str) -> String {
    if p == "~" {
        if let Some(home) = std::env::var_os("HOME") {
            return home.to_string_lossy().to_string();
        }
    } else if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return format!("{}/{}", home.to_string_lossy(), rest);
        }
    }
    p.to_string()
}

async fn run_tool(tool: &str, params: &serde_json::Value) -> (&'static str, serde_json::Value) {
    let p = |k: &str| params.get(k).and_then(|v| v.as_str()).unwrap_or("");
    match tool {
        "notify" => {
            let title = if p("title").is_empty() { "小爪" } else { p("title") };
            notify_macos(title, p("body"));
            ("ok", json!({ "shown": true }))
        }
        "read_file" => {
            // NOTE: reads an arbitrary path the agent chose. Trusted for now
            // (user's own machine); add a path allowlist / confirmation later.
            let path = expand_path(p("path"));
            match tokio::fs::read_to_string(&path).await {
                Ok(c) => ("ok", json!({ "content": c.chars().take(4000).collect::<String>() })),
                Err(e) => ("error", json!(e.to_string())),
            }
        }
        "list_dir" => {
            let path = expand_path(p("path"));
            let filter = p("filter").to_lowercase();
            match tokio::fs::read_dir(&path).await {
                Ok(mut rd) => {
                    let mut entries: Vec<String> = Vec::new();
                    while let Ok(Some(e)) = rd.next_entry().await {
                        let name = e.file_name().to_string_lossy().to_string();
                        if filter.is_empty() || name.to_lowercase().contains(&filter) {
                            entries.push(name);
                        }
                    }
                    entries.sort();
                    entries.truncate(200);
                    ("ok", json!({ "path": path, "count": entries.len(), "entries": entries }))
                }
                Err(e) => ("error", json!(e.to_string())),
            }
        }
        "search_files" => {
            let dir = expand_path(if p("dir").is_empty() { "~" } else { p("dir") });
            let pattern = p("pattern");
            match tokio::process::Command::new("find")
                .arg(&dir)
                .args(["-maxdepth", "4", "-iname", &format!("*{pattern}*")])
                .output()
                .await
            {
                Ok(o) => {
                    let files: Vec<String> =
                        String::from_utf8_lossy(&o.stdout).lines().take(100).map(String::from).collect();
                    ("ok", json!({ "count": files.len(), "files": files }))
                }
                Err(e) => ("error", json!(e.to_string())),
            }
        }
        "get_clipboard" => match tokio::process::Command::new("pbpaste").output().await {
            Ok(o) => ("ok", json!({ "text": String::from_utf8_lossy(&o.stdout).chars().take(4000).collect::<String>() })),
            Err(e) => ("error", json!(e.to_string())),
        },
        "set_clipboard" => match set_clipboard(p("text")).await {
            Ok(()) => ("ok", json!({ "set": true })),
            Err(e) => ("error", json!(e)),
        },
        "system_info" => {
            let battery = tokio::process::Command::new("pmset")
                .args(["-g", "batt"])
                .output()
                .await
                .ok()
                .map(|o| {
                    String::from_utf8_lossy(&o.stdout).lines().nth(1).unwrap_or("").trim().to_string()
                });
            (
                "ok",
                json!({
                    "os": std::env::consts::OS,
                    "user": std::env::var("USER").unwrap_or_default(),
                    "home": std::env::var("HOME").unwrap_or_default(),
                    "battery": battery,
                }),
            )
        }
        "write_file" => {
            let path = expand_path(p("path"));
            match tokio::fs::write(&path, p("content")).await {
                Ok(()) => ("ok", json!({ "written": true, "path": path })),
                Err(e) => ("error", json!(e.to_string())),
            }
        }
        "open_url" => match tokio::process::Command::new("open").arg(p("url")).output().await {
            Ok(_) => ("ok", json!({ "opened": true })),
            Err(e) => ("error", json!(e.to_string())),
        },
        "open_path" => {
            let path = expand_path(p("path"));
            match tokio::process::Command::new("open").arg(&path).output().await {
                Ok(_) => ("ok", json!({ "opened": true })),
                Err(e) => ("error", json!(e.to_string())),
            }
        }
        "run_command" => {
            let cmd = p("command").to_string();
            if !osascript_confirm(&format!("小爪想执行命令：\n\n{cmd}\n\n允许吗？")).await {
                return ("error", json!("用户拒绝执行该命令"));
            }
            match tokio::process::Command::new("sh").arg("-c").arg(&cmd).output().await {
                Ok(o) => (
                    "ok",
                    json!({
                        "stdout": String::from_utf8_lossy(&o.stdout).chars().take(4000).collect::<String>(),
                        "stderr": String::from_utf8_lossy(&o.stderr).chars().take(1000).collect::<String>(),
                        "code": o.status.code(),
                    }),
                ),
                Err(e) => ("error", json!(e.to_string())),
            }
        }
        _ => ("error", json!(format!("unknown tool '{tool}'"))),
    }
}

/// Write text to the macOS clipboard via pbcopy.
async fn set_clipboard(text: &str) -> Result<(), String> {
    use tokio::io::AsyncWriteExt;
    let mut child = tokio::process::Command::new("pbcopy")
        .stdin(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    if let Some(mut si) = child.stdin.take() {
        si.write_all(text.as_bytes()).await.map_err(|e| e.to_string())?;
        let _ = si.shutdown().await;
    }
    child.wait().await.map_err(|e| e.to_string())?;
    Ok(())
}

/// Ask the user to confirm a risky action via a native macOS dialog.
async fn osascript_confirm(prompt: &str) -> bool {
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!(
        "display dialog \"{}\" buttons {{\"拒绝\", \"允许\"}} default button \"拒绝\" with title \"小爪\"",
        esc(prompt)
    );
    match tokio::process::Command::new("osascript").arg("-e").arg(script).output().await {
        Ok(o) => String::from_utf8_lossy(&o.stdout).contains("允许"),
        Err(_) => false,
    }
}

/// Show a macOS notification via osascript (simple, no extra deps).
fn notify_macos(title: &str, body: &str) {
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let script = format!(
        "display notification \"{}\" with title \"{}\"",
        esc(body),
        esc(title)
    );
    let _ = std::process::Command::new("osascript").arg("-e").arg(script).output();
}
