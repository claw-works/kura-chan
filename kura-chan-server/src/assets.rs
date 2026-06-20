use axum::extract::{Path, Query};
use axum::http::{header, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;

// Character art lives on the server under assets/<gender>/NN_category_variant.png
// (gender = girl / boy). The device fetches the layers it needs, optionally
// scaled to its screen via ?h=. Scaled results are cached on disk.
const ASSET_DIR: &str = "assets";

fn list_pngish(dir: &str) -> Vec<String> {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().to_string())
                .filter(|n| {
                    let l = n.to_ascii_lowercase();
                    l.ends_with(".png") || l.ends_with(".webp") || l.ends_with(".jpg") || l.ends_with(".jpeg")
                })
                .collect()
        })
        .unwrap_or_default()
}

fn stem(name: &str) -> String {
    let n = name;
    for ext in [".png", ".webp", ".jpg", ".jpeg", ".PNG", ".WEBP", ".JPG"] {
        if let Some(s) = n.strip_suffix(ext) {
            return s.to_string();
        }
    }
    n.to_string()
}

/// Scan the asset folders and produce a prompt section listing the variants the
/// agent may use right now, so adding files needs no prompt edits.
pub fn options_prompt(gender: &str) -> String {
    use std::collections::BTreeSet;
    let mut hair = BTreeSet::new();
    let mut costume = BTreeSet::new();
    let mut blush = BTreeSet::new();
    for f in list_pngish(&format!("{ASSET_DIR}/{gender}")) {
        let s = stem(&f);
        if let Some(v) = s.strip_prefix("10_hair_back_").or_else(|| s.strip_prefix("50_hair_front_")) {
            hair.insert(v.to_string());
        } else if let Some(v) = s.find("costume_").map(|i| s[i + 8..].to_string()) {
            costume.insert(v);
        } else if let Some(v) = s.find("blush_").map(|i| s[i + 6..].to_string()) {
            blush.insert(v);
        }
    }
    let scenes: BTreeSet<String> = list_pngish(&format!("{ASSET_DIR}/bg"))
        .iter()
        .map(|f| stem(f))
        .collect();

    let join = |s: &BTreeSet<String>| s.iter().cloned().collect::<Vec<_>>().join("、");
    format!(
        "【当前可用项】(只能从下列里选, 没有的不要编造)\n\
         - 发型 [do:wear=X]: {}\n\
         - 服装 [do:wear=X]: {}\n\
         - 脸红 [do:blush=on|off]{}\n\
         - 眼镜 [do:glasses=on|off]\n\
         - 场景 [do:bg=X](取消=none): {}",
        join(&hair),
        join(&costume),
        if blush.is_empty() { String::new() } else { format!(" (档位: {})", join(&blush)) },
        join(&scenes),
    )
}

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
    let names: std::collections::BTreeSet<String> = match std::fs::read_dir(&dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let n = e.file_name().to_string_lossy().to_string();
                let low = n.to_ascii_lowercase();
                for ext in [".png", ".webp", ".jpg", ".jpeg"] {
                    if low.ends_with(ext) {
                        return Some(format!("{}.png", &n[..n.len() - ext.len()]));
                    }
                }
                None
            })
            .collect(),
        Err(_) => return (StatusCode::NOT_FOUND, "no such gender").into_response(),
    };
    let names: Vec<String> = names.into_iter().collect();
    Json(names).into_response()
}

/// GET /assets/{gender}/{file}[?h=240] -> PNG (optionally scaled to height h).
/// The source may be PNG/JPEG/WEBP (matched by stem); output is always PNG so the
/// device only needs a PNG decoder. Results are disk-cached.
pub async fn get_asset(
    Path((gender, file)): Path<(String, String)>,
    Query(q): Query<SizeQ>,
) -> impl IntoResponse {
    if !safe_seg(&gender) || !safe_seg(&file) || !file.to_ascii_lowercase().ends_with(".png") {
        return (StatusCode::BAD_REQUEST, "bad path").into_response();
    }
    let stem = file.trim_end_matches(".png").trim_end_matches(".PNG").to_string();
    // resolve the actual source file (allow webp/jpg masters)
    let mut src = String::new();
    for ext in [".png", ".webp", ".jpg", ".jpeg", ".PNG", ".WEBP", ".JPG"] {
        let p = format!("{ASSET_DIR}/{gender}/{stem}{ext}");
        if std::path::Path::new(&p).exists() { src = p; break; }
    }
    if src.is_empty() {
        return (StatusCode::NOT_FOUND, "no file").into_response();
    }
    let h = q.h.unwrap_or(0).min(2000);
    // serve a directly-readable PNG at native size without re-encoding
    if h == 0 && src.to_ascii_lowercase().ends_with(".png") {
        match tokio::fs::read(&src).await {
            Ok(b) => return ([(header::CONTENT_TYPE, "image/png")], b).into_response(),
            Err(_) => return (StatusCode::NOT_FOUND, "read fail").into_response(),
        }
    }
    let cache = format!("{ASSET_DIR}/.cache/{gender}/h{h}/{stem}.png");
    let data = if let Ok(b) = tokio::fs::read(&cache).await {
        b
    } else {
        match render_png(src.clone(), h).await {
            Ok(b) => {
                if let Some(parent) = std::path::Path::new(&cache).parent() {
                    let _ = tokio::fs::create_dir_all(parent).await;
                }
                let _ = tokio::fs::write(&cache, &b).await;
                b
            }
            Err(e) => return (StatusCode::NOT_FOUND, e).into_response(),
        }
    };
    ([(header::CONTENT_TYPE, "image/png")], data).into_response()
}

/// Decode any supported format, optionally resize to height `h`, encode PNG.
async fn render_png(src: String, h: u32) -> Result<Vec<u8>, String> {
    tokio::task::spawn_blocking(move || {
        let img = image::open(&src).map_err(|e| e.to_string())?.to_rgba8();
        let (w0, h0) = img.dimensions();
        if h0 == 0 {
            return Err("bad image".into());
        }
        let out_img = if h > 0 {
            let nw = ((w0 as f32) * (h as f32 / h0 as f32)).round().max(1.0) as u32;
            image::imageops::resize(&img, nw, h, image::imageops::FilterType::Lanczos3)
        } else {
            img
        };
        let mut out = Vec::new();
        image::DynamicImage::ImageRgba8(out_img)
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .map_err(|e| e.to_string())?;
        Ok(out)
    })
    .await
    .map_err(|e| e.to_string())?
}
