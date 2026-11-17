// Şarkı sözleri: zamanlı satırlar.
// - Unison (topluluk sözleri, https://unison.betterlyrics.org): önce sorulur; hece zamanlı
//   TTML gelirse karaoke dolumu tahmin yerine gerçek zamanlarla yapılır. Lisans gereği
//   "Lyrics from Unison" ve gönderen kişi bağlantılı gösterilir (bkz. Credit).
// - YouTube Music: ymus (önce YouTube Music'in kendi sözleri, yoksa LRCLIB)
// - Diğer kaynaklar: LRCLIB araması (süresi en yakın, zamanlı olan seçilir)

use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;

use crate::model::{Source, TrackInfo};

/// Uygulamanın kimliği (Unison ve LRCLIB istekleri için).
pub const USER_AGENT: &str = concat!("Vynz/", env!("CARGO_PKG_VERSION"), " (https://github.com/LURIDD/vynz)");
const UNISON: &str = "https://unison.betterlyrics.org";

/// Gerçek zamanlı kelime: hecelerinin (başlangıç, bitiş, harf sayısı) listesi.
#[derive(Clone, Debug)]
pub struct Word {
    pub t: f32,
    pub end: f32,
    pub syl: Vec<(f32, f32, usize)>,
}

#[derive(Clone, Debug)]
pub struct Line {
    pub t: f32,
    pub end: f32,
    pub text: String,
    /// Hece/kelime zamanlı sözlerde dolu; sayısı `text.split_whitespace()` ile aynıdır.
    pub words: Vec<Word>,
}

/// Unison sözleri için zorunlu atıf.
#[derive(Clone, Debug)]
pub struct Credit {
    /// Gönderen kişinin görünen adı
    pub user: String,
    /// https://unison.betterlyrics.org/song/<videoId>
    pub song_url: String,
    /// https://unison.betterlyrics.org/curator/<keyId>
    pub user_url: String,
}

#[derive(Clone, Debug)]
pub struct Lyrics {
    pub synced: bool,
    pub source: String,
    pub lines: Vec<Line>,
    pub credit: Option<Credit>,
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
    // Unison: zamanlıysa doğrudan; yalnız düz metinse diğer kaynaklarda zamanlısı aranır.
    let uni = match from_unison(t) {
        Ok(l) if l.synced => return Ok(l),
        Ok(l) => Some(l),
        Err(_) => None,
    };
    match fetch_others(t) {
        Ok(l) if l.synced => Ok(l),
        other => uni.map(Ok).unwrap_or(other),
    }
}

fn fetch_others(t: &TrackInfo) -> Result<Lyrics, String> {
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
        .map(|l| Line { t: l.t.unwrap_or(0.0), end: l.end.unwrap_or(0.0), text: l.text, words: Vec::new() })
        .collect();
    if lines.is_empty() {
        if let Some(text) = y.text {
            lines = text.lines().filter(|l| !l.trim().is_empty()).map(|l| Line { t: 0.0, end: 0.0, text: l.into(), words: Vec::new() }).collect();
        }
    }
    if lines.is_empty() {
        return Err("söz yok".into());
    }
    fill_ends(&mut lines);
    Ok(Lyrics { synced: y.synced, source: if y.source.is_empty() { "ymus".into() } else { y.source }, lines, credit: None })
}

fn from_lrclib(t: &TrackInfo) -> Result<Lyrics, String> {
    let artist = t.artist.split(',').next().unwrap_or("").trim();
    let mut resp = ureq::get("https://lrclib.net/api/search")
        .query("track_name", &t.title)
        .query("artist_name", artist)
        .header("User-Agent", USER_AGENT)
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
            return Ok(Lyrics { synced: true, source: "LRCLIB".into(), lines, credit: None });
        }
    }
    let plain = best.get("plainLyrics").and_then(Value::as_str).ok_or("söz bulunamadı")?;
    let lines: Vec<Line> = plain.lines().filter(|l| !l.trim().is_empty()).map(|l| Line { t: 0.0, end: 0.0, text: l.into(), words: Vec::new() }).collect();
    if lines.is_empty() {
        return Err("söz bulunamadı".into());
    }
    Ok(Lyrics { synced: false, source: "LRCLIB".into(), lines, credit: None })
}

fn unison_cache_dir() -> PathBuf {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or(home.join(".cache")).join("vynz").join("unison")
}

/// Unison'dan sözü getir. YouTube kimliği varsa onunla, yoksa şarkı+sanatçı+süre ile.
/// Yanıt ETag'iyle diske yazılır; sonraki istekte If-None-Match gönderilir (304 = önbellek).
/// 404 gelirse önbellek silinir (Unison'dan kaldırılan söz bizde de kalmasın).
fn from_unison(t: &TrackInfo) -> Result<Lyrics, String> {
    let (key, query): (String, Vec<(&str, String)>) = if t.source == Source::Ytm && !t.id.is_empty() {
        (format!("v-{}", t.id), vec![("v", t.id.clone())])
    } else {
        let artist = t.artist.split(',').next().unwrap_or("").trim().to_string();
        let mut q = vec![("song", t.title.clone()), ("artist", artist)];
        if t.duration > 0.0 {
            q.push(("duration", format!("{}", t.duration.round() as i64)));
        }
        let k: String = format!("q-{}-{}", t.title, t.artist).chars().map(|c| if c.is_alphanumeric() { c } else { '_' }).take(120).collect();
        (k, q)
    };
    let dir = unison_cache_dir();
    let path = dir.join(format!("{key}.json"));
    let cached: Option<Value> = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok());
    let etag = cached.as_ref().and_then(|c| c.get("etag")).and_then(Value::as_str).map(str::to_string);

    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(8)))
        .build()
        .into();
    let mut req = agent.get(format!("{UNISON}/lyrics")).header("User-Agent", USER_AGENT);
    for (k, v) in &query {
        req = req.query(*k, v);
    }
    if let Some(e) = &etag {
        req = req.header("If-None-Match", e);
    }
    let data: Value = match req.call() {
        Ok(mut resp) => match resp.status().as_u16() {
            304 => cached.as_ref().and_then(|c| c.get("data")).cloned().ok_or("önbellek bozuk")?,
            200 => {
                let new_etag = resp.headers().get("etag").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
                let body = resp.body_mut().with_config().limit(4 * 1024 * 1024).read_to_string().map_err(|e| e.to_string())?;
                let v: Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
                let data = v.get("data").cloned().ok_or("Unison: veri yok")?;
                // Yalnız herkese aynı dönen ("primary") yanıt saklanır.
                let variant = data.get("variant").and_then(Value::as_str).unwrap_or("primary");
                if variant == "primary" && !new_etag.is_empty() {
                    let _ = std::fs::create_dir_all(&dir);
                    let _ = std::fs::write(&path, serde_json::json!({"etag": new_etag, "data": data}).to_string());
                }
                data
            }
            404 => {
                let _ = std::fs::remove_file(&path);
                return Err("Unison: söz yok".into());
            }
            code => {
                // Hız sınırı ya da geçici hata: elde kopya varsa onu kullan.
                return cached.and_then(|c| c.get("data").cloned()).map(|d| unison_lyrics(&d, t)).unwrap_or(Err(format!("Unison: HTTP {code}")));
            }
        },
        Err(e) => {
            return cached.and_then(|c| c.get("data").cloned()).map(|d| unison_lyrics(&d, t)).unwrap_or(Err(format!("Unison: {e}")));
        }
    };
    unison_lyrics(&data, t)
}

fn unison_lyrics(d: &Value, t: &TrackInfo) -> Result<Lyrics, String> {
    if d.get("hidden").and_then(Value::as_bool) == Some(true) {
        return Err("Unison: gizlenmiş söz".into());
    }
    let text = d.get("lyrics").and_then(Value::as_str).ok_or("Unison: söz alanı yok")?;
    let format = d.get("format").and_then(Value::as_str).unwrap_or("");
    let video = d.get("videoId").and_then(Value::as_str).filter(|v| !v.is_empty()).unwrap_or(&t.id).to_string();
    let sub = d.get("submitter");
    let credit = Some(Credit {
        user: sub.and_then(|s| s.get("displayName")).and_then(Value::as_str).unwrap_or("").to_string(),
        song_url: format!("{UNISON}/song/{video}"),
        user_url: sub.and_then(|s| s.get("keyId")).and_then(Value::as_str).map(|k| format!("{UNISON}/curator/{k}")).unwrap_or_default(),
    });
    let mut lines = match format {
        "ttml" => parse_ttml(text)?,
        "lrc" => parse_lrc(text),
        _ => text.lines().filter(|l| !l.trim().is_empty()).map(|l| Line { t: 0.0, end: 0.0, text: l.trim().into(), words: Vec::new() }).collect(),
    };
    if lines.is_empty() {
        return Err("Unison: boş söz".into());
    }
    let synced = format != "plain" && lines.iter().any(|l| l.t > 0.0);
    if synced {
        fill_ends(&mut lines);
    }
    Ok(Lyrics { synced, source: "Unison".into(), lines, credit })
}

/// TTML zamanı: "2:30.140", "1:02:30.1", "00:02:30.140", "150.14s" ya da "150.14".
fn ttml_time(s: &str) -> Option<f32> {
    let s = s.trim();
    if let Some(x) = s.strip_suffix("ms") {
        return x.parse::<f32>().ok().map(|v| v / 1000.0);
    }
    if let Some(x) = s.strip_suffix('s') {
        return x.parse().ok();
    }
    let mut acc = 0f32;
    for part in s.split(':') {
        acc = acc * 60.0 + part.parse::<f32>().ok()?;
    }
    Some(acc)
}

/// TTML'den satırlar: her <p> bir satır; içindeki <span>'ler hece/kelime.
/// Aralarında boşluk olmayan ardışık span'ler aynı kelimenin heceleridir.
/// Arka vokal, çeviri ve okunuş (ttm:role'lü span'ler) atlanır.
pub fn parse_ttml(src: &str) -> Result<Vec<Line>, String> {
    let doc = roxmltree::Document::parse(src).map_err(|e| format!("TTML: {e}"))?;
    let ttm = "http://www.w3.org/ns/ttml#metadata";
    let mut out = Vec::new();
    for p in doc.descendants().filter(|n| n.has_tag_name("p")) {
        let begin = p.attribute("begin").and_then(ttml_time);
        let end = p.attribute("end").and_then(ttml_time);
        // (metin parçası, zaman) dizisi; zaman None = boşluk/düz metin
        let mut words: Vec<Word> = Vec::new();
        let mut texts: Vec<String> = Vec::new();
        let mut joined = false; // sonraki span önceki kelimeye eklensin mi
        for c in p.children() {
            if c.is_text() {
                let tx = c.text().unwrap_or("");
                if tx.chars().any(char::is_whitespace) {
                    joined = false;
                }
                continue;
            }
            if !c.has_tag_name("span") || c.attribute((ttm, "role")).is_some() {
                continue;
            }
            let tx: String = c.descendants().filter(|n| n.is_text()).filter_map(|n| n.text()).collect();
            let (Some(a), Some(b)) = (c.attribute("begin").and_then(ttml_time), c.attribute("end").and_then(ttml_time)) else { continue };
            let lead_space = tx.starts_with(char::is_whitespace);
            let trail_space = tx.ends_with(char::is_whitespace);
            let pieces: Vec<&str> = tx.split_whitespace().collect();
            if pieces.is_empty() {
                joined = false;
                continue;
            }
            // Span içinde birden çok kelime varsa süre harf payına göre bölünür.
            let total: usize = pieces.iter().map(|x| x.chars().count()).sum::<usize>().max(1);
            let mut at = a;
            for (i, piece) in pieces.iter().enumerate() {
                let n = piece.chars().count();
                let dur = (b - a) * n as f32 / total as f32;
                let (sa, sb) = (at, at + dur);
                at = sb;
                if i == 0 && joined && !lead_space && !words.is_empty() {
                    let w = words.last_mut().unwrap();
                    w.end = sb;
                    w.syl.push((sa, sb, n));
                    texts.last_mut().unwrap().push_str(piece);
                } else {
                    words.push(Word { t: sa, end: sb, syl: vec![(sa, sb, n)] });
                    texts.push((*piece).to_string());
                }
            }
            joined = !trail_space;
        }
        let (t, e) = match (begin, words.first(), words.last()) {
            (Some(bg), _, _) => (bg, end.unwrap_or(words.last().map(|w| w.end).unwrap_or(bg))),
            (None, Some(f), Some(l)) => (f.t, l.end),
            _ => {
                // Zamansız düz satır
                let tx: String = p.descendants().filter(|n| n.is_text()).filter_map(|n| n.text()).collect();
                let tx = tx.split_whitespace().collect::<Vec<_>>().join(" ");
                if !tx.is_empty() {
                    out.push(Line { t: 0.0, end: 0.0, text: tx, words: Vec::new() });
                }
                continue;
            }
        };
        let text = if texts.is_empty() {
            let tx: String = p.descendants().filter(|n| n.is_text()).filter_map(|n| n.text()).collect();
            tx.split_whitespace().collect::<Vec<_>>().join(" ")
        } else {
            texts.join(" ")
        };
        if text.is_empty() {
            continue;
        }
        out.push(Line { t, end: e, text, words });
    }
    out.sort_by(|a, b| a.t.total_cmp(&b.t));
    Ok(out)
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
            out.push(Line { t, end: 0.0, text: rest.to_string(), words: Vec::new() });
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
    if !line.words.is_empty() {
        return Some((idx, timed_word(&line.words, pos).0, timed_word(&line.words, pos).1));
    }
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

/// Gerçek zamanlı kelimelerde: (söylenmiş kelime sayısı, şu anki kelimenin dolumu).
/// Dolum heceye göre: hece içinde doğrusal, hece harf sayısıyla ağırlıklı.
fn timed_word(words: &[Word], pos: f32) -> (usize, f32) {
    for (k, w) in words.iter().enumerate() {
        if pos < w.t {
            return (k, 0.0); // kelimeler arası boşluk: sıradaki henüz başlamadı
        }
        if pos < w.end {
            let total: usize = w.syl.iter().map(|s| s.2).sum::<usize>().max(1);
            let mut done = 0usize;
            for &(a, b, n) in &w.syl {
                if pos < b {
                    let f = if b > a { ((pos - a) / (b - a)).clamp(0.0, 1.0) } else { 1.0 };
                    return (k, (done as f32 + f * n as f32) / total as f32);
                }
                done += n;
            }
            return (k, 1.0);
        }
    }
    (words.len(), 1.0)
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
    fn ttml_syllables_join_into_words() {
        let src = r#"<tt xmlns="http://www.w3.org/ns/ttml" xmlns:ttm="http://www.w3.org/ns/ttml#metadata"><body><div>
<p begin="2:33.757" end="2:37.159"><span begin="2:33.757" end="2:33.963">No</span><span begin="2:33.963" end="2:34.254">thing</span> <span begin="2:34.254" end="2:34.529">but</span> <span ttm:role="x-bg"><span begin="2:34.3" end="2:34.4">(ooh)</span></span> <span begin="2:34.529" end="2:34.924">plain</span></p>
<p begin="10.5s" end="12s"><span begin="10.5s" end="12s">iki kelime</span></p>
</div></body></tt>"#;
        let l = parse_ttml(src).unwrap();
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].text, "iki kelime"); // 10.5 sn: sıralamada önce
        assert_eq!(l[1].text, "Nothing but plain");
        assert_eq!(l[1].words.len(), 3);
        assert_eq!(l[1].words[0].syl.len(), 2);
        assert!((l[1].t - 153.757).abs() < 0.001);
        assert_eq!(l[0].words.len(), 2);
        let ly = Lyrics { synced: true, source: String::new(), lines: l, credit: None };
        // "No" hecesinin ortası: ilk kelime, dolum < yarım (No = 2/7 harf)
        let (i, k, f) = locate(&ly, 153.86).unwrap();
        assert_eq!((i, k), (1, 0));
        assert!(f > 0.1 && f < 0.2, "{f}");
        // "but" bitti, "plain" başlamadı değil: plain 154.529'da başlar
        let (_, k, _) = locate(&ly, 154.6).unwrap();
        assert_eq!(k, 2);
        let (_, k, f) = locate(&ly, 160.0).unwrap();
        assert_eq!((k, f), (3, 1.0));
    }

    #[test]
    fn locate_words() {
        let mut lines = vec![Line { t: 10.0, end: 14.0, text: "bir iki üç dört".into(), words: Vec::new() }];
        fill_ends(&mut lines);
        let ly = Lyrics { synced: true, source: String::new(), lines, credit: None };
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
