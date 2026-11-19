// YouTube Music: veri sıcak çözücü yardımcısından (ytmusicapi), ses libmpv + yt-dlp ile.

use serde::Deserialize;

use super::{Provider, Res};
use crate::model::{Card, Section, Source, TrackInfo};
use crate::resolve::{Prio, Resolver};

pub struct Ytm {
    resolver: Resolver,
}

impl Ytm {
    pub fn new(resolver: Resolver) -> Self {
        Self { resolver }
    }
}

#[derive(Deserialize)]
struct Item {
    id: Option<String>,
    #[serde(default)]
    title: String,
    #[serde(default)]
    artist: Option<String>,
    #[serde(default)]
    album: Option<String>,
    #[serde(default)]
    thumb: Option<String>,
    #[serde(default)]
    duration: Option<f64>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    key: Option<String>,
    #[serde(default)]
    sub: Option<String>,
}

#[derive(Deserialize)]
struct HomeSection {
    title: String,
    items: Vec<Item>,
}

/// ytmusicapi küçük kapak verir (w120); büyüğünü iste.
pub fn big_thumb(url: &str) -> String {
    match url.find("=w") {
        Some(i) => format!("{}=w544-h544-l90-rj", &url[..i]),
        None => url.to_string(),
    }
}

fn to_track(it: Item) -> Option<TrackInfo> {
    if it.kind.as_deref().is_some_and(|k| k != "track") {
        return None;
    }
    Some(TrackInfo {
        source: Source::Ytm,
        id: it.id?,
        title: it.title,
        artist: it.artist.unwrap_or_default(),
        album: it.album.unwrap_or_default(),
        duration: it.duration.unwrap_or(0.0) as f32,
        thumb: it.thumb.as_deref().map(big_thumb).unwrap_or_default(),
        url: None,
    })
}

fn parse_home(data: serde_json::Value) -> Res<Vec<Section>> {
    let secs: Vec<HomeSection> = serde_json::from_value(data).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for s in secs {
        let mut tracks = Vec::new();
        let mut cards = Vec::new();
        for it in s.items {
            match it.kind.as_deref() {
                Some("playlist") | Some("album") => {
                    if let Some(key) = it.key.clone() {
                        cards.push(Card {
                            kind: it.kind.clone().unwrap_or_default(),
                            key,
                            title: it.title.clone(),
                            sub: it.sub.clone().unwrap_or_default(),
                            thumb: it.thumb.as_deref().map(big_thumb).unwrap_or_default(),
                        });
                    }
                }
                _ => {
                    if let Some(t) = to_track(it) {
                        tracks.push(t);
                    }
                }
            }
        }
        if !tracks.is_empty() {
            out.push(Section { title: s.title.clone(), items: tracks, cards: Vec::new(), style: 0 });
        }
        if !cards.is_empty() {
            out.push(Section { title: s.title, items: Vec::new(), cards, style: 2 });
        }
    }
    Ok(out)
}

impl Provider for Ytm {
    fn source(&self) -> Source {
        Source::Ytm
    }

    fn search(&self, query: &str, limit: usize) -> Res<Vec<TrackInfo>> {
        let data = self.resolver.request_sync(
            serde_json::json!({"op": "search", "q": query, "limit": limit.max(20)}),
            Prio::Soon,
        )?;
        let songs = data.get("songs").cloned().unwrap_or(serde_json::Value::Array(Vec::new()));
        let items: Vec<Item> = serde_json::from_value(songs).map_err(|e| e.to_string())?;
        Ok(items.into_iter().filter_map(to_track).take(limit).collect())
    }

    fn home(&self) -> Res<Vec<Section>> {
        let data = self.resolver.request_sync(serde_json::json!({"op": "home"}), Prio::Soon)?;
        parse_home(data)
    }

    fn stream_url(&self, track: &TrackInfo) -> Res<String> {
        Ok(format!("https://music.youtube.com/watch?v={}", track.id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_home_splits_tracks_and_cards() {
        let raw = serde_json::json!([
            {
                "title": "Hızlı seçimler",
                "items": [
                    {"kind": "track", "id": "abc", "title": "Şarkı", "artist": "Sanatçı", "duration": 180.0, "thumb": "https://x=w120-h120"},
                    {"kind": "album", "key": "MPREb_1", "title": "Albüm", "sub": "Sanatçı · 2024", "thumb": "https://y=w120-h120"}
                ]
            }
        ]);
        let secs = parse_home(raw).unwrap();
        assert_eq!(secs.len(), 2);
        assert_eq!(secs[0].items.len(), 1);
        assert_eq!(secs[0].items[0].thumb, "https://x=w544-h544-l90-rj");
        assert_eq!(secs[1].cards.len(), 1);
        assert_eq!(secs[1].cards[0].key, "MPREb_1");
    }
}
