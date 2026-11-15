// YouTube Music: veri ymus CLI'dan (ytmusicapi, kullanıcının girişiyle), ses mpv+yt-dlp ile.
// İleride Rust'a taşınacak; şimdilik ymus'un denenmiş yolunu kullanıyoruz.

use std::process::Command;

use serde::Deserialize;

use super::{Provider, Res};
use crate::model::{Card, Section, Source, TrackInfo};

pub struct Ytm;

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

fn ymus(args: &[&str]) -> Res<String> {
    let out = Command::new("ymus")
        .args(args)
        .output()
        .map_err(|e| format!("ymus çalıştırılamadı: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
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
        return None; // liste/albüm kartları sonraki aşamada
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

impl Provider for Ytm {
    fn source(&self) -> Source {
        Source::Ytm
    }

    fn search(&self, query: &str, limit: usize) -> Res<Vec<TrackInfo>> {
        let raw = ymus(&["search", query, &limit.to_string()])?;
        let items: Vec<Item> = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
        Ok(items.into_iter().filter_map(to_track).collect())
    }

    fn home(&self) -> Res<Vec<Section>> {
        let raw = ymus(&["home"])?;
        let secs: Vec<HomeSection> = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
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

    fn stream_url(&self, track: &TrackInfo) -> Res<String> {
        Ok(format!("https://music.youtube.com/watch?v={}", track.id))
    }
}
