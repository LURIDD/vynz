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

/// Şarkı olmayan öğe: liste, albüm, sanatçı, ruh hâli ya da kayıtlı liste.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Card {
    /// "playlist" | "album" | "artist" | "mood" | "saved"
    pub kind: String,
    pub key: String,
    pub title: String,
    #[serde(default)]
    pub sub: String,
    #[serde(default)]
    pub thumb: String,
}

impl Card {
    pub fn art_key(&self) -> String {
        format!("c-{}", self.key.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect::<String>())
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Section {
    pub title: String,
    pub items: Vec<TrackInfo>,
    /// Kart rafları (style 2 liste/albüm, 3 sanatçı, 4 ruh hâli)
    #[serde(default)]
    pub cards: Vec<Card>,
    /// 0 şarkı kartları, 1 hızlı erişim kutucukları, 2 liste/albüm, 3 sanatçı, 4 ruh hâli
    #[serde(default)]
    pub style: i32,
}
