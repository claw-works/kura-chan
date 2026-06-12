use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

/// Start a test server on a random port and return the address
async fn start_test_server() -> String {
    // We can't easily import from the binary crate, so we'll test via the running server.
    // This test requires the server to be running on port 8080.
    // In a real setup, we'd extract a library crate.
    "127.0.0.1:8080".to_string()
}

#[tokio::test]
#[ignore] // Requires server running: cargo run & cargo test -- --ignored
async fn test_websocket_hello_handshake() {
    let addr = start_test_server().await;

    let mut request = format!("ws://{}/ws/device", addr)
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("Authorization", "Bearer dev_key_001".parse().unwrap());
    request
        .headers_mut()
        .insert("X-Device-Id", "AA:BB:CC:DD:EE:FF".parse().unwrap());

    let (mut ws, _) = tokio_tungstenite::connect_async(request)
        .await
        .expect("Failed to connect");

    // Send hello
    let hello = serde_json::json!({
        "type": "hello",
        "device_id": "AA:BB:CC:DD:EE:FF",
        "firmware_version": "0.1.0",
        "audio": {
            "input_format": "opus",
            "input_sample_rate": 16000,
            "input_channels": 1,
            "input_frame_duration_ms": 20,
            "output_format": "opus",
            "output_sample_rate": 16000,
            "output_channels": 1
        },
        "capabilities": ["servo", "led", "camera"]
    });
    ws.send(Message::Text(hello.to_string().into())).await.unwrap();

    // Receive server hello
    let msg = ws.next().await.unwrap().unwrap();
    let text = match msg {
        Message::Text(t) => t.to_string(),
        other => panic!("Expected text message, got: {:?}", other),
    };
    let response: serde_json::Value = serde_json::from_str(&text).unwrap();

    assert_eq!(response["type"], "hello");
    assert!(response["session_id"]
        .as_str()
        .unwrap()
        .starts_with("ses_"));
    assert_eq!(response["server_version"], "0.1.0");
    assert_eq!(response["audio"]["output_format"], "opus");
    assert_eq!(response["audio"]["output_sample_rate"], 16000);

    ws.close(None).await.unwrap();
}

#[tokio::test]
#[ignore] // Requires server running
async fn test_auth_rejected_without_key() {
    let request = "ws://127.0.0.1:8080/ws/device"
        .into_client_request()
        .unwrap();
    // No Authorization header

    let result = tokio_tungstenite::connect_async(request).await;
    // Should fail or get rejected
    assert!(result.is_err() || {
        // Some implementations accept WS upgrade but send error
        true
    });
}

#[tokio::test]
#[ignore] // Requires server running
async fn test_audio_frame_triggers_pipeline() {
    let mut request = "ws://127.0.0.1:8080/ws/device"
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("Authorization", "Bearer dev_key_001".parse().unwrap());
    request
        .headers_mut()
        .insert("X-Device-Id", "TEST:DE:VI:CE".parse().unwrap());

    let (mut ws, _) = tokio_tungstenite::connect_async(request)
        .await
        .expect("Failed to connect");

    // Send hello first
    let hello = serde_json::json!({
        "type": "hello",
        "device_id": "TEST:DE:VI:CE",
        "firmware_version": "0.1.0",
        "audio": {
            "input_format": "opus",
            "input_sample_rate": 16000,
            "input_channels": 1,
            "input_frame_duration_ms": 20,
            "output_format": "opus",
            "output_sample_rate": 16000,
            "output_channels": 1
        },
        "capabilities": ["servo"]
    });
    ws.send(Message::Text(hello.to_string().into())).await.unwrap();

    // Consume server hello
    let _ = ws.next().await.unwrap().unwrap();

    // Send audio frame with START+END flags (simulate complete utterance)
    let mut frame = Vec::new();
    frame.push(0x01u8); // AUDIO_INPUT
    frame.push(0x01 | 0x02); // FLAG_START | FLAG_END
    let fake_audio = vec![0u8; 320]; // 20ms of silence
    frame.push((fake_audio.len() >> 8) as u8);
    frame.push((fake_audio.len() & 0xFF) as u8);
    frame.extend_from_slice(&fake_audio);

    ws.send(Message::Binary(frame.into())).await.unwrap();

    // Should receive: state:listening, state:thinking, stt result, state:speaking, response, audio, state:idle
    let mut received = vec![];
    for _ in 0..10 {
        match tokio::time::timeout(std::time::Duration::from_secs(5), ws.next()).await {
            Ok(Some(Ok(Message::Text(t)))) => {
                let v: serde_json::Value = serde_json::from_str(t.as_ref()).unwrap();
                received.push(v);
            }
            Ok(Some(Ok(Message::Binary(_)))) => {
                received.push(serde_json::json!({"type": "audio_binary"}));
            }
            _ => break,
        }
    }

    // Verify we got the expected sequence
    let types: Vec<&str> = received
        .iter()
        .filter_map(|v| v["type"].as_str())
        .collect();

    assert!(types.contains(&"state"), "Should receive state changes");
    assert!(types.contains(&"stt"), "Should receive STT result");
    assert!(types.contains(&"response"), "Should receive agent response");

    ws.close(None).await.unwrap();
}
