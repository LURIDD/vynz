// Kullanıcı ayarları: ~/.config/vynz/settings.json

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Seçenek listeleri arayüzdeki düğmelerle aynı sırada.
pub const CACHE_GB: [f32; 5] = [0.0, 1.0, 2.0, 5.0, 10.0];
pub const READAHEAD_SECS: [u32; 4] = [30, 60, 120, 0]; // 0 = sınırsız
pub const COOKIE_BROWSERS: [Option<&str>; 2] = [None, Some("firefox")];
/// Arka plan indirme hız sınırı (KB/sn; 0 = sınırsız).
pub const DOWNLOAD_KBPS: [u32; 4] = [256, 512, 1024, 0];
/// Dinlenmeyen şarkıların önbellekte kalma süresi (gün; 0 = süresiz).
pub const KEEP_DAYS: [u32; 4] = [14, 30, 90, 0];
/// Çapraz geçiş süresi (sn; 0 = kapalı).
pub const CROSSFADE_SECS: [u32; 5] = [0, 3, 5, 8, 12];
/// Ekolayzır hazır ayarları (ad, EQ_BANDS sırasıyla dB). Arayüzdeki sırayla aynı.
pub const EQ_PRESETS: [(&str, [f32; 10]); 9] = [
    ("Düz", [0.0; 10]),
    ("Bas", [6.0, 5.0, 4.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
    ("Tiz", [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 2.0, 4.0, 5.0, 6.0]),
    ("Vokal", [-2.0, -2.0, -1.0, 0.0, 2.0, 4.0, 4.0, 2.0, 0.0, -1.0]),
    ("Rap", [5.0, 4.0, 2.0, 0.0, -1.0, 0.0, 1.0, 0.0, 1.0, 2.0]),
    ("Elektronik", [4.0, 3.0, 1.0, 0.0, -2.0, 0.0, 1.0, 2.0, 3.0, 4.0]),
    ("Akustik", [3.0, 2.0, 1.0, 1.0, 2.0, 2.0, 3.0, 3.0, 2.0, 1.0]),
    ("Rock", [4.0, 3.0, 2.0, 0.0, -1.0, -1.0, 1.0, 3.0, 4.0, 4.0]),
    ("Gece", [-1.0, 0.0, 1.0, 1.0, 0.0, -1.0, -2.0, -3.0, -4.0, -5.0]),
];
/// Vurgu renkleri (0 = kapaktan otomatik).
pub const ACCENTS: [u32; 9] = [0, 0xA78BFA, 0x60A5FA, 0x34D399, 0xFB7185, 0xFBBF24, 0xF97316, 0x22D3EE, 0xE4E6EB];
/// Zeminler: (canvas, outer, floating, drawer)
pub const BACKDROPS: [[u32; 4]; 5] = [
    [0x090A0D, 0x050507, 0x13151C, 0x101218], // Gece
    [0x000000, 0x000000, 0x0C0C0E, 0x08080A], // OLED
    [0x0A0F1C, 0x060914, 0x131B2D, 0x0F1626], // Lacivert
    [0x0F0B16, 0x09060E, 0x1B1526, 0x151020], // Mor
    [0x141518, 0x0E0F11, 0x1E2025, 0x191B1F], // Grafit
];

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Son ses düzeyi (0..1); kapanışta kaydedilir, açılışta geri gelir
    pub volume: f32,
    pub cache_gb: f32,
    pub prefetch: bool,
    pub readahead_secs: u32,
    pub cookies_browser: Option<String>,
    pub download_kbps: u32,
    /// Kuyruk bitince çalan şarkının radyosuyla devam et.
    pub autoradio: bool,
    /// Söz vitrini: 0 Modern, 1 Klasik (serif), 2 Sade
    pub lyrics_style: i32,
    /// 0 küçük, 1 orta, 2 büyük
    pub lyrics_size: i32,
    /// 0 sola, 1 ortalı
    pub lyrics_align: i32,
    /// Söz satır aralığı: 0 sıkı, 1 normal, 2 geniş
    pub lyrics_spacing: i32,
    /// Hareketli kapak zemini
    pub lyrics_motion: bool,
    /// Önbellekteki şarkı bu kadar gün çalınmazsa silinir (0 = süresiz)
    pub keep_days: u32,
    /// Söz çevirisi: 0 kapalı, 1 Türkçe, 2 İngilizce
    pub lyrics_translate: i32,
    /// Ses seviyesi dengeleme (mpv dynaudnorm)
    pub normalize: bool,
    /// Discord Rich Presence (çalan şarkı profilde görünür)
    pub discord_rpc: bool,
    /// Windows'ta çarpıya basınca sistem tepsisine in (varsayılan açık)
    pub close_to_tray: bool,
    /// İlk açılış Zevkler seçimi tamamlandı mı (veya geçildi mi)
    pub tastes_done: bool,
    /// Seçilen müzik türleri (ana sayfa önerileri için)
    pub taste_genres: Vec<String>,
    /// Önerilmesi istenmeyen türler (negatif filtre)
    pub blocked_genres: Vec<String>,
    /// Zevkler ekranında beğenilen örnek şarkılar (ana sayfa tohumları)
    pub taste_tracks: Vec<crate::model::TrackInfo>,
    /// Çapraz geçiş (sn; 0 = kapalı)
    pub crossfade_secs: u32,
    pub eq_on: bool,
    /// EQ_BANDS sırasıyla dB
    pub eq_gains: [f32; 10],
    /// Seçili hazır ayar; -1 = özel
    pub eq_preset: i32,
    /// ACCENTS indeksi (0 = kapaktan)
    pub theme_accent: i32,
    /// BACKDROPS indeksi
    pub theme_backdrop: i32,
    /// Açılışta güncelleme denetimi
    pub auto_update: bool,
    /// "Bu sürümü atla" denen sürüm
    pub skip_version: String,
    /// Ortam zemini stili: 0 kapak akışı, 1 renk lekeleri, 2 sade
    pub bg_style: i32,
    /// Ortam zemini parlaklığı: 0 sade, 1 orta, 2 canlı
    pub bg_level: i32,
    /// Ortam zemini çalarken süzülsün
    pub bg_motion: bool,
    /// Ortam zemini basla nabız atsın (Linux: cava)
    pub bg_reactive: bool,
    /// Ortam zemininde ince kumlanma dokusu
    pub bg_grain: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            volume: 0.8,
            cache_gb: 2.0,
            prefetch: true,
            readahead_secs: 60,
            cookies_browser: None,
            download_kbps: 512,
            autoradio: true,
            lyrics_style: 0,
            lyrics_size: 1,
            lyrics_align: 0,
            lyrics_spacing: 1,
            lyrics_motion: true,
            keep_days: 30,
            lyrics_translate: 0,
            normalize: false,
            discord_rpc: true,
            close_to_tray: true,
            tastes_done: false,
            taste_genres: Vec::new(),
            blocked_genres: Vec::new(),
            taste_tracks: Vec::new(),
            crossfade_secs: 0,
            eq_on: false,
            eq_gains: [0.0; 10],
            eq_preset: 0,
            theme_accent: 0,
            theme_backdrop: 0,
            auto_update: true,
            skip_version: String::new(),
            bg_style: 1,
            bg_level: 1,
            bg_motion: true,
            bg_reactive: false,
            bg_grain: true,
        }
    }
}

fn path() -> PathBuf {
    crate::paths::config_dir().join("settings.json")
}

impl Settings {
    pub fn load() -> Settings {
        match std::fs::read(path()) {
            Ok(raw) => serde_json::from_slice(&raw).unwrap_or_default(),
            Err(_) => {
                // İlk açılış: Firefox'ta YouTube girişi varsa onu kullan (Premium kalitesi).
                let mut s = Settings::default();
                if firefox_profile_exists() {
                    s.cookies_browser = Some("firefox".into());
                }
                let _ = s.save();
                s
            }
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        let p = path();
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::write(p, serde_json::to_vec_pretty(self).unwrap_or_default())
    }

    pub fn cache_bytes(&self) -> u64 {
        (self.cache_gb as f64 * 1024.0 * 1024.0 * 1024.0) as u64
    }

    pub fn crossfade_index(&self) -> i32 {
        CROSSFADE_SECS.iter().position(|k| *k == self.crossfade_secs).unwrap_or(0) as i32
    }

    pub fn cache_index(&self) -> i32 {
        CACHE_GB.iter().position(|g| (*g - self.cache_gb).abs() < 0.01).unwrap_or(2) as i32
    }

    pub fn readahead_index(&self) -> i32 {
        READAHEAD_SECS.iter().position(|s| *s == self.readahead_secs).unwrap_or(1) as i32
    }

    pub fn keep_index(&self) -> i32 {
        KEEP_DAYS.iter().position(|d| *d == self.keep_days).unwrap_or(1) as i32
    }

    pub fn download_index(&self) -> i32 {
        DOWNLOAD_KBPS.iter().position(|k| *k == self.download_kbps).unwrap_or(1) as i32
    }

    pub fn cookies_index(&self) -> i32 {
        COOKIE_BROWSERS
            .iter()
            .position(|b| b.map(str::to_string) == self.cookies_browser)
            .unwrap_or(0) as i32
    }
}

fn firefox_profile_exists() -> bool {
    let home = crate::paths::home_dir();
    let mut dirs_to_check = vec![home.join(".mozilla/firefox"), home.join(".config/mozilla/firefox")];
    if let Some(cfg) = dirs::config_dir() {
        dirs_to_check.push(cfg.join("Mozilla/Firefox/Profiles"));
    }
    dirs_to_check
        .iter()
        .any(|d| std::fs::read_dir(d).map(|mut r| r.any(|e| e.is_ok_and(|e| e.path().join("cookies.sqlite").exists()))).unwrap_or(false))
}
