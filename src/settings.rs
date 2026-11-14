// Kullanıcı ayarları: ~/.config/vynz/settings.json

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Seçenek listeleri arayüzdeki düğmelerle aynı sırada.
pub const CACHE_GB: [f32; 5] = [0.0, 1.0, 2.0, 5.0, 10.0];
pub const READAHEAD_SECS: [u32; 4] = [30, 60, 120, 0]; // 0 = sınırsız
pub const COOKIE_BROWSERS: [Option<&str>; 2] = [None, Some("firefox")];
/// Arka plan indirme hız sınırı (KB/sn; 0 = sınırsız).
pub const DOWNLOAD_KBPS: [u32; 4] = [256, 512, 1024, 0];

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub cache_gb: f32,
    pub prefetch: bool,
    pub readahead_secs: u32,
    pub cookies_browser: Option<String>,
    pub download_kbps: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { cache_gb: 2.0, prefetch: true, readahead_secs: 60, cookies_browser: None, download_kbps: 512 }
    }
}

fn path() -> PathBuf {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or(home.join(".config"))
        .join("vynz")
        .join("settings.json")
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

    pub fn cache_index(&self) -> i32 {
        CACHE_GB.iter().position(|g| (*g - self.cache_gb).abs() < 0.01).unwrap_or(2) as i32
    }

    pub fn readahead_index(&self) -> i32 {
        READAHEAD_SECS.iter().position(|s| *s == self.readahead_secs).unwrap_or(1) as i32
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
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    [home.join(".mozilla/firefox"), home.join(".config/mozilla/firefox")]
        .iter()
        .any(|d| std::fs::read_dir(d).map(|mut r| r.any(|e| e.is_ok_and(|e| e.path().join("cookies.sqlite").exists()))).unwrap_or(false))
}
