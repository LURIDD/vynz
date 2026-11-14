// Kaynaktan bağımsız ortak veri modeli.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Source {
    Ytm,
    SoundCloud,
    Spotify,
}

impl Source {
    /// Arayüzdeki rozet sırası: 0 YouTube Music, 1 SoundCloud, 2 Spotify.
    pub fn index(self) -> i32 {
        match self {
            Source::Ytm => 0,
            Source::SoundCloud => 1,
            Source::Spotify => 2,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrackInfo {
    pub source: Source,
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    /// Saniye; bilinmiyorsa 0.
    pub duration: f32,
    /// Kapak adresi (büyük boy).
    pub thumb: String,
    /// Sayfa adresi (SoundCloud gibi kimlikten adres türetilemeyen kaynaklar için).
    #[serde(default)]
    pub url: Option<String>,
}

impl TrackInfo {
    /// Kaynaklar arası benzersiz anahtar (önbellek dosya adı vb.).
    pub fn key(&self) -> String {
        format!("{}-{}", self.source.index(), self.id)
    }
}

#[derive(Clone, Debug)]
pub struct Section {
    pub title: String,
    pub items: Vec<TrackInfo>,
}
