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
            if l.synced {
                return Ok(l);
            }
            // YouTube'un sözü zamanlı değil: LRCLIB'de zamanlısı varsa onu tercih et.
            return match from_lrclib(t) {
                Ok(z) if z.synced => Ok(z),
                _ => Ok(l),
            };
        }
    }
    from_lrclib(t)
}

/// Kelimenin söylenme ağırlığı: hece (ünlü) sayısı; ünlüsüz kelimeler 1 sayılır.
fn weight(w: &str) -> f32 {
    let vowels = w
        .chars()
        .filter(|c| "aeıioöuüâîûAEIİOÖUÜÂÎÛyàáäãèéêëìíòóôõùúû".contains(*c))
        .count() as f32;
    vowels.max(1.0) + 0.35
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

/// Konumdaki satır, o satırda söylenmiş kelime sayısı ve şu anki kelimenin dolumu (0..1).
/// Kelime zamanları harf sayısına göre tahmin edilir.
pub fn locate(l: &Lyrics, pos: f32) -> Option<(usize, usize, f32)> {
    if !l.synced {
        return None;
    }
    let idx = l.lines.iter().rposition(|x| x.t <= pos)?;
    let line = &l.lines[idx];
    let words: Vec<&str> = line.text.split_whitespace().collect();
    // Heceye göre dağıt: uzun kelimeler daha uzun söylenir.
    let total: f32 = words.iter().map(|w| weight(w)).sum();
    let span = (line.end - line.t).max(0.3);
    let at = ((pos - line.t) / span).clamp(0.0, 1.0) * total;
    let mut acc = 0f32;
    for (k, w) in words.iter().enumerate() {
        let len = weight(w);
        if at < acc + len {
            return Some((idx, k, ((at - acc) / len).clamp(0.0, 1.0)));
        }
        acc += len;
    }
    Some((idx, words.len(), 1.0))
}

/// Uzun müzik arası (≥ 4 sn): (arada mıyız, ilerleme 0..1). Satır bitmiş, sonraki gelmemişse.
pub fn gap(l: &Lyrics, pos: f32) -> Option<f32> {
    if !l.synced || l.lines.is_empty() {
        return None;
    }
    let next = l.lines.iter().position(|x| x.t > pos)?;
    let start = if next == 0 { 0.0 } else { l.lines[next - 1].end };
    let end = l.lines[next].t;
    if end - start < 4.0 || pos < start {
        return None;
    }
    Some(((pos - start) / (end - start)).clamp(0.0, 1.0))
}

/// Satırları çevir (Google Çeviri'nin herkese açık uç noktası, tek istekte).
/// Dönen: (kaynak dil, çeviriler). Kaynak hedefle aynıysa çeviri boş döner.
pub fn translate(lines: &[String], target: &str) -> Result<(String, Vec<String>), String> {
    let text = lines.join("\n");
    let mut resp = ureq::get("https://translate.googleapis.com/translate_a/single")
        .query("client", "gtx")
        .query("sl", "auto")
        .query("tl", target)
        .query("dt", "t")
        .query("q", &text)
        .header("User-Agent", "Mozilla/5.0")
        .call()
        .map_err(|e| format!("çeviri: {e}"))?;
    let body = resp.body_mut().read_to_string().map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let src = v.get(2).and_then(Value::as_str).unwrap_or("").to_string();
    if src == target {
        return Ok((src, Vec::new()));
    }
    let joined: String = v
        .get(0)
        .and_then(Value::as_array)
        .map(|segs| segs.iter().filter_map(|s| s.get(0).and_then(Value::as_str)).collect())
        .unwrap_or_default();
    let mut out: Vec<String> = joined.split('\n').map(|x| x.trim().to_string()).collect();
    out.resize(lines.len(), String::new());
    Ok((src, out))
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
        // Hece ağırlığı: "iki" iki heceli olduğu için satırın ortası ona düşer.
        let (_, k, f) = locate(&ly, 12.0).unwrap();
        assert!(k == 1 && f >= 0.0 && f <= 1.0);
        // satırdan önce 10 sn boşluk: arada
        assert!(gap(&ly, 5.0).is_some());
        assert!(gap(&ly, 12.0).is_none());
    }
}
