// Liste aktarma: bağlantıdan şarkı listesi okuma ve Spotify şarkılarını
// çalınabilir kaynaklarla (YouTube Music, gerekirse SoundCloud) eşleştirme.
//
// Desteklenen bağlantılar (giriş gerekmez):
// - Spotify liste/albüm/şarkı: herkese açık gömülü sayfadan (en fazla 100 şarkı)
// - YouTube / YouTube Music liste ve albüm: yt-dlp ile tamamı
// - SoundCloud set/liste: yt-dlp ile tamamı

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use serde_json::Value;

use crate::model::{Source, TrackInfo};
use crate::resolve::{Prio, Resolver};
use crate::sources::Provider;

/// Kaynak listedeki bir şarkı (henüz çalınabilir değil).
#[derive(Clone, Debug)]
pub struct Wanted {
    pub title: String,
    pub artist: String,
    pub duration: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Matched = 0,
    Uncertain = 1,
    Missing = 2,
    /// Kaynağın kendisi çalınabilir (eşleştirme gerekmedi).
    Direct = 3,
}

#[derive(Clone, Debug)]
pub struct Row {
    pub wanted: Wanted,
    /// Puana göre sıralı adaylar.
    pub candidates: Vec<(TrackInfo, f32)>,
    pub chosen: usize,
    pub status: Status,
}

impl Row {
    pub fn pick(&self) -> Option<&TrackInfo> {
        self.candidates.get(self.chosen).map(|c| &c.0)
    }
    pub fn score(&self) -> f32 {
        self.candidates.get(self.chosen).map(|c| c.1).unwrap_or(0.0)
    }
}

pub struct Fetched {
    pub name: String,
    pub source_label: &'static str,
    pub rows: Vec<Row>,
    /// Kaynak gömülü sayfa sınırına takıldıysa (Spotify 100) bilgi.
    pub truncated: bool,
}

pub enum Link {
    Spotify { kind: String, id: String },
    YouTube(String),
    SoundCloud(String),
}

pub fn detect(url: &str) -> Option<Link> {
    let u = url.trim();
    if let Some(rest) = u.split("open.spotify.com/").nth(1) {
        // Örnekler: playlist/ID?si=..  intl-tr/album/ID  embed/track/ID
        let path = rest.split(['?', '#']).next().unwrap_or("");
        let mut parts = path.split('/').filter(|p| !p.is_empty() && !p.starts_with("intl-") && *p != "embed");
        let kind = parts.next()?.to_string();
        let id = parts.next()?.to_string();
        if matches!(kind.as_str(), "playlist" | "album" | "track") {
            return Some(Link::Spotify { kind, id });
        }
        return None;
    }
    if u.contains("youtube.com/") || u.contains("youtu.be/") {
        return Some(Link::YouTube(u.replace("://youtube.com", "://www.youtube.com")));
    }
    if u.contains("soundcloud.com/") {
        return Some(Link::SoundCloud(u.to_string()));
    }
    None
}

// ---------- Okuma ----------

fn spotify(kind: &str, id: &str) -> Result<(String, Vec<Wanted>, bool), String> {
    let url = format!("https://open.spotify.com/embed/{kind}/{id}");
    let mut resp = ureq::get(&url)
        .header("User-Agent", "Mozilla/5.0 (X11; Linux x86_64; rv:140.0) Gecko/20100101 Firefox/140.0")
        .call()
        .map_err(|e| format!("Spotify sayfası açılamadı: {e}"))?;
    let html = resp.body_mut().read_to_string().map_err(|e| e.to_string())?;
    let start_tag = "<script id=\"__NEXT_DATA__\" type=\"application/json\">";
    let a = html.find(start_tag).ok_or("Spotify sayfa biçimi değişmiş (veri yok); liste gizli olabilir")? + start_tag.len();
    let b = html[a..].find("</script>").ok_or("Spotify verisi eksik")? + a;
    let data: Value = serde_json::from_str(&html[a..b]).map_err(|e| e.to_string())?;
    let entity = find_key(&data, "trackList").ok_or("Liste boş ya da gizli")?;
    let name = entity
        .get("name")
        .or_else(|| entity.get("title"))
        .and_then(Value::as_str)
        .unwrap_or("Spotify listesi")
        .to_string();
    let items: Vec<Wanted> = entity["trackList"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|t| {
                    Some(Wanted {
                        title: t.get("title")?.as_str()?.to_string(),
                        artist: t.get("subtitle").and_then(Value::as_str).unwrap_or("").replace('\u{a0}', " "),
                        duration: t.get("duration").and_then(Value::as_f64).unwrap_or(0.0) as f32 / 1000.0,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let truncated = items.len() >= 100 && kind == "playlist";
    Ok((name, items, truncated))
}

fn find_key<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    match v {
        Value::Object(m) => {
            if m.contains_key(key) {
                return Some(v);
            }
            m.values().find_map(|x| find_key(x, key))
        }
        Value::Array(a) => a.iter().find_map(|x| find_key(x, key)),
        _ => None,
    }
}

/// YouTube/SoundCloud listesi: sıcak çözücü yardımcısıyla (yt-dlp) düz liste.
fn flat_list(url: &str, source: Source, resolver: &Resolver) -> Result<(String, Vec<TrackInfo>), String> {
    let d = resolver.request_sync(serde_json::json!({"op": "flat", "url": url}), Prio::Soon)?;
    let name = d.get("title").and_then(Value::as_str).unwrap_or("Aktarılan liste").to_string();
    let entries = d.get("entries").and_then(Value::as_array).cloned().unwrap_or_default();
    let tracks = entries
        .iter()
        .filter_map(|e| {
            let id = e.get("id")?.as_str()?.to_string();
            let title = e.get("title").and_then(Value::as_str).unwrap_or("").to_string();
            let uploader = e
                .get("artists")
                .and_then(Value::as_array)
                .and_then(|a| a.first())
                .and_then(Value::as_str)
                .or_else(|| e.get("channel").and_then(Value::as_str))
                .or_else(|| e.get("uploader").and_then(Value::as_str))
                .unwrap_or("")
                .trim_end_matches(" - Topic")
                .trim_end_matches(" Official")
                .to_string();
            let duration = e.get("duration").and_then(Value::as_f64).unwrap_or(0.0) as f32;
            match source {
                Source::Ytm => Some(TrackInfo {
                    source,
                    title,
                    artist: uploader,
                    album: String::new(),
                    duration,
                    thumb: format!("https://i.ytimg.com/vi/{id}/hqdefault.jpg"),
                    url: None,
                    id,
                }),
                _ => {
                    let page = e.get("url").and_then(Value::as_str).map(str::to_string)?;
                    let thumb = e
                        .get("thumbnails")
                        .and_then(Value::as_array)
                        .and_then(|a| a.last())
                        .and_then(|t| t.get("url"))
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    let (title, artist) = split_dash(&title, &uploader);
                    Some(TrackInfo { source, id, title, artist, album: String::new(), duration, thumb, url: Some(page) })
                }
            }
        })
        .collect();
    Ok((name, tracks))
}

fn split_dash(title: &str, uploader: &str) -> (String, String) {
    for sep in [" - ", " – ", " — "] {
        if let Some((a, t)) = title.split_once(sep) {
            if !a.trim().is_empty() && !t.trim().is_empty() {
                return (t.trim().to_string(), a.trim().to_string());
            }
        }
    }
    (title.to_string(), uploader.to_string())
}

// ---------- Eşleştirme ----------

fn fold(c: char) -> char {
    match c {
        'ç' => 'c',
        'ğ' => 'g',
        'ı' | 'î' | 'í' | 'ì' => 'i',
        'ö' | 'ó' | 'ò' | 'ô' => 'o',
        'ş' => 's',
        'ü' | 'ú' | 'ù' | 'û' => 'u',
        'â' | 'á' | 'à' | 'ä' | 'ã' => 'a',
        'é' | 'è' | 'ê' | 'ë' => 'e',
        'ñ' => 'n',
        _ => c,
    }
}

/// Küçük harf, aksansız; parantez içleri ve "- Remastered" gibi ekler atılır.
pub fn norm(s: &str) -> String {
    let lower: String = s.to_lowercase().chars().map(fold).collect();
    let mut out = String::new();
    let mut depth = 0;
    for c in lower.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = (depth - 1).max(0),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    let out = out.split(" - ").next().unwrap_or(&out).to_string();
    out.chars().map(|c| if c.is_alphanumeric() { c } else { ' ' }).collect::<String>()
}

fn tokens(s: &str) -> Vec<String> {
    norm(s).split_whitespace().filter(|w| !matches!(*w, "feat" | "ft" | "the" | "and" | "ve")).map(str::to_string).collect()
}

fn overlap(a: &[String], b: &[String]) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let hit = a.iter().filter(|w| b.contains(w)).count() as f32;
    // İki yönlü: kısa olanın kapsanması ağırlıklı.
    let p = hit / a.len() as f32;
    let r = hit / b.len() as f32;
    0.65 * p.max(r) + 0.35 * p.min(r)
}

const VERSION_WORDS: [&str; 10] = ["live", "canli", "remix", "cover", "karaoke", "instrumental", "slowed", "sped", "reverb", "acoustic"];

pub fn score(w: &Wanted, c: &TrackInfo) -> f32 {
    let wt = tokens(&w.title);
    let ct = tokens(&c.title);
    let title = overlap(&wt, &ct);
    let wa = tokens(&w.artist);
    // Aday başlığı "Sanatçı - Şarkı" biçimindeyse sanatçı başlıkta olabilir.
    let mut ca = tokens(&c.artist);
    ca.extend(tokens(&c.title));
    // Spotify birden çok sanatçı verir ("A, B, C"); asıl sanatçının bulunması yeterli.
    let cover = |ws: &[String]| if ws.is_empty() { 0.0 } else { ws.iter().filter(|x| ca.contains(x)).count() as f32 / ws.len() as f32 };
    let first = tokens(w.artist.split(',').next().unwrap_or(""));
    let artist = if wa.is_empty() { 0.5 } else { cover(&wa).max(cover(&first)) };
    let dur = if w.duration <= 0.0 || c.duration <= 0.0 {
        0.6
    } else {
        let d = (w.duration - c.duration).abs();
        if d <= 3.0 { 1.0 } else if d <= 10.0 { 0.75 } else if d <= 30.0 { 0.35 } else { 0.0 }
    };
    let mut s = 0.5 * title + 0.3 * artist + 0.2 * dur;
    // Asılda olmayan sürüm kelimeleri (canlı, remix...) cezalandırılır.
    let raw_w = norm(&w.title);
    let raw_c = norm(&c.title);
    for v in VERSION_WORDS {
        if raw_c.split_whitespace().any(|x| x == v) && !raw_w.split_whitespace().any(|x| x == v) {
            s -= 0.2;
        }
    }
    s.clamp(0.0, 1.0)
}

fn classify(score: f32) -> Status {
    if score >= 0.78 {
        Status::Matched
    } else if score >= 0.5 {
        Status::Uncertain
    } else {
        Status::Missing
    }
}

fn match_one(w: &Wanted, ytm: &dyn Provider, sc: Option<&dyn Provider>) -> Row {
    let q = format!("{} {}", w.title, w.artist.split(',').next().unwrap_or(""));
    let mut cands: Vec<(TrackInfo, f32)> = ytm.search(&q, 6).unwrap_or_default().into_iter().map(|t| { let s = score(w, &t); (t, s) }).collect();
    cands.sort_by(|a, b| b.1.total_cmp(&a.1));
    // YouTube Music'te iyi aday yoksa SoundCloud'a da bak.
    if cands.first().map(|c| c.1).unwrap_or(0.0) < 0.5 {
        if let Some(sc) = sc {
            let mut more: Vec<(TrackInfo, f32)> = sc.search(&q, 5).unwrap_or_default().into_iter().map(|t| { let s = score(w, &t); (t, s) }).collect();
            cands.append(&mut more);
            cands.sort_by(|a, b| b.1.total_cmp(&a.1));
        }
    }
    cands.truncate(5);
    let status = classify(cands.first().map(|c| c.1).unwrap_or(0.0));
    Row { wanted: w.clone(), candidates: cands, chosen: 0, status }
}

/// Bağlantıyı oku; Spotify ise eşleştir. `progress(bitti, toplam)` iş parçacığından çağrılır.
pub fn run(
    url: &str,
    ytm: Arc<dyn Provider>,
    sc: Arc<dyn Provider>,
    resolver: &Resolver,
    progress: impl Fn(usize, usize) + Send + Sync,
) -> Result<Fetched, String> {
    match detect(url).ok_or("Bu bağlantı tanınmadı. Spotify, YouTube Music ya da SoundCloud liste bağlantısı yapıştır.")? {
        Link::YouTube(u) => {
            let u = if u.contains("list=") && !u.contains("/playlist") {
                // izleme bağlantısındaki listeyi aç
                let list = u.split("list=").nth(1).unwrap_or("").split('&').next().unwrap_or("");
                format!("https://music.youtube.com/playlist?list={list}")
            } else {
                u
            };
            let (name, tracks) = flat_list(&u, Source::Ytm, resolver)?;
            Ok(direct(name, "YouTube Music", tracks))
        }
        Link::SoundCloud(u) => {
            let (name, tracks) = flat_list(&u, Source::SoundCloud, resolver)?;
            Ok(direct(name, "SoundCloud", tracks))
        }
        Link::Spotify { kind, id } => {
            let (name, wanted, truncated) = spotify(&kind, &id)?;
            Ok(match_all(name, "Spotify", wanted, truncated, ytm, sc, progress))
        }
    }
}

/// Kaynak listedeki şarkıları paralel eşleştir (4 iş parçacığı).
pub fn match_all(
    name: String,
    label: &'static str,
    wanted: Vec<Wanted>,
    truncated: bool,
    ytm: Arc<dyn Provider>,
    sc: Arc<dyn Provider>,
    progress: impl Fn(usize, usize) + Send + Sync,
) -> Fetched {
    let total = wanted.len();
    progress(0, total);
    let rows: Vec<Mutex<Option<Row>>> = (0..total).map(|_| Mutex::new(None)).collect();
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    thread::scope(|s| {
        for _ in 0..4 {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= total {
                    break;
                }
                let row = match_one(&wanted[i], ytm.as_ref(), Some(sc.as_ref()));
                *rows[i].lock().unwrap() = Some(row);
                let d = done.fetch_add(1, Ordering::Relaxed) + 1;
                progress(d, total);
            });
        }
    });
    let rows = rows.into_iter().filter_map(|m| m.into_inner().ok().flatten()).collect();
    Fetched { name, source_label: label, rows, truncated }
}

// ---------- Dosyadan aktarma ----------

/// Dosyadaki bir liste (eşleştirilmemiş).
#[derive(Clone, Debug)]
pub struct FileList {
    pub name: String,
    pub tracks: Vec<Wanted>,
}

/// Spotify veri dışa aktarımı (Playlist*.json, YourLibrary.json) ya da CSV.
pub fn read_file(path: &str) -> Result<Vec<FileList>, String> {
    let path = path.trim().trim_start_matches("file://");
    let path = crate::paths::expand_tilde(path);
    let raw = std::fs::read(&path).map_err(|e| format!("Dosya okunamadı: {e}"))?;
    let text = String::from_utf8_lossy(&raw);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Liste").to_string();
    let is_json = path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("json"));
    let lists = if is_json { from_json(&text)? } else { from_csv(&text, &stem)? };
    let lists: Vec<FileList> = lists.into_iter().filter(|l| !l.tracks.is_empty()).collect();
    if lists.is_empty() {
        return Err("Dosyada şarkı bulunamadı. Spotify dışa aktarımındaki Playlist/YourLibrary JSON dosyalarını ya da bir CSV seç.".into());
    }
    Ok(lists)
}

fn from_json(text: &str) -> Result<Vec<FileList>, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("JSON okunamadı: {e}"))?;
    let mut out = Vec::new();
    // Playlist1.json: {"playlists":[{"name":..,"items":[{"track":{"trackName","artistName","albumName"}}]}]}
    if let Some(pls) = v.get("playlists").and_then(Value::as_array) {
        for p in pls {
            let name = p.get("name").and_then(Value::as_str).unwrap_or("Liste").to_string();
            let tracks = p
                .get("items")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|it| {
                            let t = it.get("track")?;
                            Some(Wanted {
                                title: t.get("trackName")?.as_str()?.to_string(),
                                artist: t.get("artistName").and_then(Value::as_str).unwrap_or("").to_string(),
                                duration: 0.0,
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            out.push(FileList { name, tracks });
        }
    }
    // YourLibrary.json: {"tracks":[{"artist","album","track","uri"}]}
    if let Some(ts) = v.get("tracks").and_then(Value::as_array) {
        let tracks = ts
            .iter()
            .filter_map(|t| {
                Some(Wanted {
                    title: t.get("track")?.as_str()?.to_string(),
                    artist: t.get("artist").and_then(Value::as_str).unwrap_or("").to_string(),
                    duration: 0.0,
                })
            })
            .collect();
        out.push(FileList { name: "Beğenilen Şarkılar (Spotify)".into(), tracks });
    }
    if out.is_empty() {
        return Err("Bu JSON dosyası tanınmadı (Spotify Playlist ya da YourLibrary dosyası değil).".into());
    }
    Ok(out)
}

/// Tırnaklı alanları ve kaçışlı tırnakları destekleyen basit CSV ayrıştırıcı.
fn csv_rows(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.trim_start_matches('\u{feff}').chars().peekable();
    while let Some(c) = chars.next() {
        match (c, quoted) {
            ('"', true) if chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            ('"', true) => quoted = false,
            ('"', false) if field.is_empty() => quoted = true,
            (',', false) | (';', false) => row.push(std::mem::take(&mut field)),
            ('\n', false) => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            ('\r', false) => {}
            _ => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

fn from_csv(text: &str, name: &str) -> Result<Vec<FileList>, String> {
    let rows = csv_rows(text);
    let header: Vec<String> = rows.first().ok_or("CSV boş")?.iter().map(|h| h.trim().to_lowercase()).collect();
    let find = |names: &[&str]| header.iter().position(|h| names.iter().any(|n| h == n));
    let ti = find(&["track name", "track", "title", "name", "song", "şarkı", "şarkı adı"]).ok_or("CSV'de şarkı adı sütunu bulunamadı")?;
    let ai = find(&["artist name(s)", "artist name", "artist", "artists", "sanatçı"]);
    let di = find(&["duration (ms)", "duration_ms", "duration", "süre"]);
    let tracks = rows
        .iter()
        .skip(1)
        .filter_map(|r| {
            let title = r.get(ti)?.trim().to_string();
            if title.is_empty() {
                return None;
            }
            let artist = ai.and_then(|i| r.get(i)).map(|a| a.trim().to_string()).unwrap_or_default();
            let d = di.and_then(|i| r.get(i)).and_then(|d| d.trim().parse::<f32>().ok()).unwrap_or(0.0);
            Some(Wanted { title, artist, duration: if d > 10_000.0 { d / 1000.0 } else { d } })
        })
        .collect();
    Ok(vec![FileList { name: name.to_string(), tracks }])
}

fn direct(name: String, label: &'static str, tracks: Vec<TrackInfo>) -> Fetched {
    let rows = tracks
        .into_iter()
        .map(|t| Row {
            wanted: Wanted { title: t.title.clone(), artist: t.artist.clone(), duration: t.duration },
            candidates: vec![(t, 1.0)],
            chosen: 0,
            status: Status::Direct,
        })
        .collect();
    Fetched { name, source_label: label, rows, truncated: false }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(title: &str, artist: &str, d: f32) -> TrackInfo {
        TrackInfo { source: Source::Ytm, id: "x".into(), title: title.into(), artist: artist.into(), album: String::new(), duration: d, thumb: String::new(), url: None }
    }

    #[test]
    fn exact_match_scores_high() {
        let w = Wanted { title: "Cemalim".into(), artist: "Erkin Koray".into(), duration: 461.0 };
        assert!(score(&w, &t("Cemalim", "Erkin Koray", 461.0)) > 0.9);
    }

    #[test]
    fn turkish_and_brackets_fold() {
        let w = Wanted { title: "Gamsız Hayat (2023 Remaster)".into(), artist: "Candan Erçetin".into(), duration: 200.0 };
        assert!(score(&w, &t("Gamsiz Hayat", "Candan Ercetin", 201.0)) > 0.9);
    }

    #[test]
    fn live_version_penalized() {
        let w = Wanted { title: "Dönence".into(), artist: "Barış Manço".into(), duration: 308.0 };
        let studio = score(&w, &t("Dönence", "Barış Manço", 306.0));
        let live = score(&w, &t("Dönence (Live)", "Barış Manço", 372.0));
        let live2 = score(&w, &t("Dönence Live 1982", "Barış Manço", 372.0));
        assert!(studio > live && studio > live2 + 0.3);
    }

    #[test]
    fn primary_artist_is_enough() {
        let w = Wanted { title: "Sweet Dreams (Are Made of This) - 2005 Remaster".into(), artist: "Eurythmics, Annie Lennox, Dave Stewart".into(), duration: 217.0 };
        assert_eq!(classify(score(&w, &t("Sweet Dreams (Are Made of This) (2005 Remaster)", "Eurythmics", 217.0))), Status::Matched);
    }

    #[test]
    fn unrelated_is_missing() {
        let w = Wanted { title: "Housewife".into(), artist: "Tate McRae".into(), duration: 208.0 };
        assert_eq!(classify(score(&w, &t("Bir Derdim Var", "Mor ve Ötesi", 214.0))), Status::Missing);
    }

    #[test]
    fn csv_exportify_and_quotes() {
        let csv = "\u{feff}Track URI,Track Name,Artist Name(s),Album Name,Duration (ms)\nspotify:x,\"Dönence\",\"Barış Manço\",Sözüm Meclisten Dışarı,308000\nspotify:y,\"Hello, \"\"World\"\"\",A,B,1000\n";
        let l = from_csv(csv, "x").unwrap();
        assert_eq!(l[0].tracks.len(), 2);
        assert_eq!(l[0].tracks[0].title, "Dönence");
        assert!((l[0].tracks[0].duration - 308.0).abs() < 0.1);
        assert_eq!(l[0].tracks[1].title, "Hello, \"World\"");
    }

    #[test]
    fn spotify_export_json() {
        let pl = r#"{"playlists":[{"name":"Yol","items":[{"track":{"trackName":"Cemalim","artistName":"Erkin Koray","albumName":"x","trackUri":"u"}},{"episode":null}]}]}"#;
        let l = from_json(pl).unwrap();
        assert_eq!(l[0].name, "Yol");
        assert_eq!(l[0].tracks.len(), 1);
        let lib = r#"{"tracks":[{"artist":"BLOK3","album":"a","track":"Sebebi Yar","uri":"u"}]}"#;
        assert_eq!(from_json(lib).unwrap()[0].tracks[0].artist, "BLOK3");
    }

    #[test]
    fn detects_links() {
        assert!(matches!(detect("https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M?si=abc"), Some(Link::Spotify { .. })));
        assert!(matches!(detect("https://open.spotify.com/intl-tr/album/1A2B3C"), Some(Link::Spotify { kind, .. }) if kind == "album"));
        assert!(matches!(detect("https://music.youtube.com/playlist?list=PL123"), Some(Link::YouTube(_))));
        assert!(matches!(detect("https://soundcloud.com/a/sets/b"), Some(Link::SoundCloud(_))));
        assert!(detect("https://example.com").is_none());
    }
}

#[cfg(test)]
mod live {
    use super::*;
    use crate::resolve::Resolver;
    use crate::sources::{soundcloud::SoundCloud, ytm::Ytm};

    /// Ağa çıkar: `cargo test live_spotify -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_spotify() {
        let url = std::env::var("VYNZ_TEST_URL").unwrap_or("https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M".into());
        let t0 = std::time::Instant::now();
        let resolver = Resolver::new(None);
        let f = run(&url, Arc::new(Ytm::new(resolver.clone())), Arc::new(SoundCloud::new(resolver.clone())), &resolver, |_, _| {}).unwrap();
        let count = |s: Status| f.rows.iter().filter(|r| r.status == s).count();
        println!("{} | {} şarkı | {:.1} sn | eşleşti {} · emin değil {} · yok {}", f.name, f.rows.len(), t0.elapsed().as_secs_f64(),
            count(Status::Matched), count(Status::Uncertain), count(Status::Missing));
        for r in f.rows.iter().filter(|r| r.status != Status::Matched).take(12) {
            let p = r.pick().map(|t| format!("{} — {} ({:.0}s)", t.title, t.artist, t.duration)).unwrap_or("-".into());
            println!("  [{:?} {:.2}] {} — {} ({:.0}s)  =>  {}", r.status, r.score(), r.wanted.title, r.wanted.artist, r.wanted.duration, p);
        }
    }
}
