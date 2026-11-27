// Kapak indirme, önbellek ve kapaktan vurgu rengi.

use std::path::PathBuf;

pub struct Art {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Okunabilirliği ayarlanmış vurgu rengi (r, g, b).
    pub accent: (u8, u8, u8),
    /// Söz ekranı ve arka plan için önceden bulanıklaştırılmış kapak (BLUR x BLUR, opak RGBA).
    pub blur: Vec<u8>,
    /// Sıvı katman ve hale için 180° döndürülmüş, yüksek kontrastlı, kenarları radyal eriyen kapak (BLUR x BLUR).
    pub blur2: Vec<u8>,
    /// Ortam zemini için kapaktan üç renk (yalnız `load`; diğerlerinde boş anlamında siyah)
    pub palette: [(u8, u8, u8); 3],
}

pub const BLUR: u32 = 128;

fn cache_dir() -> PathBuf {
    crate::paths::cache_dir().join("covers")
}

/// Liste ve raflardaki kapak boyutu; çalan şarkı için BIG.
pub const SMALL: u32 = 160;
pub const BIG: u32 = 320;

/// `key` önbellek dosya adı, `size` çözülecek kare boyutu.
pub fn load(key: &str, url: &str, size: u32) -> Result<Art, String> {
    if url.is_empty() {
        return Err("kapak yok".into());
    }
    let bytes = fetch(key, url)?;
    let img = image::load_from_memory(&bytes).map_err(|e| e.to_string())?.to_rgba8();
    // YouTube video küçük resimlerindeki siyah bantları (letterbox) kes.
    let (x, y, w, h) = content_box(&img);
    let img = image::DynamicImage::ImageRgba8(image::imageops::crop_imm(&img, x, y, w, h).to_image());
    let img = img.resize_to_fill(size, size, image::imageops::FilterType::Triangle).to_rgba8();
    let accent = accent_of(&img);
    // Bulanık kapaklar: 128x128 küçült, doygunluğu artır, açıklığı 0.18–0.72'ye sıkıştır, bulanıklaştır + titreşim.
    let mut small = image::imageops::resize(&img, BLUR, BLUR, image::imageops::FilterType::Triangle);
    let palette = crate::ambience::palette(&small, accent);
    for p in small.pixels_mut() {
        let (h, s, l) = rgb_to_hsl(p[0] as f64 / 255.0, p[1] as f64 / 255.0, p[2] as f64 / 255.0);
        let s = (s * 1.4).min(0.9);
        let l = 0.18 + l * (0.72 - 0.18);
        let (r, g, b) = hsl_to_rgb(h, s, l);
        p[0] = (r * 255.0).round().clamp(0.0, 255.0) as u8;
        p[1] = (g * 255.0).round().clamp(0.0, 255.0) as u8;
        p[2] = (b * 255.0).round().clamp(0.0, 255.0) as u8;
        p[3] = 255;
    }
    let blurred = image::imageops::blur(&small, 10.0);
    let mut rng = crate::ambience::Rng(0xA5A5_1357);
    let mut blur = blurred.as_raw().clone();
    for p in blur.chunks_exact_mut(4) {
        for c in 0..3 {
            let d = rng.unit() - rng.unit();
            p[c] = (p[c] as f32 + d).round().clamp(0.0, 255.0) as u8;
        }
        p[3] = 255;
    }
    // blur2: 180° döndürülmüş, aydınlatılmış, merkez 1 → kenar 0 yumuşak radyal alfa.
    // Siyah/gri ağırlıklı kapakta ortalama renk soluk kalıp zeminde kayboluyordu: soluk
    // pikseller kapağın en canlı renginin tonuna doğru boyanır (Y4).
    let vivid = halo_vivid(&palette, accent);
    let n = BLUR as usize;
    let mut blur2 = vec![0u8; n * n * 4];
    let center = (n as f32 - 1.0) / 2.0;
    for y in 0..n {
        for x in 0..n {
            let src = blurred.get_pixel((n - 1 - x) as u32, (n - 1 - y) as u32);
            let dx = (x as f32 - center) / center;
            let dy = (y as f32 - center) / center;
            let r = (dx * dx + dy * dy).sqrt();
            let edge = (1.0 - r).clamp(0.0, 1.0);
            let a = edge * edge * (3.0 - 2.0 * edge);
            let i = (y * n + x) * 4;
            // Işık gibi parlasın: rengi koru, açıklığı 0.42–0.82 bandına kaldır (koyu kapakta
            // kontrast artırmak haleyi zeminle aynı karanlığa gömüyordu).
            let (h, sat, l) = rgb_to_hsl(src[0] as f64 / 255.0, src[1] as f64 / 255.0, src[2] as f64 / 255.0);
            let (h, sat) = halo_tint(h, sat, vivid);
            let (r2, g2, b2) = hsl_to_rgb(h, (sat * 1.15).min(0.9), 0.42 + l * 0.4);
            for (c, v) in [r2, g2, b2].into_iter().enumerate() {
                let d = rng.unit() - rng.unit();
                blur2[i + c] = (v as f32 * 255.0 + d).round().clamp(0.0, 255.0) as u8;
            }
            let da = rng.unit() - rng.unit();
            blur2[i + 3] = (a * 255.0 + da).round().clamp(0.0, 255.0) as u8;
        }
    }
    let (width, height) = img.dimensions();
    Ok(Art { rgba: img.into_raw(), width, height, accent, blur, blur2, palette })
}

/// Görseli önbellekten ya da ağdan getir (ham bayt).
fn fetch(key: &str, url: &str) -> Result<Vec<u8>, String> {
    let dir = cache_dir();
    let path = dir.join(format!("{key}.img"));
    let bytes = match std::fs::read(&path) {
        Ok(b) => {
            // Kullanıldı: süre temizliğinde silinmesin.
            if let Ok(f) = std::fs::File::options().append(true).open(&path) {
                let _ = f.set_modified(std::time::SystemTime::now());
            }
            b
        }
        Err(_) if std::fs::metadata(crate::downloads::cover_path(key)).is_ok() => {
            // İndirilmiş şarkının yanına kaydedilen kapak (çevrimdışı da görünsün)
            std::fs::read(crate::downloads::cover_path(key)).map_err(|e| e.to_string())?
        }
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
    Ok(bytes)
}

/// Sanatçı sayfasının geniş afişi (kırpma yok, vurgu ve bulanıklık hesaplanmaz).
pub fn load_wide(key: &str, url: &str, w: u32, h: u32) -> Result<Art, String> {
    if url.is_empty() {
        return Err("afiş yok".into());
    }
    let bytes = fetch(key, url)?;
    let img = image::load_from_memory(&bytes).map_err(|e| e.to_string())?;
    let img = img.resize_to_fill(w, h, image::imageops::FilterType::Triangle).to_rgba8();
    let (width, height) = img.dimensions();
    Ok(Art { rgba: img.into_raw(), width, height, accent: (0, 0, 0), blur: Vec::new(), blur2: Vec::new(), palette: [(0, 0, 0); 3] })
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

/// Hale için kapağın en canlı rengi (ton, doygunluk): paletten ya da vurgudan.
/// Kapak tamamen griyse None (hale beyazımsı kalır).
fn halo_vivid(palette: &[(u8, u8, u8); 3], accent: (u8, u8, u8)) -> Option<(f64, f64)> {
    palette
        .iter()
        .chain(std::iter::once(&accent))
        .map(|&(r, g, b)| rgb_to_hsl(r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0))
        // Çok koyu ya da çok açık renklerin doygunluğu yanıltıcı: orta açıklığa yakın olan önde
        .map(|(h, s, l)| (h, s, s * (1.0 - (l - 0.5).abs() * 1.2).max(0.0)))
        .max_by(|a, b| a.2.total_cmp(&b.2))
        .filter(|v| v.1 >= 0.12)
        .map(|(h, s, _)| (h, s))
}

/// Soluk pikseli canlı tona doğru çek: doygunluk 0.35'in altındaysa ton ve doygunluk
/// canlı renge karışır (ağırlık soluklukla artar).
fn halo_tint(h: f64, sat: f64, vivid: Option<(f64, f64)>) -> (f64, f64) {
    const FLOOR: f64 = 0.35;
    let Some((vh, vs)) = vivid else { return (h, sat) };
    if sat >= FLOOR {
        return (h, sat);
    }
    let w = (FLOOR - sat) / FLOOR;
    // tonu kısa yoldan karıştır (0..1 çember)
    let mut d = vh - h;
    if d > 0.5 {
        d -= 1.0;
    } else if d < -0.5 {
        d += 1.0;
    }
    let h2 = (h + d * w).rem_euclid(1.0);
    let s2 = sat + (vs.max(0.55) - sat) * w;
    (h2, s2)
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

/// Kapak önbelleği temizliği: `days` gündür kullanılmayanları sil, sonra toplamı `limit` baytın altına indir.
/// Listelere özel kapaklar: kullanıcı verisi, önbellek temizliği silmez.
pub fn playlist_cover_dir() -> PathBuf {
    crate::paths::data_dir().join("playlist-covers")
}

/// Liste kapağı kare boyutu (başlıkta 180 px gösterilir).
pub const PL_COVER: u32 = 320;

/// Seçilen resmi kareye kırpıp küçültür ve kendi klasörüne kaydeder; dosya adını döner.
pub fn import_playlist_cover(src: &std::path::Path) -> Result<String, String> {
    let img = image::open(src).map_err(|e| format!("resim açılamadı: {e}"))?;
    let img = img.resize_to_fill(PL_COVER, PL_COVER, image::imageops::FilterType::Lanczos3);
    let dir = playlist_cover_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let name = format!("pl-{stamp}.png");
    img.to_rgba8().save(dir.join(&name)).map_err(|e| format!("kaydedilemedi: {e}"))?;
    Ok(name)
}

/// Kaydedilmiş liste kapağını çöz (vurgu rengiyle).
pub fn load_playlist_cover(name: &str) -> Result<Art, String> {
    let img = image::open(playlist_cover_dir().join(name)).map_err(|e| e.to_string())?.to_rgba8();
    let accent = accent_of(&img);
    let (width, height) = img.dimensions();
    Ok(Art { rgba: img.into_raw(), width, height, accent, blur: Vec::new(), blur2: Vec::new(), palette: [(0, 0, 0); 3] })
}

pub fn remove_playlist_cover(name: &str) {
    if !name.is_empty() && !name.contains('/') && !name.contains('\\') {
        let _ = std::fs::remove_file(playlist_cover_dir().join(name));
    }
}

pub fn prune(days: u64, limit: u64) {
    let Ok(rd) = std::fs::read_dir(cache_dir()) else { return };
    let mut files: Vec<(PathBuf, u64, std::time::SystemTime)> = Vec::new();
    for e in rd.flatten() {
        let Ok(m) = e.metadata() else { continue };
        let t = m.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        if t.elapsed().map(|d| d.as_secs() > days * 86400).unwrap_or(false) {
            let _ = std::fs::remove_file(e.path());
            continue;
        }
        files.push((e.path(), m.len(), t));
    }
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    files.sort_by_key(|f| f.2);
    for (p, len, _) in files {
        if total <= limit {
            break;
        }
        if std::fs::remove_file(&p).is_ok() {
            total -= len;
        }
    }
}
