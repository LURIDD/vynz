// Kullanıcının kaydettiği listeler: ~/.local/share/vynz/library.json

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::model::TrackInfo;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Playlist {
    pub name: String,
    /// Aktarıldığı bağlantı (bilgi amaçlı).
    #[serde(default)]
    pub origin: String,
    pub tracks: Vec<TrackInfo>,
}

fn path() -> PathBuf {
    crate::paths::data_dir().join("library.json")
}

pub fn load() -> Vec<Playlist> {
    std::fs::read(path()).ok().and_then(|raw| serde_json::from_slice(&raw).ok()).unwrap_or_default()
}

pub fn save(lists: &[Playlist]) -> std::io::Result<()> {
    let p = path();
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    // Yarım yazılmış dosya kalmasın: önce geçici dosyaya yaz, sonra taşı.
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(lists).unwrap_or_default())?;
    std::fs::rename(tmp, p)
}
