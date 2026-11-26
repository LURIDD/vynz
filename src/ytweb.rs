// YouTube Music web istemcisi (WEB_REMIX) ile çözüm: ytfast (visionOS) başarısız olursa yedek.
//
// WEB_REMIX şifreli adres verir (signatureCipher + n-parametresi) ve PO token ister.
// Şifre çözümü ve token üretimi deno altında çalışan küçük bir JS yardımcısında (src/ytjs):
// - EJS (yt-dlp/ejs) oynatıcıyı sürüm başına bir kez işler, sonucu diske yazar.
// - pot.js (bgutils) botguard'ı çalıştırıp VİDEO KİMLİĞİNE bağlı GVS token üretir
//   (ziyaretçi verisine bağlı token adreste 403 verir).
// Yardımcı ilk istekte başlar, IDLE_SECS boyunca iş gelmezse kapanır.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::resolve::Resolved;
use crate::ytfast;

const SCRIPT: &str = concat!(
    include_str!("ytjs/ejs/yt.solver.lib.min.js"),
    "\n",
    include_str!("ytjs/glue.js"),
    "\n",
    include_str!("ytjs/ejs/yt.solver.core.min.js"),
    "\n",
    include_str!("ytjs/pot.js"),
    "\n",
    include_str!("ytjs/host.js"),
);

const PLAYER_URL: &str = "https://music.youtube.com/youtubei/v1/player?prettyPrint=false";
const CLIENT_VERSION: &str = "1.20261118.01.00";
const WEB_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:140.0) Gecko/20100101 Firefox/140.0";
const IDLE_SECS: u64 = 600;
const CALL_TIMEOUT: Duration = Duration::from_secs(30);
/// Oynatıcı kimliği bu kadar süre geçerli sayılır.
const PLAYER_ID_TTL: Duration = Duration::from_secs(3600);

struct Proc {
    child: Child,
    stdin: ChildStdin,
}

#[derive(Default)]
struct Helper {
    proc: Option<Proc>,
    next_id: u64,
    pending: HashMap<u64, Sender<Result<Value, String>>>,
    last_used: Option<Instant>,
    janitor: bool,
}

fn helper() -> &'static Arc<Mutex<Helper>> {
    static H: OnceLock<Arc<Mutex<Helper>>> = OnceLock::new();
    H.get_or_init(|| Arc::new(Mutex::new(Helper::default())))
}

fn js_dir() -> std::path::PathBuf {
    crate::paths::cache_dir().join("ytjs")
}

fn spawn(h: &mut Helper) -> Result<(), String> {
    if let Some(p) = h.proc.as_mut() {
        if matches!(p.child.try_wait(), Ok(None)) {
            return Ok(());
        }
        h.proc = None;
    }
    let deno = crate::runtime::deno_bin().ok_or("deno yok")?;
    let dir = js_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let script = dir.join("host.js");
    if std::fs::read_to_string(&script).ok().as_deref() != Some(SCRIPT) {
        std::fs::write(&script, SCRIPT).map_err(|e| e.to_string())?;
    }
    // Eski oynatıcı sürümlerinin önbelleklerini temizle (her biri ~4 MB): en yeni 2 kalsın.
    prune_players(&dir);
    let dir_s = dir.to_string_lossy().into_owned();
    let mut cmd = Command::new(deno);
    crate::runtime::hide_window(&mut cmd);
    cmd.args(["run", "--quiet", "--no-prompt", "--no-config", "--no-lock", "--no-npm", "--no-remote"])
        .arg("--allow-net")
        .arg(format!("--allow-read={dir_s}"))
        .arg(format!("--allow-write={dir_s}"))
        .arg("--v8-flags=--stack-size=8192")
        .arg(&script)
        .arg(&dir_s)
        .current_dir(&dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = cmd.spawn().map_err(|e| format!("deno başlatılamadı: {e}"))?;
    let stdin = child.stdin.take().ok_or("stdin yok")?;
    let stdout = child.stdout.take().ok_or("stdout yok")?;
    let shared = helper().clone();
    thread::Builder::new()
        .name("vynz-ytjs-out".into())
        .spawn(move || read_loop(shared, stdout))
        .map_err(|e| e.to_string())?;
    h.proc = Some(Proc { child, stdin });
    if !h.janitor {
        h.janitor = true;
        start_janitor();
    }
    Ok(())
}

fn prune_players(dir: &std::path::Path) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut files: Vec<_> = rd
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("ejs-"))
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    files.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, p) in files.into_iter().skip(2) {
        let _ = std::fs::remove_file(p);
    }
}

fn read_loop(shared: Arc<Mutex<Helper>>, stdout: std::process::ChildStdout) {
    for line in BufReader::new(stdout).lines() {
        let Ok(line) = line else { break };
        let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
        let Some(id) = v["id"].as_u64() else { continue }; // "ready"
        let tx = shared.lock().unwrap().pending.remove(&id);
        if let Some(tx) = tx {
            let r = if v["ok"].as_bool() == Some(true) {
                Ok(v)
            } else {
                Err(v["error"].as_str().unwrap_or("yardımcı hatası").to_string())
            };
            let _ = tx.send(r);
        }
    }
    let mut h = shared.lock().unwrap();
    h.proc = None;
    for (_, tx) in h.pending.drain() {
        let _ = tx.send(Err("yardımcı kapandı".into()));
    }
}

fn start_janitor() {
    thread::spawn(|| loop {
        thread::sleep(Duration::from_secs(30));
        let mut h = helper().lock().unwrap();
        let idle = h.last_used.is_none_or(|t| t.elapsed() > Duration::from_secs(IDLE_SECS));
        if idle && h.pending.is_empty() {
            if let Some(mut p) = h.proc.take() {
                let _ = p.child.kill();
                let _ = p.child.wait();
            }
        }
    });
}

/// Yardımcıya bir istek gönder ve yanıtı bekle.
fn call(mut req: Value) -> Result<Value, String> {
    let (tx, rx) = channel();
    {
        let mut h = helper().lock().unwrap();
        spawn(&mut h)?;
        h.next_id += 1;
        let id = h.next_id;
        req["id"] = id.into();
        h.pending.insert(id, tx);
        h.last_used = Some(Instant::now());
        let line = format!("{req}\n");
        let ok = h.proc.as_mut().is_some_and(|p| p.stdin.write_all(line.as_bytes()).and_then(|_| p.stdin.flush()).is_ok());
        if !ok {
            h.pending.remove(&id);
            h.proc = None;
            return Err("yardımcıya yazılamadı".into());
        }
    }
    let r = rx.recv_timeout(CALL_TIMEOUT).map_err(|_| "yardımcı zaman aşımı".to_string())?;
    helper().lock().unwrap().last_used = Some(Instant::now());
    r
}

/// Program kapanırken yardımcıyı kapat.
pub fn shutdown() {
    if let Some(mut p) = helper().lock().unwrap().proc.take() {
        let _ = p.child.kill();
        let _ = p.child.wait();
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(15)))
        .http_status_as_error(false)
        .build()
        .into()
}

/// Güncel oynatıcı kimliği (iframe_api içinden), bir saat önbellekli.
fn player_id(agent: &ureq::Agent) -> Result<String, String> {
    static CACHE: Mutex<Option<(String, Instant)>> = Mutex::new(None);
    if let Some((id, t)) = CACHE.lock().unwrap().as_ref() {
        if t.elapsed() < PLAYER_ID_TTL {
            return Ok(id.clone());
        }
    }
    let mut resp = agent
        .get("https://www.youtube.com/iframe_api")
        .header("User-Agent", WEB_UA)
        .call()
        .map_err(|e| format!("iframe_api: {e}"))?;
    let text = resp.body_mut().read_to_string().map_err(|e| e.to_string())?;
    let id = extract_player_id(&text).ok_or("oynatıcı kimliği bulunamadı")?;
    *CACHE.lock().unwrap() = Some((id.clone(), Instant::now()));
    Ok(id)
}

fn extract_player_id(text: &str) -> Option<String> {
    let t = text.replace("\\/", "/");
    let i = t.find("/s/player/")? + "/s/player/".len();
    let id: String = t[i..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-').collect();
    (id.len() >= 4).then_some(id)
}

fn pct_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => {
                let hex = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
                if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                    out.push(h * 16 + l);
                    i += 3;
                    continue;
                }
                out.push(b'%');
            }
            b'+' => out.push(b' '),
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn pct_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// "a=1&b=2" -> [(a, 1), (b, 2)] (değerler çözülmüş).
fn parse_query(q: &str) -> Vec<(String, String)> {
    q.split('&')
        .filter(|p| !p.is_empty())
        .map(|p| match p.split_once('=') {
            Some((k, v)) => (pct_decode(k), pct_decode(v)),
            None => (pct_decode(p), String::new()),
        })
        .collect()
}

/// Adresteki bir parametreyi değiştir ya da ekle.
fn set_param(url: &str, key: &str, value: &str) -> String {
    let (base, query) = url.split_once('?').unwrap_or((url, ""));
    let mut parts: Vec<String> = query
        .split('&')
        .filter(|p| !p.is_empty() && p.split('=').next() != Some(key))
        .map(str::to_string)
        .collect();
    parts.push(format!("{key}={}", pct_encode(value)));
    format!("{base}?{}", parts.join("&"))
}

fn get_param(url: &str, key: &str) -> Option<String> {
    let query = url.split_once('?')?.1;
    parse_query(query).into_iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

struct Candidate {
    url: String,
    sig: Option<(String, String)>, // (s, sp)
    ext: String,
}

fn pick_audio(v: &Value) -> Option<Candidate> {
    let formats = v["streamingData"]["adaptiveFormats"].as_array()?;
    let mut best: Option<(u8, u64, Candidate)> = None;
    for f in formats {
        let mime = f["mimeType"].as_str().unwrap_or("");
        if !mime.starts_with("audio/") {
            continue;
        }
        if f.get("audioTrack").is_some_and(|t| t["audioIsDefault"].as_bool() == Some(false)) {
            continue;
        }
        let (url, sig) = if let Some(u) = f["url"].as_str() {
            (u.to_string(), None)
        } else if let Some(c) = f["signatureCipher"].as_str() {
            let q = parse_query(c);
            let find = |k: &str| q.iter().find(|(a, _)| a == k).map(|(_, b)| b.clone());
            let (Some(u), Some(s)) = (find("url"), find("s")) else { continue };
            (u, Some((s, find("sp").unwrap_or_else(|| "signature".into()))))
        } else {
            continue;
        };
        let rank = if mime.contains("opus") { 2 } else if mime.contains("mp4a") { 1 } else { 0 };
        let br = f["bitrate"].as_u64().unwrap_or(0);
        let ext = if mime.contains("webm") { "webm" } else { "m4a" }.to_string();
        if best.as_ref().is_none_or(|b| (rank, br) > (b.0, b.1)) {
            best = Some((rank, br, Candidate { url, sig, ext }));
        }
    }
    best.map(|b| b.2)
}

fn player_request(agent: &ureq::Agent, id: &str, vd: &str, sts: u64, pot: &str) -> Result<Value, String> {
    let body = json!({
        "context": {"client": {
            "clientName": "WEB_REMIX",
            "clientVersion": CLIENT_VERSION,
            "hl": "en",
            "gl": "US",
            "visitorData": vd,
        }},
        "videoId": id,
        "contentCheckOk": true,
        "racyCheckOk": true,
        "playbackContext": {"contentPlaybackContext": {"signatureTimestamp": sts}},
        "serviceIntegrityDimensions": {"poToken": pot},
    });
    let mut resp = agent
        .post(PLAYER_URL)
        .header("Content-Type", "application/json")
        .header("User-Agent", WEB_UA)
        .header("Origin", "https://music.youtube.com")
        .header("Referer", "https://music.youtube.com/")
        .header("X-YouTube-Client-Name", "67")
        .header("X-YouTube-Client-Version", CLIENT_VERSION)
        .header("X-Goog-Visitor-Id", vd)
        .send(body.to_string())
        .map_err(|e| format!("ağ: {e}"))?;
    let status = resp.status().as_u16();
    let text = resp.body_mut().with_config().limit(8 << 20).read_to_string().map_err(|e| e.to_string())?;
    if status != 200 {
        return Err(format!("HTTP {status}"));
    }
    serde_json::from_str(&text).map_err(|e| format!("json: {e}"))
}

/// Ziyaretçi verisi yoksa visionOS isteğinden al (ytfast aynı dosyayı paylaşır).
fn visitor(agent: &ureq::Agent, video: &str) -> Result<String, String> {
    if let Some(vd) = ytfast::load_visitor() {
        return Ok(vd);
    }
    let body = json!({"context": {"client": {"clientName": "WEB_REMIX", "clientVersion": CLIENT_VERSION, "hl": "en", "gl": "US"}}, "videoId": video});
    let mut resp = agent
        .post(PLAYER_URL)
        .header("Content-Type", "application/json")
        .header("User-Agent", WEB_UA)
        .header("X-YouTube-Client-Name", "67")
        .header("X-YouTube-Client-Version", CLIENT_VERSION)
        .send(body.to_string())
        .map_err(|e| format!("ağ: {e}"))?;
    let v: Value = serde_json::from_str(&resp.body_mut().read_to_string().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let vd = v["responseContext"]["visitorData"].as_str().ok_or("visitorData alınamadı")?.to_string();
    ytfast::store_visitor(&vd);
    Ok(vd)
}

enum Probe {
    Ok,
    Rejected,
}

/// Adresin 1.1 MB sonrasından 2 bayt iste: 403 = token reddedildi.
/// Dosya bundan küçükse ya da ağ hatası olursa sınamadan geç (çalma yine denenecek).
fn probe(agent: &ureq::Agent, url: &str) -> Probe {
    const AT: u64 = 1_100_000;
    let clen: u64 = get_param(url, "clen").and_then(|c| c.parse().ok()).unwrap_or(0);
    if clen <= AT + 2 {
        return Probe::Ok;
    }
    match agent.get(url).header("Range", &format!("bytes={AT}-{}", AT + 1)).call() {
        Ok(r) if r.status().as_u16() == 403 => Probe::Rejected,
        _ => Probe::Ok,
    }
}

/// Etkin mi? VYNZ_YT_WEB=0 kapatır.
pub fn enabled() -> bool {
    std::env::var("VYNZ_YT_WEB").map(|v| v != "0").unwrap_or(true) && crate::runtime::deno_bin().is_some()
}

pub fn resolve(page_url: &str) -> Result<Resolved, String> {
    let id = ytfast::video_id(page_url).ok_or("YouTube adresi değil")?;
    let agent = agent();
    let vd = visitor(&agent, &id)?;
    let pid = player_id(&agent)?;
    // Token ve oynatıcı hazırlığı (sts) aynı anda.
    let (pot, prep) = thread::scope(|s| {
        let pot = s.spawn(|| call(json!({"op": "pot", "video": id, "visitor": vd, "ua": WEB_UA})));
        let prep = call(json!({"op": "solve", "player": pid}));
        (pot.join().unwrap_or_else(|_| Err("pot iş parçacığı".into())), prep)
    });
    let pot = pot?;
    let token = pot["token"].as_str().ok_or("token yok")?.to_string();
    let sts = prep?["sts"].as_u64().unwrap_or(0);
    let v = player_request(&agent, &id, &vd, sts, &token)?;
    let (status, reason) = ytfast::status_of(&v);
    if status != "OK" {
        return Err(format!("{status}: {reason}"));
    }
    let c = pick_audio(&v).ok_or("uygun ses akışı yok (yalnız SABR?)")?;
    let n_in = get_param(&c.url, "n");
    let mut req = json!({"op": "solve", "player": pid});
    if let Some(n) = &n_in {
        req["n"] = json!([n]);
    }
    if let Some((s, _)) = &c.sig {
        req["sig"] = json!([s]);
    }
    let solved = if n_in.is_some() || c.sig.is_some() { call(req)? } else { json!({}) };
    let mut url = c.url;
    if let Some(n) = &n_in {
        let out = solved["n"][n].as_str().ok_or("n çözülemedi")?;
        url = set_param(&url, "n", out);
    }
    if let Some((s, sp)) = &c.sig {
        let out = solved["sig"][s].as_str().ok_or("imza çözülemedi")?;
        url = set_param(&url, sp, out);
    }
    url = set_param(&url, "pot", &token);
    // Token geçerliliği adresten anlaşılmaz: googlevideo `pot`'u ilk ~1 MB'tan sonra denetler.
    // Küçük bir aralık isteğiyle sına; reddedilirse üreticiyi sıfırla, yeni token al.
    for attempt in 0..3 {
        match probe(&agent, &url) {
            Probe::Ok => break,
            Probe::Rejected if attempt < 2 => {
                crate::dbg(&format!("ytweb {id}: token reddedildi, üretici yenileniyor"));
                call(json!({"op": "reset"}))?;
                let t = call(json!({"op": "pot", "video": id, "visitor": vd, "ua": WEB_UA}))?;
                let token = t["token"].as_str().ok_or("token yok")?;
                url = set_param(&url, "pot", token);
            }
            Probe::Rejected => return Err("token reddedildi".into()),
        }
    }
    let expire = get_param(&url, "expire").and_then(|e| e.parse().ok()).unwrap_or(ytfast::now() + 3 * 3600);
    Ok(Resolved { stream: url, expire, ext: Some(c.ext) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_helpers() {
        assert_eq!(pct_decode("a%3Db%26c+d"), "a=b&c d");
        assert_eq!(pct_encode("a=b/c"), "a%3Db%2Fc");
        let u = set_param("https://x/y?a=1&n=old&b=2", "n", "new=");
        assert_eq!(u, "https://x/y?a=1&b=2&n=new%3D");
        assert_eq!(get_param(&u, "n").as_deref(), Some("new="));
        assert_eq!(extract_player_id(r#"var a='https:\/\/www.youtube.com\/s\/player\/b485e332\/www-widgetapi.vflset'"#).as_deref(), Some("b485e332"));
    }

    /// Ağ testi: `cargo test ytweb::tests::live -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live() {
        for id in ["daMS-E6Fbis", "J7p4bzqLvCw", "fJ9rUzIMcZQ"] {
            let t0 = Instant::now();
            let r = resolve(&format!("https://music.youtube.com/watch?v={id}")).unwrap();
            println!("{id}: {} ms ext={:?}", t0.elapsed().as_millis(), r.ext);
            println!("URL {}", r.stream);
        }
    }
}
