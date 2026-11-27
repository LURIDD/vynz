// Profil istatistikleri: dinleme geçmişinden.

use std::collections::HashMap;

use crate::history::{now, History};
use crate::model::TrackInfo;

pub struct Stats {
    pub total_secs: f32,
    pub week_secs: f32,
    pub tracks: usize,
    pub artists: usize,
    pub likes: usize,
    /// (sanatçı adı, dinlenen sn)
    pub top_artists: Vec<(String, f32)>,
    /// (şarkı, dinlenme sayısı, dinlenen sn)
    pub top_tracks: Vec<(TrackInfo, u32, f32)>,
    /// Yerel saate göre 24 dilim (dinlenen sn)
    pub hours: [f32; 24],
}

/// Yerel saat farkı (sn): `date +%z` ile, ek bağımlılık olmadan.
fn utc_offset() -> i64 {
    std::process::Command::new("date")
        .arg("+%z")
        .output()
        .ok()
        .and_then(|o| {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            let sign = if s.starts_with('-') { -1 } else { 1 };
            let d = s.trim_start_matches(['+', '-']);
            let h: i64 = d.get(0..2)?.parse().ok()?;
            let m: i64 = d.get(2..4)?.parse().ok()?;
            Some(sign * (h * 3600 + m * 60))
        })
        .unwrap_or(0)
}

pub fn compute(h: &History) -> Stats {
    let week_ago = now().saturating_sub(7 * 86400);
    let off = utc_offset();
    let mut total = 0.0;
    let mut week = 0.0;
    let mut hours = [0f32; 24];
    let mut by_artist: HashMap<String, (String, f32)> = HashMap::new();
    let mut by_track: HashMap<String, (TrackInfo, u32, f32)> = HashMap::new();
    for l in &h.listens {
        total += l.played;
        if l.ts >= week_ago {
            week += l.played;
        }
        let local = (l.ts as i64 + off).rem_euclid(86400);
        hours[(local / 3600) as usize] += l.played;
        let name = l.track.artist.split(',').next().unwrap_or("").trim().to_string();
        if !name.is_empty() {
            let e = by_artist.entry(crate::history::artist_id(&l.track)).or_insert((name, 0.0));
            e.1 += l.played;
        }
        let e = by_track.entry(l.track.key()).or_insert((l.track.clone(), 0, 0.0));
        if !l.skipped() {
            e.1 += 1;
        }
        e.2 += l.played;
    }
    let artists = by_artist.len();
    let tracks = by_track.len();
    let mut top_artists: Vec<(String, f32)> = by_artist.into_values().collect();
    top_artists.sort_by(|a, b| b.1.total_cmp(&a.1));
    top_artists.truncate(8);
    let mut top_tracks: Vec<(TrackInfo, u32, f32)> = by_track.into_values().filter(|t| t.1 > 0).collect();
    top_tracks.sort_by(|a, b| b.1.cmp(&a.1).then(b.2.total_cmp(&a.2)));
    top_tracks.truncate(10);
    Stats { total_secs: total, week_secs: week, tracks, artists, likes: h.likes.len(), top_artists, top_tracks, hours }
}

/// "3 sa 12 dk" / "45 dk"
pub fn human(secs: f32) -> String {
    let m = (secs / 60.0).round() as u64;
    if m >= 60 { tr!("{} sa {} dk", m / 60, m % 60) } else { tr!("{} dk", m) }
}
