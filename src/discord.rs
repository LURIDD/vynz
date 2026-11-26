// Discord Rich Presence: çalan şarkıyı Discord profilinde "Dinliyor" olarak gösterir.
// Bağımlılıksız IPC: Unix'te discord-ipc-N soketi, Windows'ta \\.\pipe\discord-ipc-N.
// Çerçeve: opcode (u32 LE) + uzunluk (u32 LE) + JSON. 0 = el sıkışma, 1 = komut.
//
// MPRIS'e giden mesajlar `feed` ile buraya da kopyalanır. Konum güncellemeleri kanala
// girmez (atomik değişkene yazılır), iş parçacığı yalnız şarkı/durum değişince uyanır.
// Discord kapalıyken yeniden deneme aralığı 15 sn'den 2 dk'ya kadar katlanır.

use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::mpris::Msg;

/// Discord Developer Portal'daki uygulama kimliği ("Vynz"). Boşsa özellik kapalıdır.
/// `VYNZ_DISCORD_ID` ortam değişkeni bunu geçersiz kılar.
const CLIENT_ID: &str = "1574105198274547844";

const BACKOFF_MIN: Duration = Duration::from_secs(15);
const BACKOFF_MAX: Duration = Duration::from_secs(120);

enum Ev {
    Track { title: String, artist: String, album: String, art: String, length: f64 },
    Length(f64),
    Playing(bool),
    Seeked,
    Enable(bool),
}

static TX: OnceLock<Mutex<Sender<Ev>>> = OnceLock::new();
/// Son konum (sn, f64 bitleri).
static POS: AtomicU64 = AtomicU64::new(0);
static ENABLED: AtomicBool = AtomicBool::new(false);

fn client_id() -> Option<String> {
    let id = std::env::var("VYNZ_DISCORD_ID").unwrap_or_else(|_| CLIENT_ID.to_string());
    let id = id.trim().to_string();
    (!id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())).then_some(id)
}

pub fn start(enabled: bool) {
    let Some(id) = client_id() else { return };
    let (tx, rx) = mpsc::channel::<Ev>();
    if TX.set(Mutex::new(tx)).is_err() {
        return;
    }
    ENABLED.store(enabled, Ordering::Relaxed);
    std::thread::Builder::new()
        .name("discord-rpc".into())
        .spawn(move || run(id, rx, enabled))
        .ok();
}

pub fn set_enabled(on: bool) {
    if ENABLED.swap(on, Ordering::Relaxed) != on {
        send(Ev::Enable(on));
    }
}

fn send(ev: Ev) {
    if let Some(tx) = TX.get() {
        if let Ok(tx) = tx.lock() {
            let _ = tx.send(ev);
        }
    }
}

/// MPRIS mesajının kopyası.
pub fn feed(m: &Msg) {
    if TX.get().is_none() {
        return;
    }
    match m {
        Msg::Track { title, artist, album, art, length, .. } => {
            POS.store(0f64.to_bits(), Ordering::Relaxed);
            send(Ev::Track { title: title.clone(), artist: artist.clone(), album: album.clone(), art: art.clone(), length: *length });
        }
        Msg::Length(d) => send(Ev::Length(*d)),
        Msg::Playing(p) => send(Ev::Playing(*p)),
        Msg::Position(t) => POS.store(t.to_bits(), Ordering::Relaxed),
        Msg::Seeked(t) => {
            POS.store(t.to_bits(), Ordering::Relaxed);
            send(Ev::Seeked);
        }
        _ => {}
    }
}

struct Now {
    title: String,
    artist: String,
    album: String,
    art: String,
    length: f64,
    playing: bool,
}

fn run(id: String, rx: mpsc::Receiver<Ev>, mut enabled: bool) {
    let mut now: Option<Now> = None;
    let mut conn: Option<Conn> = None;
    let mut dirty = false;
    let mut backoff = BACKOFF_MIN;
    let mut last_try: Option<Instant> = None;
    let mut no_type = false;
    loop {
        // Bağlantı bekleniyorsa zaman aşımıyla, yoksa olay gelene kadar uyu.
        let waiting = enabled && conn.is_none() && dirty;
        let ev = if waiting {
            let wait = last_try.map(|t| backoff.saturating_sub(t.elapsed())).unwrap_or_default();
            match rx.recv_timeout(wait) {
                Ok(e) => Some(e),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        } else {
            match rx.recv() {
                Ok(e) => Some(e),
                Err(_) => return,
            }
        };
        if let Some(ev) = ev {
            apply(ev, &mut now, &mut enabled, &mut conn);
            // Ardışık olayları (Track + Playing + Length) tek güncellemede topla.
            while let Ok(more) = rx.recv_timeout(Duration::from_millis(150)) {
                apply(more, &mut now, &mut enabled, &mut conn);
            }
            dirty = true;
        }
        if !enabled || !dirty {
            continue;
        }
        if conn.is_none() {
            if last_try.is_some_and(|t| t.elapsed() < backoff) {
                continue;
            }
            last_try = Some(Instant::now());
            match Conn::open(&id) {
                Some(c) => {
                    conn = Some(c);
                    backoff = BACKOFF_MIN;
                }
                None => {
                    backoff = (backoff * 2).min(BACKOFF_MAX);
                    continue;
                }
            }
        }
        let Some(c) = conn.as_mut() else { continue };
        let act = now.as_ref().map(|n| activity(n, no_type, false)).unwrap_or(Value::Null);
        match c.set_activity(act) {
            Ok(None) => dirty = false,
            Ok(Some(err)) => {
                eprintln!("discord: etkinlik reddedildi: {err}");
                // Yalnız "type" alanını tanımayan eski istemcide türsüz gönder; başka her ret
                // (uzun alan, geçersiz kapak adresi...) için türü koruyup sade sürümü dene.
                // Önceden her ret kalıcı olarak türü düşürüyordu: durum "Dinliyor" yerine
                // "Oynuyor"a takılıp kalıyordu.
                let type_err = !no_type && err.to_lowercase().contains("type");
                let act = now.as_ref().map(|n| activity(n, no_type || type_err, !type_err)).unwrap_or(Value::Null);
                match c.set_activity(act) {
                    // Türsüz gönderim gerçekten kabul edildiyse bu bağlantı için türü bırak.
                    Ok(None) => no_type |= type_err,
                    Ok(Some(err)) => eprintln!("discord: sade etkinlik de reddedildi: {err}"),
                    Err(_) => {
                        conn = None;
                        last_try = None;
                    }
                }
                dirty = false; // aynı içeriği tekrar tekrar gönderme
            }
            Err(_) => {
                conn = None;
                last_try = None; // Discord yeniden başlatılmış olabilir: hemen bir kez dene
                no_type = false; // yeni istemci türü tanıyor olabilir
            }
        }
    }
}

fn apply(ev: Ev, now: &mut Option<Now>, enabled: &mut bool, conn: &mut Option<Conn>) {
    match ev {
        Ev::Track { title, artist, album, art, length } => {
            *now = Some(Now { title, artist, album, art, length, playing: true });
        }
        Ev::Length(d) => {
            if let Some(n) = now.as_mut() {
                n.length = d;
            }
        }
        Ev::Playing(p) => {
            if let Some(n) = now.as_mut() {
                n.playing = p;
            }
        }
        Ev::Seeked => {}
        Ev::Enable(on) => {
            *enabled = on;
            if !on {
                if let Some(c) = conn.as_mut() {
                    let _ = c.set_activity(Value::Null);
                }
                *conn = None;
            }
        }
    }
}

fn clock(sec: f64) -> String {
    let s = sec.max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

/// Discord alanları 2..128 karakter ister.
fn field(s: &str) -> String {
    let mut s: String = s.trim().chars().take(128).collect();
    while s.chars().count() < 2 {
        s.push(' ');
    }
    s
}

/// `plain`: kapak ve zaman damgası olmadan (ret sonrası güvenli sürüm).
fn activity(n: &Now, no_type: bool, plain: bool) -> Value {
    let artist = if n.artist.trim().is_empty() { "Vynz" } else { n.artist.as_str() };
    let pos = f64::from_bits(POS.load(Ordering::Relaxed)).max(0.0);
    // Duraklatılmışken Discord'da işleyen bir sayaç yerine son konum sabit yazı olarak kalır.
    let state = if n.playing {
        artist.to_string()
    } else if n.length > 0.0 {
        format!("⏸ {} / {} · {artist}", clock(pos), clock(n.length))
    } else {
        format!("⏸ {} · {artist}", clock(pos))
    };
    let mut a = json!({
        "details": field(&n.title),
        "state": field(&state),
    });
    if !no_type {
        a["type"] = json!(2); // Listening
        // Üye listesinde "Vynz dinliyor" yerine şarkı adı (details); profil kartı yine "Vynz".
        a["status_display_type"] = json!(2);
    }
    let mut assets = serde_json::Map::new();
    if !plain && n.art.starts_with("https://") && n.art.len() <= 256 {
        assets.insert("large_image".into(), json!(n.art));
        let text = if n.album.trim().is_empty() { &n.title } else { &n.album };
        assets.insert("large_text".into(), json!(field(text)));
    }
    if !n.playing {
        assets.insert("small_text".into(), json!("Duraklatıldı"));
    }
    if !assets.is_empty() {
        a["assets"] = Value::Object(assets);
    }
    if n.playing && !plain {
        let now_ms = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
        let start = now_ms - (pos * 1000.0) as i64;
        let mut ts = json!({ "start": start });
        if n.length > 0.0 {
            ts["end"] = json!(start + (n.length * 1000.0) as i64);
        }
        a["timestamps"] = ts;
    }
    a
}

// ---------- IPC ----------

trait Pipe: Read + Write + Send {}
impl<T: Read + Write + Send> Pipe for T {}

struct Conn {
    io: Box<dyn Pipe>,
    nonce: u64,
}

impl Conn {
    fn open(id: &str) -> Option<Conn> {
        for io in candidates() {
            let mut c = Conn { io, nonce: 0 };
            if c.handshake(id).is_ok() {
                return Some(c);
            }
        }
        None
    }

    fn write_frame(&mut self, op: u32, body: &Value) -> std::io::Result<()> {
        let data = serde_json::to_vec(body)?;
        let mut buf = Vec::with_capacity(8 + data.len());
        buf.extend_from_slice(&op.to_le_bytes());
        buf.extend_from_slice(&(data.len() as u32).to_le_bytes());
        buf.extend_from_slice(&data);
        self.io.write_all(&buf)?;
        self.io.flush()
    }

    fn read_frame(&mut self) -> std::io::Result<(u32, Value)> {
        let mut head = [0u8; 8];
        self.io.read_exact(&mut head)?;
        let op = u32::from_le_bytes([head[0], head[1], head[2], head[3]]);
        let len = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as usize;
        if len > 1 << 20 {
            return Err(std::io::Error::other("çerçeve çok büyük"));
        }
        let mut data = vec![0u8; len];
        self.io.read_exact(&mut data)?;
        Ok((op, serde_json::from_slice(&data).unwrap_or(Value::Null)))
    }

    fn handshake(&mut self, id: &str) -> std::io::Result<()> {
        self.write_frame(0, &json!({ "v": 1, "client_id": id }))?;
        let (op, body) = self.read_frame()?;
        if op == 1 && body["evt"] == "READY" {
            Ok(())
        } else {
            Err(std::io::Error::other("el sıkışma reddedildi"))
        }
    }

    /// Ok(true) kabul edildi, Ok(false) Discord hata döndü, Err bağlantı koptu.
    /// `Ok(None)`: kabul edildi; `Ok(Some(ileti))`: Discord reddetti.
    fn set_activity(&mut self, activity: Value) -> std::io::Result<Option<String>> {
        self.nonce += 1;
        let req = json!({
            "cmd": "SET_ACTIVITY",
            "args": { "pid": std::process::id(), "activity": activity },
            "nonce": self.nonce.to_string(),
        });
        self.write_frame(1, &req)?;
        // Yanıtı nonce ile eşle; araya giren başka olayları atla.
        for _ in 0..8 {
            let (op, body) = self.read_frame()?;
            if op == 2 {
                return Err(std::io::Error::other("Discord bağlantıyı kapattı"));
            }
            if body["nonce"] == req["nonce"] {
                if body["evt"] == "ERROR" {
                    let msg = body["data"]["message"].as_str().unwrap_or("bilinmeyen hata");
                    return Ok(Some(format!("{} ({})", msg, body["data"]["code"])));
                }
                return Ok(None);
            }
        }
        Ok(Some("yanıt gelmedi".into()))
    }
}

#[cfg(unix)]
fn candidates() -> Vec<Box<dyn Pipe>> {
    use std::os::unix::net::UnixStream;
    let mut bases: Vec<String> = Vec::new();
    for var in ["XDG_RUNTIME_DIR", "TMPDIR", "TMP", "TEMP"] {
        if let Ok(v) = std::env::var(var) {
            if !v.is_empty() && !bases.contains(&v) {
                bases.push(v);
            }
        }
    }
    bases.push("/tmp".into());
    // Flatpak / Snap Discord ve Vesktop kendi alt klasörlerinde açar.
    let subs = ["", "app/com.discordapp.Discord/", "app/com.discordapp.DiscordCanary/", "snap.discord/", ".flatpak/dev.vencord.Vesktop/xdg-run/"];
    let mut out: Vec<Box<dyn Pipe>> = Vec::new();
    for b in &bases {
        for s in subs {
            for i in 0..10 {
                let p = format!("{}/{}discord-ipc-{}", b.trim_end_matches('/'), s, i);
                if let Ok(st) = UnixStream::connect(&p) {
                    let _ = st.set_read_timeout(Some(Duration::from_secs(5)));
                    let _ = st.set_write_timeout(Some(Duration::from_secs(5)));
                    out.push(Box::new(st));
                }
            }
        }
    }
    out
}

#[cfg(windows)]
fn candidates() -> Vec<Box<dyn Pipe>> {
    let mut out: Vec<Box<dyn Pipe>> = Vec::new();
    for i in 0..10 {
        let p = format!(r"\\.\pipe\discord-ipc-{i}");
        if let Ok(f) = std::fs::OpenOptions::new().read(true).write(true).open(&p) {
            out.push(Box::new(f));
        }
    }
    out
}

#[cfg(not(any(unix, windows)))]
fn candidates() -> Vec<Box<dyn Pipe>> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_are_padded_and_clipped() {
        assert_eq!(field("a"), "a ");
        assert_eq!(field(&"x".repeat(300)).chars().count(), 128);
    }

    #[test]
    fn paused_activity_has_no_timestamps() {
        let n = Now { title: "Şarkı".into(), artist: "Sanatçı".into(), album: "".into(), art: "https://i.ytimg.com/x.jpg".into(), length: 200.0, playing: false };
        let a = activity(&n, false, false);
        assert!(a.get("timestamps").is_none());
        assert_eq!(a["type"], 2);
        assert_eq!(a["assets"]["large_image"], "https://i.ytimg.com/x.jpg");
        let n = Now { playing: true, ..n };
        let a = activity(&n, true, false);
        assert!(a.get("type").is_none());
        assert!(a["timestamps"]["end"].as_i64().unwrap() > a["timestamps"]["start"].as_i64().unwrap());
    }
}

#[cfg(test)]
mod rpc_tests {
    use super::*;

    #[test]
    fn paused_activity_is_static() {
        POS.store(83.4f64.to_bits(), Ordering::Relaxed);
        let mut n = Now { title: "Gidiyorum".into(), artist: "Sezen Aksu".into(), album: String::new(), art: String::new(), length: 225.0, playing: false };
        let a = activity(&n, false, false);
        assert!(a.get("timestamps").is_none());
        assert_eq!(a["state"], "⏸ 1:23 / 3:45 · Sezen Aksu");
        assert_eq!(a["status_display_type"], 2);
        n.playing = true;
        let a = activity(&n, false, false);
        assert_eq!(a["state"], "Sezen Aksu");
        assert!(a["timestamps"]["end"].as_i64().unwrap() > a["timestamps"]["start"].as_i64().unwrap());
        assert!(activity(&n, true, false).get("status_display_type").is_none());
    }

    #[test]
    fn plain_fallback_keeps_listening_type() {
        let n = Now { title: "Şarkı".into(), artist: "Sanatçı".into(), album: String::new(), art: "https://i.ytimg.com/x.jpg".into(), length: 200.0, playing: true };
        let a = activity(&n, false, true);
        assert_eq!(a["type"], 2);
        assert!(a.get("assets").is_none());
        assert!(a.get("timestamps").is_none());
    }
}
