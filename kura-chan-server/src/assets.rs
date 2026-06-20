use axum::extract::{Path, Query};
use axum::http::{header, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;

// Character art lives on the server under assets/<gender>/NN_category_variant.png
// (gender = girl / boy). The device fetches the layers it needs, optionally
// scaled to its screen via ?h=. Scaled results are cached on disk.
const ASSET_DIR: &str = "assets";

fn safe_seg(s: &str) -> bool {
    !s.is_empty()
        && s.len() < 64
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.' || b == b'-')
        && !s.contains("..")
}

#[derive(Deserialize)]
pub struct SizeQ {
    pub h: Option<u32>,
    pub w: Option<u32>,
}

/// GET /assets/{gender} -> ["00_bg_blank.png", ...]
pub async fn list_assets(Path(gender): Path<String>) -> impl IntoResponse {
    if !safe_seg(&gender) {
        return (StatusCode::BAD_REQUEST, "bad gender").into_response();
    }
    let dir = format!("{ASSET_DIR}/{gender}");
    let mut names: Vec<String> = match std::fs::read_dir(&dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.to_ascii_lowercase().ends_with(".png"))
            .collect(),
        Err(_) => return (StatusCode::NOT_FOUND, "no such gender").into_response(),
    };
    names.sort();
    Json(names).into_response()
}

/// GET /assets/{gender}/{file}[?h=240] -> PNG (optionally scaled to height h)
pub async fn get_asset(
    Path((gender, file)): Path<(String, String)>,
    Query(q): Query<SizeQ>,
) -> impl IntoResponse {
    if !safe_seg(&gender) || !safe_seg(&file) || !file.to_ascii_lowercase().ends_with(".png") {
        return (StatusCode::BAD_REQUEST, "bad path").into_response();
    }
    let src = format!("{ASSET_DIR}/{gender}/{file}");
    let data = match q.h {
        Some(h) if h > 0 && h <= 2000 => {
            let cache = format!("{ASSET_DIR}/.cache/{gender}/h{h}/{file}");
            if let Ok(b) = tokio::fs::read(&cache).await {
                b
            } else {
                match scale_png(src.clone(), h).await {
                    Ok(b) => {
                        if let Some(parent) = std::path::Path::new(&cache).parent() {
                            let _ = tokio::fs::create_dir_all(parent).await;
                        }
                        let _ = tokio::fs::write(&cache, &b).await;
                        b
                    }
                    Err(e) => return (StatusCode::NOT_FOUND, e).into_response(),
                }
            }
        }
        _ => match tokio::fs::read(&src).await {
            Ok(b) => b,
            Err(_) => return (StatusCode::NOT_FOUND, "no file").into_response(),
        },
    };
    ([(header::CONTENT_TYPE, "image/png")], data).into_response()
}

/// Resize a PNG to height `h` (preserving aspect + alpha), Lanczos3.
async fn scale_png(src: String, h: u32) -> Result<Vec<u8>, String> {
    tokio::task::spawn_blocking(move || {
        let img = image::open(&src).map_err(|e| e.to_string())?.to_rgba8();
        let (w0, h0) = img.dimensions();
        if h0 == 0 {
            return Err("bad image".into());
        }
        let nw = ((w0 as f32) * (h as f32 / h0 as f32)).round().max(1.0) as u32;
        let resized =
            image::imageops::resize(&img, nw, h, image::imageops::FilterType::Lanczos3);
        let mut out = Vec::new();
        image::DynamicImage::ImageRgba8(resized)
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .map_err(|e| e.to_string())?;
        Ok(out)
    })
    .await
    .map_err(|e| e.to_string())?
}
