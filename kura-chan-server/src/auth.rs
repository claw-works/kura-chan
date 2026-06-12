use axum::http::HeaderMap;

use crate::config::AuthConfig;

pub fn validate_api_key(headers: &HeaderMap, auth_config: &AuthConfig) -> Result<String, String> {
    let auth_header = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| "Missing Authorization header".to_string())?;

    let key = auth_header
        .strip_prefix("Bearer ")
        .ok_or_else(|| "Authorization header must use Bearer scheme".to_string())?;

    if auth_config.api_keys.contains(&key.to_string()) {
        let device_id = headers
            .get("x-device-id")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("unknown")
            .to_string();
        Ok(device_id)
    } else {
        Err("Invalid API key".to_string())
    }
}
