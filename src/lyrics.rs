// Şarkı sözleri: zamanlı satırlar.
// - YouTube Music: ymus (önce YouTube Music'in kendi sözleri, yoksa LRCLIB)
// - Diğer kaynaklar: LRCLIB araması (süresi en yakın, zamanlı olan seçilir)

use std::process::Command;

use serde::Deserialize;
use serde_json::Value;

use crate::model::{Source, TrackInfo};

#[derive(Clone, Debug)]
pub struct Line {
    pub t: f32,
    pub end: f32,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct Lyrics {
    pub synced: bool,
    pub source: String,
    pub lines: Vec<Line>,
}

#[derive(Deserialize)]
struct YmusLine {
    t: Option<f32>,
    end: Option<f32>,
    text: String,
}

#[derive(Deserialize)]
struct YmusLyrics {
    #[serde(default)]
    source: String,
    #[serde(default)]
    synced: bool,
    #[serde(default)]
    lines: Vec<YmusLine>,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

pub fn fetch(t: &TrackInfo) -> Result<Lyrics, String> {
    if t.source == Source::Ytm {
        if let Ok(l) = from_ymus(&t.id) {
            return Ok(l);
        }
    }
    from_lrclib(t)
}

fn from_ymus(id: &str) -> Result<Lyrics, String> {
    let out = Command::new("ymus").args(["lyrics", id]).output().map_err(|e| e.to_string())?;
    let y: YmusLyrics = serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())?;
    if let Some(e) = y.error {
        return Err(e);
    }
    let mut lines: Vec<Line> = y
        .lines
        .into_iter()
        .filter(|l| !l.text.trim().is_empty())
        .map(|l| Line { t: l.t.unwrap_or(0.0), end: l.end.unwrap_or(0.0), text: l.text })
        .collect();
    if lines.is_empty() {
        if let Some(text) = y.text {
            lines = text.lines().filter(|l| !l.trim().is_empty()).map(|l| Line { t: 0.0, end: 0.0, text: l.into() }).collect();
        }
    }
    if lines.is_empty() {
        return Err("söz yok".into());
    }
    fill_ends(&mut lines);
    Ok(Lyrics { synced: y.synced, source: if y.source.is_empty() { "ymus".into() } else { y.source }, lines })
}

fn from_lrclib(t: &TrackInfo) -> Result<Lyrics, String> {
    let artist = t.artist.split(',').next().unwrap_or("").trim();
    let mut resp = ureq::get("https://lrclib.net/api/search")
        .query("track_name", &t.title)
        .query("artist_name", artist)
        .header("User-Agent", "vynz (masaüstü müzik çalar)")
        .call()
        .map_err(|e| format!("LRCLIB: {e}"))?;
    let body = resp.body_mut().read_to_string().map_err(|e| e.to_string())?;
    let items: Vec<Value> = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    // Zamanlı olanlar önce, sonra süresi en yakın.
    let best = items
        .iter()
        .filter(|i| i.get("instrumental").and_then(Value::as_bool) != Some(true))
        .min_by_key(|i| {
            let synced = i.get("syncedLyrics").and_then(Value::as_str).is_some_and(|s| !s.is_empty());
            let d = i.get("duration").and_then(Value::as_f64).unwrap_or(0.0) as f32;
            let diff = if t.duration > 0.0 { (d - t.duration).abs() as i64 } else { 0 };
            (!synced as i64, diff)
        })
        .ok_or("söz bulunamadı")?;
    if let Some(lrc) = best.get("syncedLyrics").and_then(Value::as_str).filter(|s| !s.is_empty()) {
        let mut lines = parse_lrc(lrc);
        if !lines.is_empty() {
            fill_ends(&mut lines);
            return Ok(Lyrics { synced: true, source: "LRCLIB".into(), lines });
        }
    }
    let plain = best.get("plainLyrics").and_then(Value::as_str).ok_or("söz bulunamadı")?;
    let lines: Vec<Line> = plain.lines().filter(|l| !l.trim().is_empty()).map(|l| Line { t: 0.0, end: 0.0, text: l.into() }).collect();
    if lines.is_empty() {
        return Err("söz bulunamadı".into());
    }
    Ok(Lyrics { synced: false, source: "LRCLIB".into(), lines })
}

/// "[mm:ss.xx] metin" satırları.
pub fn parse_lrc(lrc: &str) -> Vec<Line> {
    let mut out = Vec::new();
    for raw in lrc.lines() {
        let mut rest = raw.trim();
        let mut stamps = Vec::new();
        while let Some(stripped) = rest.strip_prefix('[') {
            let Some(close) = stripped.find(']') else { break };
            let tag = &stripped[..close];
            if let Some((m, s)) = tag.split_once(':') {
                if let (Ok(m), Ok(s)) = (m.parse::<f32>(), s.parse::<f32>()) {
                    stamps.push(m * 60.0 + s);
                }
            }
            rest = stripped[close + 1..].trim_start();
        }
        if rest.is_empty() {
            continue;
        }
        for t in stamps {
            out.push(Line { t, end: 0.0, text: rest.to_string() });
        }
    }
    out.sort_by(|a, b| a.t.total_cmp(&b.t));
    out
}

/// Bitiş zamanı olmayan satırlara bir sonrakinin başlangıcını ver (en çok ~11 harf/sn).
fn fill_ends(lines: &mut [Line]) {
    for i in 0..lines.len() {
        if lines[i].end > lines[i].t {
            continue;
        }
        let next = lines.get(i + 1).map(|l| l.t).unwrap_or(lines[i].t + 5.0);
        let by_chars = lines[i].text.chars().count() as f32 / 11.0 + 0.6;
        lines[i].end = next.min(lines[i].t + by_chars.max(1.2));
    }
}

/// Konumdaki satır ve o satırda kaç kelimenin söylendiği (harf oranına göre tahmin).
pub fn locate(l: &Lyrics, pos: f32) -> Option<(usize, usize, usize)> {
    if !l.synced {
        return None;
    }
    let idx = l.lines.iter().rposition(|x| x.t <= pos)?;
    let line = &l.lines[idx];
    let words: Vec<&str> = line.text.split_whitespace().collect();
    let total_chars: usize = words.iter().map(|w| w.chars().count() + 1).sum();
    let span = (line.end - line.t).max(0.3);
    let frac = ((pos - line.t) / span).clamp(0.0, 1.0);
    let mut acc = 0usize;
    let mut spoken = 0;
    for w in &words {
        acc += w.chars().count() + 1;
        if (acc as f32 / total_chars as f32) <= frac + 0.02 {
            spoken += 1;
        }
    }
    Some((idx, spoken.min(words.len()), words.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lrc_parses_and_sorts() {
        let l = parse_lrc("[00:12.50] ikinci\n[00:01.00]birinci\n[ar:x]\n[01:00.00][01:30.00] tekrar");
        assert_eq!(l.len(), 4);
        assert_eq!(l[0].text, "birinci");
        assert!((l[1].t - 12.5).abs() < 0.01);
        assert_eq!(l[3].text, "tekrar");
    }

    #[test]
    fn locate_words() {
        let mut lines = vec![Line { t: 10.0, end: 14.0, text: "bir iki üç dört".into() }];
        fill_ends(&mut lines);
        let ly = Lyrics { synced: true, source: String::new(), lines };
        assert_eq!(locate(&ly, 9.0), None);
        assert_eq!(locate(&ly, 10.0).map(|x| x.1), Some(0));
        assert_eq!(locate(&ly, 14.0).map(|x| x.1), Some(4));
    }
}
