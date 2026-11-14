// SoundCloud: arama ve akış yt-dlp ile (girişsiz). Ana sayfa akışı yok.

use std::process::Command;

use serde::Deserialize;

use super::{Provider, Res};
use crate::model::{Section, Source, TrackInfo};

pub struct SoundCloud;

#[derive(Deserialize)]
struct Search {
    #[serde(default)]
    entries: Vec<Entry>,
}

#[derive(Deserialize)]
struct Thumb {
    url: String,
}

#[derive(Deserialize)]
struct Entry {
    id: Option<String>,
    #[serde(default)]
    title: String,
    #[serde(default)]
    uploader: Option<String>,
    #[serde(default)]
    duration: Option<f64>,
    #[serde(default)]
    webpage_url: Option<String>,
    #[serde(default)]
    thumbnails: Vec<Thumb>,
}

/// "Sanatçı - Şarkı" biçimindeyse ayır; değilse yükleyeni sanatçı say.
fn split_title(title: &str, uploader: &str) -> (String, String) {
    for sep in [" - ", " – ", " — "] {
        if let Some((a, t)) = title.split_once(sep) {
            if !a.trim().is_empty() && !t.trim().is_empty() {
                return (t.trim().to_string(), a.trim().to_string());
            }
        }
    }
    (title.to_string(), uploader.to_string())
}

/// sndcdn adresleri boyut ekiyle biter (-large, -t67x67...): 500 piksellik olanı iste.
fn big_thumb(thumbs: &[Thumb]) -> String {
    let Some(t) = thumbs.last() else { return String::new() };
    let url = &t.url;
    match (url.rfind('-'), url.rfind('.')) {
        (Some(d), Some(e)) if d < e => format!("{}-t500x500{}", &url[..d], &url[e..]),
        _ => url.clone(),
    }
}

impl Provider for SoundCloud {
    fn source(&self) -> Source {
        Source::SoundCloud
    }

    fn search(&self, query: &str, limit: usize) -> Res<Vec<TrackInfo>> {
        let out = Command::new("yt-dlp")
            .args(["--flat-playlist", "-J", "--no-warnings"])
            .arg(format!("scsearch{limit}:{query}"))
            .output()
            .map_err(|e| format!("yt-dlp çalıştırılamadı: {e}"))?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("SoundCloud hatası").to_string());
        }
        let s: Search = serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())?;
        Ok(s.entries
            .into_iter()
            .filter_map(|e| {
                let url = e.webpage_url?;
                let (title, artist) = split_title(&e.title, e.uploader.as_deref().unwrap_or(""));
                Some(TrackInfo {
                    source: Source::SoundCloud,
                    id: e.id.unwrap_or_else(|| url.clone()),
                    title,
                    artist,
                    album: String::new(),
                    duration: e.duration.unwrap_or(0.0) as f32,
                    thumb: big_thumb(&e.thumbnails),
                    url: Some(url),
                })
            })
            .collect())
    }

    fn home(&self) -> Res<Vec<Section>> {
        Ok(Vec::new())
    }

    fn stream_url(&self, track: &TrackInfo) -> Res<String> {
        track.url.clone().ok_or_else(|| "SoundCloud adresi yok".into())
    }
}
