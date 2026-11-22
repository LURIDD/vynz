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
    crate::paths::data_dir()
}

/// Asıl sanatçı, karşılaştırma için sadeleştirilmiş.
pub fn artist_id(t: &TrackInfo) -> String {
    norm(t.artist.split(',').next().unwrap_or("")).split_whitespace().collect::<Vec<_>>().join(" ")
}

#[derive(Default)]
pub struct History {
    pub listens: Vec<Listen>,
    pub likes: Vec<TrackInfo>,
    pub disliked_tracks: Vec<TrackInfo>,
    pub disliked_artists: Vec<String>,
    /// "Bunu daha çok öner": şarkı ve asıl sanatçısı güçlü pozitif sinyal alır.
    pub boosted_tracks: Vec<TrackInfo>,
}

/// "Daha çok öner" ağırlığı (beğeni +3, İlgilenmiyorum −2.5).
pub const BOOST: f32 = 4.0;

#[derive(Default, Serialize, Deserialize)]
struct BoostsFile {
    #[serde(default)]
    tracks: Vec<TrackInfo>,
}

#[derive(Default, Serialize, Deserialize)]
struct DislikesFile {
    #[serde(default)]
    tracks: Vec<TrackInfo>,
    #[serde(default)]
    artists: Vec<String>,
}

pub fn norm_artist(name: &str) -> String {
    norm(name.split(',').next().unwrap_or("")).split_whitespace().collect::<Vec<_>>().join(" ")
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
        if let Ok(raw) = std::fs::read(dir().join("dislikes.json")) {
            if let Ok(d) = serde_json::from_slice::<DislikesFile>(&raw) {
                h.disliked_tracks = d.tracks;
                h.disliked_artists = d.artists;
            }
        }
        if let Ok(raw) = std::fs::read(dir().join("boosts.json")) {
            if let Ok(b) = serde_json::from_slice::<BoostsFile>(&raw) {
                h.boosted_tracks = b.tracks;
            }
        }
        h
    }

    fn save_boosts(&self) {
        let _ = std::fs::create_dir_all(dir());
        let f = BoostsFile { tracks: self.boosted_tracks.clone() };
        let _ = std::fs::write(dir().join("boosts.json"), serde_json::to_vec_pretty(&f).unwrap_or_default());
    }

    pub fn is_boosted(&self, key: &str) -> bool {
        self.boosted_tracks.iter().any(|t| t.key() == key)
    }

    /// "Bunu daha çok öner"i değiştir; yeni durumu döner. Öne çıkarılan şarkı "İlgilenmiyorum"dan çıkar.
    pub fn toggle_boost(&mut self, t: &TrackInfo) -> bool {
        let key = t.key();
        let on = if let Some(i) = self.boosted_tracks.iter().position(|x| x.key() == key) {
            self.boosted_tracks.remove(i);
            false
        } else {
            self.boosted_tracks.insert(0, t.clone());
            if self.disliked_tracks.iter().any(|x| x.key() == key) {
                self.undislike_track(&key);
            }
            true
        };
        self.save_boosts();
        on
    }

    /// Öneri profilini sıfırla: "daha çok öner" ve "İlgilenmiyorum" listeleri.
    /// Dinleme geçmişi ve beğeniler korunur.
    pub fn reset_profile(&mut self) {
        self.boosted_tracks.clear();
        self.disliked_tracks.clear();
        self.disliked_artists.clear();
        self.save_boosts();
        self.save_dislikes();
    }

    /// Geri al: sıfırlanan öneri profilini geri koy.
    pub fn restore_profile(&mut self, boosts: Vec<TrackInfo>, tracks: Vec<TrackInfo>, artists: Vec<String>) {
        self.boosted_tracks = boosts;
        self.disliked_tracks = tracks;
        self.disliked_artists = artists;
        self.save_boosts();
        self.save_dislikes();
    }

    /// Geri al: silinen dinleme geçmişini dosyasıyla birlikte geri yaz.
    pub fn restore_listens(&mut self, listens: Vec<Listen>) {
        let _ = std::fs::create_dir_all(dir());
        let body: String = listens.iter().filter_map(|l| serde_json::to_string(l).ok()).map(|s| s + "\n").collect();
        let _ = std::fs::write(dir().join("history.jsonl"), body);
        // Geri alma sırasında yeni dinleme eklendiyse onu da koru.
        let newer = std::mem::take(&mut self.listens);
        self.listens = listens;
        for l in newer {
            if let Ok(line) = serde_json::to_string(&l) {
                if let Ok(mut f) = std::fs::OpenOptions::new().append(true).open(dir().join("history.jsonl")) {
                    let _ = writeln!(f, "{line}");
                }
            }
            self.listens.push(l);
        }
    }

    /// Dinleme geçmişini sil (beğeniler kütüphane olduğu için korunur).
    pub fn clear_listens(&mut self) {
        self.listens.clear();
        let _ = std::fs::remove_file(dir().join("history.jsonl"));
    }

    fn save_dislikes(&self) {
        let _ = std::fs::create_dir_all(dir());
        let f = DislikesFile { tracks: self.disliked_tracks.clone(), artists: self.disliked_artists.clone() };
        let _ = std::fs::write(dir().join("dislikes.json"), serde_json::to_vec_pretty(&f).unwrap_or_default());
    }

    pub fn dislike_track(&mut self, t: &TrackInfo) {
        let key = t.key();
        if !self.disliked_tracks.iter().any(|x| x.key() == key) {
            self.disliked_tracks.insert(0, t.clone());
        }
        if let Some(pos) = self.boosted_tracks.iter().position(|x| x.key() == key) {
            self.boosted_tracks.remove(pos);
            self.save_boosts();
        }
        if let Some(pos) = self.likes.iter().position(|x| x.key() == key) {
            self.likes.remove(pos);
            let _ = std::fs::write(dir().join("likes.json"), serde_json::to_vec_pretty(&self.likes).unwrap_or_default());
        }
        self.save_dislikes();
    }

    pub fn undislike_track(&mut self, key: &str) {
        self.disliked_tracks.retain(|x| x.key() != key);
        self.save_dislikes();
    }

    pub fn dislike_artist(&mut self, artist: &str) {
        let primary = artist.split(',').next().unwrap_or("").trim();
        if primary.is_empty() {
            return;
        }
        let nid = norm_artist(primary);
        if !self.disliked_artists.iter().any(|a| norm_artist(a) == nid) {
            self.disliked_artists.insert(0, primary.to_string());
            self.save_dislikes();
        }
    }

    pub fn undislike_artist(&mut self, artist: &str) {
        let nid = norm_artist(artist);
        self.disliked_artists.retain(|a| norm_artist(a) != nid);
        self.save_dislikes();
    }

    pub fn disliked_track_keys(&self) -> std::collections::HashSet<String> {
        self.disliked_tracks.iter().map(TrackInfo::key).collect()
    }

    pub fn disliked_artist_ids(&self) -> std::collections::HashSet<String> {
        self.disliked_artists.iter().map(|a| norm_artist(a)).filter(|s| !s.is_empty()).collect()
    }

    pub fn is_disliked(&self, t: &TrackInfo) -> bool {
        let key = t.key();
        if self.disliked_tracks.iter().any(|x| x.key() == key) {
            return true;
        }
        let aid = artist_id(t);
        !aid.is_empty() && self.disliked_artists.iter().any(|a| norm_artist(a) == aid)
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
        // İlgilenmiyorum: o şarkıyı negatif yapar, aynı sanatçıdan 2-3 tane olunca sanatçıyı da aşağı çeker.
        for t in &self.disliked_tracks {
            m.entry(t.key()).or_insert((0.0, t.clone())).0 -= 2.5;
        }
        // Daha çok öner: şarkı puanı, dolayısıyla sanatçı yakınlığı da yükselir.
        for t in &self.boosted_tracks {
            m.entry(t.key()).or_insert((0.0, t.clone())).0 += BOOST;
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

    /// Son dinlenenler (tekrarsız, yeniden eskiye, engellenenler hariç).
    pub fn recent(&self, n: usize) -> Vec<TrackInfo> {
        let mut seen = std::collections::HashSet::new();
        self.listens
            .iter()
            .rev()
            .filter(|l| !l.skipped() && !self.is_disliked(&l.track))
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
        History {
            listens: self.listens.clone(),
            likes: self.likes.clone(),
            disliked_tracks: self.disliked_tracks.clone(),
            disliked_artists: self.disliked_artists.clone(),
            boosted_tracks: self.boosted_tracks.clone(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.listens.is_empty() && self.likes.is_empty() && self.boosted_tracks.is_empty()
    }
}
