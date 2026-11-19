// Platformdan bağımsız uygulama klasörleri (Linux'ta XDG, Windows'ta AppData).

use std::path::PathBuf;

pub fn home_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_default()
}

/// Yapılandırma klasörü: Linux'ta `$XDG_CONFIG_HOME/vynz` (~/.config/vynz), Windows'ta `%APPDATA%\vynz`.
pub fn config_dir() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| home_dir().join(".config")).join("vynz")
}

/// Kalıcı veri klasörü: Linux'ta `$XDG_DATA_HOME/vynz` (~/.local/share/vynz), Windows'ta `%LOCALAPPDATA%\vynz`.
pub fn data_dir() -> PathBuf {
    dirs::data_local_dir().unwrap_or_else(|| home_dir().join(".local/share")).join("vynz")
}

/// Önbellek klasörü: Linux'ta `$XDG_CACHE_HOME/vynz` (~/.cache/vynz), Windows'ta `%LOCALAPPDATA%\vynz`.
pub fn cache_dir() -> PathBuf {
    dirs::cache_dir().unwrap_or_else(|| home_dir().join(".cache")).join("vynz")
}

/// Başındaki `~/` (veya Windows'ta `~\`) önekini kullanıcı ana klasörüne açar.
pub fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) {
        home_dir().join(rest)
    } else {
        PathBuf::from(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dirs_end_with_vynz() {
        assert_eq!(config_dir().file_name().and_then(|s| s.to_str()), Some("vynz"));
        assert_eq!(data_dir().file_name().and_then(|s| s.to_str()), Some("vynz"));
        assert_eq!(cache_dir().file_name().and_then(|s| s.to_str()), Some("vynz"));
    }

    #[test]
    fn expand_tilde_works() {
        let p = expand_tilde("~/Muzik/liste.csv");
        assert!(p.ends_with("Muzik/liste.csv"));
        assert!(!p.to_string_lossy().starts_with("~/"));
        assert_eq!(expand_tilde("/tmp/a.json"), PathBuf::from("/tmp/a.json"));
    }
}
