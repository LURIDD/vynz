// Ortam zemini: kapak renklerinden süzülen yumuşak ışık lekeleri + basamaklanmayı kıran kumlanma.
//
// Slint'te büyük radial-gradient'ler 8 bit nicemlemede koyu tonlarda basamak (banding) yapıyor.
// Bunun yerine bir kez üretilen beyaz, alfa kanalı titreşimli (dither) yumuşak disk görseli
// `colorize` ile kapak renklerine boyanır; üstüne ince, döşenen bir gürültü katmanı gelir.

/// Işık diski (beyaz, alfa = yumuşak düşüş). Kenara varmadan tamamen söner.
pub const DISC: u32 = 256;
/// Kumlanma dokusu kenarı (döşenir).
pub const GRAIN: u32 = 128;

/// Basit, tohumlu xorshift: her açılışta aynı doku (titreme yapmasın).
struct Rng(u32);
impl Rng {
    fn next(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }
    /// 0..1
    fn unit(&mut self) -> f32 {
        (self.next() >> 8) as f32 / (1u32 << 24) as f32
    }
}

pub fn disc_rgba() -> Vec<u8> {
    let n = DISC as usize;
    let mut out = vec![0u8; n * n * 4];
    let mut rng = Rng(0x9E37_79B9);
    let c = (n as f32 - 1.0) / 2.0;
    for y in 0..n {
        for x in 0..n {
            let dx = (x as f32 - c) / c;
            let dy = (y as f32 - c) / c;
            let r = (dx * dx + dy * dy).sqrt();
            // Gauss benzeri düşüş, r=1'de sıfıra bağlanır (kare kenarı hiç görünmez)
            let g = (-r * r * 3.2).exp();
            let edge = (1.0 - r).clamp(0.0, 1.0);
            let a = g * (edge * (2.0 - edge)); // kenarda yumuşakça 0
            // Üçgen dağılımlı titreşim: ±1 LSB
            let d = rng.unit() - rng.unit();
            let v = (a * 255.0 + d).round().clamp(0.0, 255.0) as u8;
            let i = (y * n + x) * 4;
            // Renk kanalı = alfa (önceden çarpılmış beyaz). colorize rengi zaten değiştirir; ama
            // renk > alfa olan pikseller GPU çiziminde koyu beneklere dönüşüyordu.
            out[i] = v;
            out[i + 1] = v;
            out[i + 2] = v;
            out[i + 3] = v;
        }
    }
    out
}

/// Beyaz diski verilen renge boya (düz alfa; renk kanalı sabit, alfa diskten).
pub fn tinted_disc(base: &[u8], c: (u8, u8, u8)) -> Vec<u8> {
    let mut out = base.to_vec();
    for p in out.chunks_exact_mut(4) {
        p[0] = c.0;
        p[1] = c.1;
        p[2] = c.2;
    }
    out
}

/// Kumlanma: yarısı açık yarısı koyu, çok düşük alfa. Ortalaması nötr kalır, yalnız basamakları dağıtır.
pub fn grain_rgba() -> Vec<u8> {
    let n = GRAIN as usize;
    let mut out = vec![0u8; n * n * 4];
    let mut rng = Rng(0x1234_5677);
    for p in out.chunks_exact_mut(4) {
        let light = rng.next() & 1 == 0;
        let a = (rng.unit() * 10.0) as u8;
        let v = if light { 255 } else { 0 };
        p[0] = v;
        p[1] = v;
        p[2] = v;
        p[3] = a;
    }
    out
}

/// Kapaktan üç zemin rengi: ağırlıklı ton kovaları, birbirinden en az ~40° uzak.
/// Gri kapakta vurgu tonunun komşuları döner. Dönüş (r, g, b), orta açıklıkta.
pub fn palette(img: &image::RgbaImage, accent: (u8, u8, u8)) -> [(u8, u8, u8); 3] {
    const BINS: usize = 18; // 20° kova
    let mut w = [0f64; BINS];
    let mut sums = [(0f64, 0f64); BINS]; // (s, l) ağırlıklı toplam
    let mut seen = 0f64;
    for p in img.pixels().step_by(3) {
        seen += 1.0;
        let (r, g, b) = (p[0] as f64 / 255.0, p[1] as f64 / 255.0, p[2] as f64 / 255.0);
        let (h, s, l) = rgb_to_hsl(r, g, b);
        if s < 0.12 || l < 0.08 || l > 0.95 {
            continue;
        }
        let k = ((h * BINS as f64) as usize).min(BINS - 1);
        let wt = s * s * (1.0 - (l - 0.5).abs());
        w[k] += wt;
        sums[k].0 += s * wt;
        sums[k].1 += l * wt;
    }
    let total: f64 = w.iter().sum();
    // Kapak ne kadar renkli: 0 (gri) .. 1. Az renkli kapakta palet de soluk kalır.
    let vivid = (total / seen.max(1.0) / 0.10).clamp(0.0, 1.0);
    let (ah, _, _) = rgb_to_hsl(accent.0 as f64 / 255.0, accent.1 as f64 / 255.0, accent.2 as f64 / 255.0);
    let mut picks: Vec<(f64, f64)> = Vec::new(); // (ton, doygunluk)
    if total > 0.5 {
        let mut order: Vec<usize> = (0..BINS).collect();
        order.sort_by(|a, b| w[*b].total_cmp(&w[*a]));
        for k in order {
            if picks.len() == 3 || w[k] < total * 0.06 {
                break;
            }
            let h = (k as f64 + 0.5) / BINS as f64;
            if picks.iter().all(|(ph, _)| hue_dist(*ph, h) >= 40.0 / 360.0) {
                picks.push((h, sums[k].0 / w[k]));
            }
        }
    }
    // Renksiz (gri/siyah-beyaz) kapak: palet de nötr kalsın, hafif soğuk griler
    let (_, a_s, _) = rgb_to_hsl(accent.0 as f64 / 255.0, accent.1 as f64 / 255.0, accent.2 as f64 / 255.0);
    if picks.is_empty() && a_s < 0.15 {
        let grey = |s: f64, l: f64| {
            let (r, g, b) = hsl_to_rgb(0.62, s, l);
            ((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
        };
        return [grey(0.10, 0.62), grey(0.14, 0.48), grey(0.06, 0.70)];
    }
    // Eksik kalanları vurgu tonunun komşularıyla doldur (analog palet)
    let base = picks.first().copied().unwrap_or((ah, 0.55));
    let mut shift = [0.09, -0.11, 0.18].into_iter();
    while picks.len() < 3 {
        let d = shift.next().unwrap_or(0.25);
        picks.push(((base.0 + d).rem_euclid(1.0), base.1 * 0.8));
    }
    let mut out = [(0u8, 0u8, 0u8); 3];
    for (i, (h, s)) in picks.into_iter().enumerate() {
        // canlı kapakta doygunluk 0.45-0.85; gri kapakta 0.08'e kadar iner
        let s = 0.08 + (s.clamp(0.45, 0.85) - 0.08) * vivid;
        let (r, g, b) = hsl_to_rgb(h, s, 0.55);
        out[i] = ((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8);
    }
    out
}

fn hue_dist(a: f64, b: f64) -> f64 {
    let d = (a - b).abs();
    d.min(1.0 - d)
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
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let f = |t: f64| {
        let t = t.rem_euclid(1.0);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disc_edges_are_transparent() {
        let d = disc_rgba();
        let n = DISC as usize;
        // köşe ve kenar ortası tamamen saydam, merkez dolu
        assert_eq!(d[3], 0);
        assert_eq!(d[((n / 2) * n) * 4 + 3], 0);
        assert!(d[((n / 2) * n + n / 2) * 4 + 3] > 200);
    }

    #[test]
    fn grey_cover_falls_back_to_accent_neighbours() {
        let img = image::RgbaImage::from_pixel(16, 16, image::Rgba([90, 90, 90, 255]));
        let p = palette(&img, (167, 139, 250));
        assert_ne!(p[0], p[1]);
        assert_ne!(p[1], p[2]);
    }

    #[test]
    fn mostly_grey_cover_gives_muted_palette() {
        // %95 gri, %5 doygun camgöbeği
        let mut img = image::RgbaImage::from_pixel(20, 20, image::Rgba([90, 90, 95, 255]));
        for x in 0..20 {
            img.put_pixel(x, 0, image::Rgba([20, 200, 210, 255]));
        }
        for (r, g, b) in palette(&img, (170, 170, 172)) {
            let (_, s, _) = rgb_to_hsl(r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0);
            assert!(s < 0.35, "{r},{g},{b} s={s}");
        }
        // tamamen doygun kapak canlı kalır
        let img = image::RgbaImage::from_pixel(20, 20, image::Rgba([220, 40, 60, 255]));
        let (r, g, b) = palette(&img, (220, 90, 100))[0];
        let (_, s, _) = rgb_to_hsl(r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0);
        assert!(s > 0.6);
    }

    #[test]
    fn grey_cover_with_neutral_accent_stays_neutral() {
        let img = image::RgbaImage::from_pixel(16, 16, image::Rgba([90, 90, 90, 255]));
        let p = palette(&img, (170, 170, 172));
        for (r, g, b) in p {
            let (_, s, _) = rgb_to_hsl(r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0);
            assert!(s < 0.2, "{r},{g},{b}");
        }
    }
}
