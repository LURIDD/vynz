// YouTube ses adresini yardımcı süreç olmadan, doğrudan InnerTube ile çözer.
//
// visionOS istemcisi imzasız, n-parametresiz ve PO token gerektirmeyen adresler veriyor.
// Tek şart bir visitorData: yoksa ilk yanıt LOGIN_REQUIRED döner ama içinde yenisi gelir,
// onu saklayıp yeniden deneriz. Adresler Range başlığı olmadan istenirse yavaşlatılır
// (mpv/ffmpeg ve fetcher zaten Range gönderiyor).
//
// Başarısız olursa çağıran taraf yt-dlp yardımcısına düşer.

use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::resolve::Resolved;

const PLAYER_URL: &str = "https://www.youtube.com/youtubei/v1/player?prettyPrint=false";
const CLIENT_NAME: &str = "VISIONOS";
const CLIENT_ID: &str = "101";
const CLIENT_VERSION: &str = "1.02";
const DEVICE_MODEL: &str = "RealityDevice17,1";
const OS_VERSION: &str = "26.5.23O471";
pub(crate) const UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 15_7_3) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15";
/// visitorData bu süreden eskiyse yenilenir.
const VD_MAX_AGE: u64 = 3 * 24 * 3600;

static VISITOR: Mutex<Option<(String, u64)>> = Mutex::new(None);

pub(crate) fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn vd_file() -> std::path::PathBuf {
    crate::paths::cache_dir().join("yt-visitor.json")
}

pub(crate) fn load_visitor() -> Option<String> {
    let mut g = VISITOR.lock().unwrap();
    if g.is_none() {
        if let Ok(raw) = std::fs::read(vd_file()) {
            if let Ok(v) = serde_json::from_slice::<Value>(&raw) {
                if let (Some(vd), Some(t)) = (v["vd"].as_str(), v["t"].as_u64()) {
                    *g = Some((vd.to_string(), t));
                }
            }
        }
    }
    g.as_ref().filter(|(_, t)| now().saturating_sub(*t) < VD_MAX_AGE).map(|(vd, _)| vd.clone())
}

pub(crate) fn store_visitor(vd: &str) {
    let t = now();
    *VISITOR.lock().unwrap() = Some((vd.to_string(), t));
    let _ = std::fs::create_dir_all(crate::paths::cache_dir());
    let _ = std::fs::write(vd_file(), json!({"vd": vd, "t": t}).to_string());
}

pub(crate) fn forget_visitor() {
    *VISITOR.lock().unwrap() = None;
    let _ = std::fs::remove_file(vd_file());
}

/// VYNZ_YT_FAST=0 visionOS yolunu kapatır (web yedeği ve yt-dlp yine denenir).
pub fn enabled() -> bool {
    std::env::var("VYNZ_YT_FAST").map(|v| v != "0").unwrap_or(true)
}

/// Sayfa adresinden YouTube video kimliği.
pub fn video_id(page_url: &str) -> Option<String> {
    let valid = |s: &str| s.len() == 11 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if let Some(i) = page_url.find("youtu.be/") {
        let id: String = page_url[i + 9..].chars().take(11).collect();
        return valid(&id).then_some(id);
    }
    if !page_url.contains("youtube.com/") {
        return None;
    }
    let query = page_url.split_once('?')?.1;
    for pair in query.split('&') {
        if let Some(id) = pair.strip_prefix("v=") {
            let id: String = id.chars().take(11).collect();
            return valid(&id).then_some(id);
        }
    }
    None
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .http_status_as_error(false)
        .build()
        .into()
}

fn player_request(agent: &ureq::Agent, id: &str, vd: Option<&str>) -> Result<Value, String> {
    let mut client = json!({
        "clientName": CLIENT_NAME,
        "clientVersion": CLIENT_VERSION,
        "deviceMake": "Apple",
        "deviceModel": DEVICE_MODEL,
        "userAgent": UA,
        "osName": "visionOS",
        "osVersion": OS_VERSION,
        "hl": "en",
        "gl": "US",
    });
    if let Some(vd) = vd {
        client["visitorData"] = vd.into();
    }
    let body = json!({
        "context": {"client": client},
        "videoId": id,
        "contentCheckOk": true,
        "racyCheckOk": true,
    });
    let mut req = agent
        .post(PLAYER_URL)
        .header("Content-Type", "application/json")
        .header("User-Agent", UA)
        .header("Origin", "https://www.youtube.com")
        .header("X-YouTube-Client-Name", CLIENT_ID)
        .header("X-YouTube-Client-Version", CLIENT_VERSION);
    if let Some(vd) = vd {
        req = req.header("X-Goog-Visitor-Id", vd);
    }
    let mut resp = req.send(body.to_string()).map_err(|e| format!("ağ: {e}"))?;
    let status = resp.status().as_u16();
    let text = resp.body_mut().with_config().limit(8 << 20).read_to_string().map_err(|e| format!("okuma: {e}"))?;
    if status != 200 {
        return Err(format!("HTTP {status}"));
    }
    serde_json::from_str(&text).map_err(|e| format!("json: {e}"))
}

/// Yanıttan en iyi ses akışını seçer: önce opus (webm), yoksa aac (m4a); yüksek bit hızı önde.
fn pick_audio(v: &Value) -> Option<(String, String, u64)> {
    let formats = v["streamingData"]["adaptiveFormats"].as_array()?;
    let mut best: Option<(u8, u64, String, String)> = None;
    for f in formats {
        let mime = f["mimeType"].as_str().unwrap_or("");
        if !mime.starts_with("audio/") {
            continue;
        }
        let Some(url) = f["url"].as_str() else { continue }; // signatureCipher: bu yol değil
        if url.contains("&n=") || url.contains("?n=") {
            continue; // çözülmemiş n-parametresi yavaşlatılır
        }
        // Dublaj/çeviri ses izlerini atla, orijinali tercih et.
        if let Some(track) = f.get("audioTrack") {
            if track["audioIsDefault"].as_bool() == Some(false) {
                continue;
            }
        }
        let rank = if mime.contains("opus") { 2 } else if mime.contains("mp4a") { 1 } else { 0 };
        let br = f["bitrate"].as_u64().unwrap_or(0);
        let ext = if mime.contains("webm") { "webm" } else { "m4a" };
        if best.as_ref().is_none_or(|b| (rank, br) > (b.0, b.1)) {
            best = Some((rank, br, url.to_string(), ext.to_string()));
        }
    }
    let (_, _, url, ext) = best?;
    let expire = url
        .split(['?', '&'])
        .find_map(|p| p.strip_prefix("expire=")?.parse().ok())
        .unwrap_or(now() + 3 * 3600);
    Some((url, ext, expire))
}

pub(crate) fn status_of(v: &Value) -> (&str, &str) {
    let ps = &v["playabilityStatus"];
    (ps["status"].as_str().unwrap_or(""), ps["reason"].as_str().unwrap_or(""))
}

/// Sayfa adresini çöz. Hata metni yalnız günlük içindir.
pub fn resolve(page_url: &str) -> Result<Resolved, String> {
    let id = video_id(page_url).ok_or("YouTube adresi değil")?;
    let agent = agent();
    let mut vd = load_visitor();
    // En fazla iki tur: visitorData yoksa ya da geçersizse yanıttaki yenisiyle tekrar.
    for attempt in 0..2 {
        let v = player_request(&agent, &id, vd.as_deref())?;
        let resp_vd = v["responseContext"]["visitorData"].as_str().map(str::to_string);
        let (status, reason) = status_of(&v);
        if status == "OK" {
            if vd.is_none() {
                if let Some(r) = &resp_vd {
                    store_visitor(r);
                }
            }
            let (stream, ext, expire) = pick_audio(&v).ok_or("uygun ses akışı yok")?;
            return Ok(Resolved { stream, expire, ext: Some(ext) });
        }
        if status == "LOGIN_REQUIRED" {
            // Bot kontrolü: yanıttaki taze visitorData ile bir kez daha dene.
            if attempt == 0 && resp_vd.is_some() && resp_vd != vd {
                store_visitor(resp_vd.as_deref().unwrap_or_default());
                vd = resp_vd;
                continue;
            }
            forget_visitor();
        }
        return Err(format!("{status}: {reason}"));
    }
    Err("çözülemedi".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids() {
        assert_eq!(video_id("https://music.youtube.com/watch?v=daMS-E6Fbis").as_deref(), Some("daMS-E6Fbis"));
        assert_eq!(video_id("https://www.youtube.com/watch?list=x&v=daMS-E6Fbis&t=3").as_deref(), Some("daMS-E6Fbis"));
        assert_eq!(video_id("https://youtu.be/daMS-E6Fbis?si=a").as_deref(), Some("daMS-E6Fbis"));
        assert_eq!(video_id("https://soundcloud.com/a/b"), None);
    }

    /// Ağ testi: `cargo test ytfast::tests::live -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live() {
        for id in ["daMS-E6Fbis", "J7p4bzqLvCw", "hsm4poTWjMs"] {
            let t0 = std::time::Instant::now();
            let r = resolve(&format!("https://music.youtube.com/watch?v={id}")).unwrap();
            println!("{id}: {} ms ext={:?} exp={}", t0.elapsed().as_millis(), r.ext, r.expire);
            assert!(r.stream.contains("googlevideo.com"));
        }
    }
}
