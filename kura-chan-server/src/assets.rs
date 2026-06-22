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


// ===== Pre-composited character art (RGB565 + alpha8) =====
// The device used to fetch each layer and composite locally onto an opaque
// "baked" body — which couldn't move independently of the background (the
// breathing bob dragged the baked-in background → tearing). Now the server
// composites all body layers at native resolution, scales ONCE, and emits
// RGB565+A8 so the device renders a single transparent sprite over a fixed
// background. Compositing before scaling avoids per-layer sub-pixel drift.

#[derive(Deserialize)]
pub struct CompositeQ {
    pub hair_back: Option<String>,
    pub hair_front: Option<String>,
    pub body: Option<String>,
    pub costume: Option<String>,
    pub blush: Option<String>,
    pub glasses: Option<u8>,
    pub h: Option<u32>,
}

/// Resolve a stem to an actual source file (png/webp/jpg master), if present.
fn resolve_src(gender: &str, stem: &str) -> Option<String> {
    for ext in [".png", ".webp", ".jpg", ".jpeg", ".PNG", ".WEBP", ".JPG"] {
        let p = format!("{ASSET_DIR}/{gender}/{stem}{ext}");
        if std::path::Path::new(&p).exists() {
            return Some(p);
        }
    }
    None
}

/// RGB565 (big-endian) + A8 interleaved, 8-byte header: "KRA1" w:u16 h:u16 (BE).
fn encode_rgb565a8(img: &image::RgbaImage) -> Vec<u8> {
    let (w, h) = img.dimensions();
    let mut out = Vec::with_capacity(8 + (w * h * 3) as usize);
    out.extend_from_slice(b"KRA1");
    out.extend_from_slice(&(w as u16).to_be_bytes());
    out.extend_from_slice(&(h as u16).to_be_bytes());
    for px in img.pixels() {
        let [r, g, b, a] = px.0;
        let c: u16 = (((r as u16) & 0xF8) << 8) | (((g as u16) & 0xFC) << 3) | ((b as u16) >> 3);
        out.extend_from_slice(&c.to_be_bytes());
        out.push(a);
    }
    out
}

/// Composite the given layer stems at native resolution (alpha overlay in
/// order), scale once to height `h`, encode RGB565+A8. Missing optional layers
/// are skipped.
fn composite_and_encode(gender: &str, stems: &[String], h: u32) -> Result<Vec<u8>, String> {
    use image::imageops;
    let mut base: Option<image::RgbaImage> = None;
    for stem in stems {
        let src = match resolve_src(gender, stem) {
            Some(p) => p,
            None => continue,
        };
        let layer = image::open(&src).map_err(|e| e.to_string())?.to_rgba8();
        match base.as_mut() {
            None => base = Some(layer),
            Some(b) => imageops::overlay(b, &layer, 0, 0),
        }
    }
    let mut img = base.ok_or_else(|| "no layers resolved".to_string())?;
    if h > 0 && img.height() != h {
        let (w0, h0) = img.dimensions();
        let nw = ((w0 as f32) * (h as f32 / h0 as f32)).round().max(1.0) as u32;
        img = imageops::resize(&img, nw, h, imageops::FilterType::Lanczos3);
    }
    Ok(encode_rgb565a8(&img))
}

/// GET /assets/composite/{gender}?hair_back=&hair_front=&body=&costume=&blush=&glasses=1&h=240
/// Returns the composited character sprite (body layers, no face) as RGB565+A8.
pub async fn get_composite(
    Path(gender): Path<String>,
    Query(q): Query<CompositeQ>,
) -> impl IntoResponse {
    if !safe_seg(&gender) {
        return (StatusCode::BAD_REQUEST, "bad gender").into_response();
    }
    let body = q.body.clone().unwrap_or_else(|| "base".into());
    let mut stems: Vec<String> = Vec::new();
    if let Some(v) = q.hair_back.as_deref().filter(|s| !s.is_empty()) {
        stems.push(format!("10_hair_back_{v}"));
    }
    stems.push(format!("20_body_{body}"));
    if let Some(v) = q.blush.as_deref().filter(|s| !s.is_empty()) {
        stems.push(format!("30_blush_{v}"));
    }
    if let Some(v) = q.costume.as_deref().filter(|s| !s.is_empty()) {
        stems.push(format!("40_costume_{v}"));
    }
    if let Some(v) = q.hair_front.as_deref().filter(|s| !s.is_empty()) {
        stems.push(format!("50_hair_front_{v}"));
    }
    if q.glasses.unwrap_or(0) != 0 {
        stems.push("70_accessory_glasses".to_string());
    }
    for s in &stems {
        if !safe_seg(s) {
            return (StatusCode::BAD_REQUEST, "bad layer").into_response();
        }
    }
    let h = q.h.unwrap_or(0).min(2000);
    let gender2 = gender.clone();
    match tokio::task::spawn_blocking(move || composite_and_encode(&gender2, &stems, h)).await {
        Ok(Ok(bytes)) => (
            [(header::CONTENT_TYPE, "application/octet-stream")],
            bytes,
        )
            .into_response(),
        Ok(Err(e)) => (StatusCode::NOT_FOUND, e).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// GET /assets/face/{gender}/{expr}?h=240 -> single face layer as RGB565+A8
/// (same canvas size as the character so the device overlays it 1:1).
pub async fn get_face(
    Path((gender, expr)): Path<(String, String)>,
    Query(q): Query<SizeQ>,
) -> impl IntoResponse {
    if !safe_seg(&gender) || !safe_seg(&expr) {
        return (StatusCode::BAD_REQUEST, "bad path").into_response();
    }
    let stems = vec![format!("60_face_{expr}")];
    let h = q.h.unwrap_or(0).min(2000);
    let gender2 = gender.clone();
    match tokio::task::spawn_blocking(move || composite_and_encode(&gender2, &stems, h)).await {
        Ok(Ok(bytes)) => (
            [(header::CONTENT_TYPE, "application/octet-stream")],
            bytes,
        )
            .into_response(),
        Ok(Err(e)) => (StatusCode::NOT_FOUND, e).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}
