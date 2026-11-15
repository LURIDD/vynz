// Dinleme geçmişi ve beğeniler: öneri algoritmasının girdisi. Tamamen yerel.
//   ~/.local/share/vynz/history.jsonl  (her dinleme bir satır)
//   ~/.local/share/vynz/likes.json

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::import::norm;
use crate::model::TrackInfo;

/// Sinyallerin yarı ömrü (gün): eski dinlemeler daha az sayılır.
const HALF_LIFE_DAYS: f64 = 21.0;
const MAX_EVENTS: usize = 5000;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Listen {
    pub ts: u64,
    pub track: TrackInfo,
    /// Gerçekten dinlenen saniye (atlamalar sayılmaz).
    pub played: f32,
}

impl Listen {
    fn fraction(&self) -> f32 {
        let d = if self.track.duration > 0.0 { self.track.duration } else { 200.0 };
        (self.played / d).clamp(0.0, 1.0)
    }

    /// Bu dinlemenin beğeni sinyali: sonuna kadar dinlendi +1, erken atlandı eksi.
    pub fn signal(&self) -> f32 {
        let f = self.fraction();
        if f >= 0.9 {
            1.0
        } else if f >= 0.5 {
            0.6
        } else if self.played >= 30.0 {
            0.3
        } else if self.played < 30.0 && f < 0.3 {
            -0.6
        } else {
            0.0
        }
    }

    pub fn skipped(&self) -> bool {
        self.signal() < 0.0
    }
}

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn dir() -> PathBuf {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or(home.join(".local/share")).join("vynz")
}

/// Asıl sanatçı, karşılaştırma için sadeleştirilmiş.
pub fn artist_id(t: &TrackInfo) -> String {
    norm(t.artist.split(',').next().unwrap_or("")).split_whitespace().collect::<Vec<_>>().join(" ")
}

#[derive(Default)]
pub struct History {
    pub listens: Vec<Listen>,
    pub likes: Vec<TrackInfo>,
}

impl History {
    pub fn load() -> History {
        let mut h = History::default();
        if let Ok(raw) = std::fs::read_to_string(dir().join("history.jsonl")) {
            h.listens = raw.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
            let n = h.listens.len();
            if n > MAX_EVENTS {
                h.listens.drain(..n - MAX_EVENTS);
            }
        }
        if let Ok(raw) = std::fs::read(dir().join("likes.json")) {
            h.likes = serde_json::from_slice(&raw).unwrap_or_default();
        }
        h
    }

    pub fn record(&mut self, track: TrackInfo, played: f32) {
        if played < 3.0 {
            return; // yanlış tıklama ya da açılamadı
        }
        let l = Listen { ts: now(), track, played };
        let _ = std::fs::create_dir_all(dir());
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir().join("history.jsonl")) {
            if let Ok(line) = serde_json::to_string(&l) {
                let _ = writeln!(f, "{line}");
            }
        }
        self.listens.push(l);
    }

    pub fn is_liked(&self, key: &str) -> bool {
        self.likes.iter().any(|t| t.key() == key)
    }

    /// Beğeniyi değiştir; yeni durumu döner.
    pub fn toggle_like(&mut self, t: &TrackInfo) -> bool {
        let key = t.key();
        let liked = if let Some(i) = self.likes.iter().position(|x| x.key() == key) {
            self.likes.remove(i);
            false
        } else {
            self.likes.insert(0, t.clone());
            true
        };
        let _ = std::fs::create_dir_all(dir());
        let _ = std::fs::write(dir().join("likes.json"), serde_json::to_vec_pretty(&self.likes).unwrap_or_default());
        liked
    }

    fn decay(ts: u64, at: u64) -> f32 {
        let days = at.saturating_sub(ts) as f64 / 86400.0;
        0.5f64.powf(days / HALF_LIFE_DAYS) as f32
    }

    /// Şarkı puanları (zamanla azalan dinleme sinyalleri + beğeni).
    pub fn track_scores(&self) -> HashMap<String, (f32, TrackInfo)> {
        let at = now();
        let mut m: HashMap<String, (f32, TrackInfo)> = HashMap::new();
        for l in &self.listens {
            let e = m.entry(l.track.key()).or_insert((0.0, l.track.clone()));
            e.0 += l.signal() * Self::decay(l.ts, at);
        }
        for t in &self.likes {
            m.entry(t.key()).or_insert((0.0, t.clone())).0 += 3.0;
        }
        m
    }

    /// Sanatçı yakınlığı: şarkı puanlarının toplamı.
    pub fn artist_scores(&self) -> HashMap<String, f32> {
        let mut m: HashMap<String, f32> = HashMap::new();
        for (score, t) in self.track_scores().values() {
            *m.entry(artist_id(t)).or_insert(0.0) += *score;
        }
        m
    }

    /// Son dinlenenler (tekrarsız, yeniden eskiye).
    pub fn recent(&self, n: usize) -> Vec<TrackInfo> {
        let mut seen = std::collections::HashSet::new();
        self.listens
            .iter()
            .rev()
            .filter(|l| !l.skipped())
            .filter(|l| seen.insert(l.track.key()))
            .take(n)
            .map(|l| l.track.clone())
            .collect()
    }

    /// Son `days` gün içinde atlanan şarkılar.
    pub fn recently_skipped(&self, days: u64) -> std::collections::HashSet<String> {
        let since = now().saturating_sub(days * 86400);
        self.listens.iter().filter(|l| l.ts >= since && l.skipped()).map(|l| l.track.key()).collect()
    }

    /// Son `hours` saatte çalınanlar.
    pub fn played_since(&self, hours: u64) -> std::collections::HashSet<String> {
        let since = now().saturating_sub(hours * 3600);
        self.listens.iter().filter(|l| l.ts >= since).map(|l| l.track.key()).collect()
    }

    /// Arka plan hesapları için kopya.
    pub fn snapshot(&self) -> History {
        History { listens: self.listens.clone(), likes: self.likes.clone() }
    }

    pub fn is_empty(&self) -> bool {
        self.listens.is_empty() && self.likes.is_empty()
    }
}
