// Kapak indirme, önbellek ve kapaktan vurgu rengi.

use std::path::PathBuf;

pub struct Art {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Okunabilirliği ayarlanmış vurgu rengi (r, g, b).
    pub accent: (u8, u8, u8),
    /// Söz ekranı zemini için önceden bulanıklaştırılmış küçük kapak (BLUR x BLUR).
    pub blur: Vec<u8>,
}

pub const BLUR: u32 = 48;

fn cache_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".cache"));
    base.join("vynz").join("covers")
}

/// Liste ve raflardaki kapak boyutu; çalan şarkı için BIG.
pub const SMALL: u32 = 192;
pub const BIG: u32 = 320;

/// `key` önbellek dosya adı, `size` çözülecek kare boyutu.
pub fn load(key: &str, url: &str, size: u32) -> Result<Art, String> {
    if url.is_empty() {
        return Err("kapak yok".into());
    }
    let dir = cache_dir();
    let path = dir.join(format!("{key}.img"));
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(_) => {
            let mut resp = ureq::get(url).call().map_err(|e| e.to_string())?;
            let b = resp
                .body_mut()
                .with_config()
                .limit(8 * 1024 * 1024)
                .read_to_vec()
                .map_err(|e| e.to_string())?;
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::write(&path, &b);
            b
        }
    };
    let img = image::load_from_memory(&bytes).map_err(|e| e.to_string())?.to_rgba8();
    // YouTube video küçük resimlerindeki siyah bantları (letterbox) kes.
    let (x, y, w, h) = content_box(&img);
    let img = image::DynamicImage::ImageRgba8(image::imageops::crop_imm(&img, x, y, w, h).to_image());
    let img = img.resize_to_fill(size, size, image::imageops::FilterType::Triangle).to_rgba8();
    let accent = accent_of(&img);
    // Ucuz bulanıklık: küçült, bulanıklaştır; arayüz büyütünce yumuşak görünür.
    let small = image::imageops::resize(&img, BLUR, BLUR, image::imageops::FilterType::Triangle);
    let blur = image::imageops::blur(&small, 6.0).into_raw();
    let (width, height) = img.dimensions();
    Ok(Art { rgba: img.into_raw(), width, height, accent, blur })
}

/// Kenarlardaki neredeyse siyah satır/sütunları atlayan içerik kutusu.
fn content_box(img: &image::RgbaImage) -> (u32, u32, u32, u32) {
    let (w, h) = img.dimensions();
    let dark_row = |y: u32| (0..w).step_by(4).all(|x| {
        let p = img.get_pixel(x, y);
        (p[0] as u32 + p[1] as u32 + p[2] as u32) < 60
    });
    let dark_col = |x: u32, y0: u32, y1: u32| (y0..y1).step_by(4).all(|y| {
        let p = img.get_pixel(x, y);
        (p[0] as u32 + p[1] as u32 + p[2] as u32) < 60
    });
    let mut top = 0;
    while top < h / 3 && dark_row(top) {
        top += 1;
    }
    let mut bottom = h;
    while bottom > h * 2 / 3 && dark_row(bottom - 1) {
        bottom -= 1;
    }
    let mut left = 0;
    while left < w / 3 && dark_col(left, top, bottom) {
        left += 1;
    }
    let mut right = w;
    while right > w * 2 / 3 && dark_col(right - 1, top, bottom) {
        right -= 1;
    }
    // Tamamen koyu kapakları bozmamak için çok küçük sonuçta orijinali kullan.
    if right - left < w / 3 || bottom - top < h / 3 {
        return (0, 0, w, h);
    }
    (left, top, right - left, bottom - top)
}

/// Doygun piksellere ağırlık veren ortalama, sonra koyu zeminde okunur aralığa çekilir.
fn accent_of(img: &image::RgbaImage) -> (u8, u8, u8) {
    let (mut r, mut g, mut b, mut w) = (0f64, 0f64, 0f64, 0f64);
    for p in img.pixels().step_by(7) {
        let (pr, pg, pb) = (p[0] as f64 / 255.0, p[1] as f64 / 255.0, p[2] as f64 / 255.0);
        let max = pr.max(pg).max(pb);
        let min = pr.min(pg).min(pb);
        let sat = if max > 0.0 { (max - min) / max } else { 0.0 };
        let weight = 0.05 + sat * sat * max; // gri ve karanlık pikseller az sayılır
        r += pr * weight;
        g += pg * weight;
        b += pb * weight;
        w += weight;
    }
    let (h, s, _) = rgb_to_hsl(r / w, g / w, b / w);
    // Koyu tema: açıklık 0.68, doygunluk en az 0.45 (gri kapakta nötr kalsın)
    let s = if s < 0.12 { s } else { s.max(0.45).min(0.85) };
    let (r, g, b) = hsl_to_rgb(h, s, 0.68);
    ((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
}

fn rgb_to_hsl(r: f64, g: f64, b: f64) -> (f64, f64, f64) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    if max == min {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
    let h = if max == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h / 6.0, s, l)
}

fn hsl_to_rgb(h: f64, s: f64, l: f64) -> (f64, f64, f64) {
    if s == 0.0 {
        return (l, l, l);
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let f = |mut t: f64| {
        if t < 0.0 {
            t += 1.0;
        }
        if t > 1.0 {
            t -= 1.0;
        }
        if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        }
    };
    (f(h + 1.0 / 3.0), f(h), f(h - 1.0 / 3.0))
}
