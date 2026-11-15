// Akış adresi çözümü: resolver.py yardımcısını yönetir.
// - İhtiyaç olunca başlatır, IDLE_SECS boyunca iş gelmezse kapatır (bellekte durmaz).
// - Çözülen adresleri süresi dolana kadar bellekte ve diskte tutar.
// - Aynı parça için gelen istekleri birleştirir; "şimdi çal" istekleri öne geçer.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

const HELPER: &str = include_str!("resolver.py");
/// Boşta bu kadar kalınca yardımcı kapanır (ilk tıklama sıcak kalsın diye uzun).
const IDLE_SECS: u64 = 900;
/// Adresin bitmesine bu kadar kala yeniden çöz.
const EXPIRY_MARGIN: u64 = 120;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Resolved {
    pub stream: String,
    pub expire: u64,
    #[serde(default)]
    pub ext: Option<String>,
}

pub type Done = Box<dyn FnOnce(Result<Resolved, String>) + Send>;

/// Radyo şarkısı (ytmusicapi'den ham).
#[derive(Clone, Debug, Deserialize)]
pub struct RadioTrack {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub artist: String,
    #[serde(default)]
    pub album: String,
    #[serde(default)]
    pub duration: f32,
    #[serde(default)]
    pub thumb: String,
}

pub type RadioDone = Box<dyn FnOnce(Result<Vec<RadioTrack>, String>) + Send>;
pub type ValueDone = Box<dyn FnOnce(Result<serde_json::Value, String>) + Send>;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Prio {
    Now = 0,
    Soon = 1,
    Maybe = 2,
}

struct Proc {
    child: Child,
    stdin: ChildStdin,
    generation: u64,
}

#[derive(Default)]
struct Inner {
    proc: Option<Proc>,
    generation: u64,
    next_rid: u64,
    /// rid -> parça anahtarı
    rids: HashMap<u64, String>,
    /// parça anahtarı -> bekleyen geri çağrılar
    waiting: HashMap<String, Vec<Done>>,
    /// radyo istekleri: rid -> geri çağrı
    radios: HashMap<u64, RadioDone>,
    /// genel istekler (keşfet, koleksiyon): rid -> geri çağrı
    calls: HashMap<u64, ValueDone>,
    cache: HashMap<String, Resolved>,
    last_used: Option<Instant>,
    dirty: bool,
}

#[derive(Clone)]
pub struct Resolver {
    inner: Arc<Mutex<Inner>>,
    cookies: Arc<Mutex<Option<String>>>,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn data_dir() -> PathBuf {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or(home.join(".cache")).join("vynz")
}

fn cache_file() -> PathBuf {
    data_dir().join("streams.json")
}

impl Resolver {
    pub fn new(cookies: Option<String>) -> Resolver {
        let mut inner = Inner::default();
        if let Ok(raw) = std::fs::read(cache_file()) {
            if let Ok(map) = serde_json::from_slice::<HashMap<String, Resolved>>(&raw) {
                let t = now();
                inner.cache = map.into_iter().filter(|(_, r)| r.expire > t + EXPIRY_MARGIN).collect();
            }
        }
        let r = Resolver { inner: Arc::new(Mutex::new(inner)), cookies: Arc::new(Mutex::new(cookies)) };
        r.start_janitor();
        r
    }

    pub fn set_cookies(&self, browser: Option<String>) {
        *self.cookies.lock().unwrap() = browser;
    }

    /// Önbellekte geçerli adres varsa hemen döner.
    pub fn cached(&self, key: &str) -> Option<Resolved> {
        let g = self.inner.lock().unwrap();
        g.cache.get(key).filter(|r| r.expire > now() + EXPIRY_MARGIN).cloned()
    }

    pub fn is_pending(&self, key: &str) -> bool {
        self.inner.lock().unwrap().waiting.contains_key(key)
    }

    /// Çöz; sonuç `done` ile (yardımcı iş parçacığından) bildirilir.
    pub fn resolve(&self, key: &str, page_url: &str, prio: Prio, done: Option<Done>) {
        if let Some(r) = self.cached(key) {
            if let Some(d) = done {
                d(Ok(r));
            }
            return;
        }
        let mut g = self.inner.lock().unwrap();
        g.last_used = Some(Instant::now());
        if let Some(list) = g.waiting.get_mut(key) {
            // Zaten yolda. "Şimdi" isteğiyse yine de öncelikli bir kopya gönder.
            if let Some(d) = done {
                list.push(d);
            }
            if prio != Prio::Now {
                return;
            }
        } else {
            g.waiting.insert(key.to_string(), done.into_iter().collect());
        }
        if let Err(e) = self.ensure_proc(&mut g) {
            let waiters = g.waiting.remove(key).unwrap_or_default();
            drop(g);
            for d in waiters {
                d(Err(e.clone()));
            }
            return;
        }
        g.next_rid += 1;
        let rid = g.next_rid;
        g.rids.insert(rid, key.to_string());
        let req = serde_json::json!({
            "rid": rid,
            "url": page_url,
            "prio": prio as u8,
            "cookies": *self.cookies.lock().unwrap(),
        });
        let line = format!("{req}\n");
        let ok = g.proc.as_mut().map(|p| p.stdin.write_all(line.as_bytes()).and_then(|_| p.stdin.flush()).is_ok());
        if ok != Some(true) {
            g.proc = None; // ölü; bir sonraki istekte yeniden başlar
            g.rids.remove(&rid);
            let waiters = g.waiting.remove(key).unwrap_or_default();
            drop(g);
            for d in waiters {
                d(Err("çözücüye yazılamadı".into()));
            }
        }
    }

    fn ensure_proc(&self, g: &mut Inner) -> Result<(), String> {
        if let Some(p) = g.proc.as_mut() {
            if matches!(p.child.try_wait(), Ok(None)) {
                return Ok(());
            }
            g.proc = None;
        }
        let dir = data_dir();
        let _ = std::fs::create_dir_all(&dir);
        let script = dir.join("resolver.py");
        if std::fs::read_to_string(&script).ok().as_deref() != Some(HELPER) {
            std::fs::write(&script, HELPER).map_err(|e| e.to_string())?;
        }
        let warm_cookies = self.cookies.lock().unwrap().clone().unwrap_or_else(|| "-".into());
        let mut child = Command::new("python3")
            .arg("-I")
            .arg(&script)
            .arg(warm_cookies)
            .current_dir(&dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("python3 başlatılamadı: {e}"))?;
        let stdin = child.stdin.take().ok_or("stdin yok")?;
        let stdout = child.stdout.take().ok_or("stdout yok")?;
        g.generation += 1;
        let generation = g.generation;
        let inner = self.inner.clone();
        thread::Builder::new()
            .name("vynz-resolver-out".into())
            .spawn(move || read_loop(inner, stdout, generation))
            .map_err(|e| e.to_string())?;
        g.proc = Some(Proc { child, stdin, generation });
        Ok(())
    }

    fn start_janitor(&self) {
        let inner = self.inner.clone();
        thread::spawn(move || loop {
            thread::sleep(Duration::from_secs(20));
            let mut g = inner.lock().unwrap();
            let idle = g.last_used.map(|t| t.elapsed() > Duration::from_secs(IDLE_SECS)).unwrap_or(true);
            if idle && g.waiting.is_empty() && g.radios.is_empty() && g.calls.is_empty() {
                if let Some(mut p) = g.proc.take() {
                    let _ = p.child.kill();
                    let _ = p.child.wait();
                }
            }
            if g.dirty {
                g.dirty = false;
                let t = now();
                let keep: HashMap<&String, &Resolved> = g.cache.iter().filter(|(_, r)| r.expire > t + EXPIRY_MARGIN).collect();
                if let Ok(raw) = serde_json::to_vec(&keep) {
                    let _ = std::fs::write(cache_file(), raw);
                }
            }
        });
    }

    /// Bir YouTube Music şarkısına benzer şarkılar (radyo).
    pub fn radio(&self, video_id: &str, prio: Prio, done: RadioDone) {
        let mut g = self.inner.lock().unwrap();
        g.last_used = Some(Instant::now());
        if let Err(e) = self.ensure_proc(&mut g) {
            drop(g);
            done(Err(e));
            return;
        }
        g.next_rid += 1;
        let rid = g.next_rid;
        g.radios.insert(rid, done);
        let line = format!("{}\n", serde_json::json!({"rid": rid, "op": "radio", "id": video_id, "prio": prio as u8}));
        let ok = g.proc.as_mut().map(|p| p.stdin.write_all(line.as_bytes()).and_then(|_| p.stdin.flush()).is_ok());
        if ok != Some(true) {
            g.proc = None;
            let d = g.radios.remove(&rid);
            drop(g);
            if let Some(d) = d {
                d(Err("çözücüye yazılamadı".into()));
            }
        }
    }

    /// Genel istek: `payload` içinde "op" ve parametreler; yanıtın "data" alanı döner.
    pub fn request(&self, mut payload: serde_json::Value, prio: Prio, done: ValueDone) {
        let mut g = self.inner.lock().unwrap();
        g.last_used = Some(Instant::now());
        if let Err(e) = self.ensure_proc(&mut g) {
            drop(g);
            done(Err(e));
            return;
        }
        g.next_rid += 1;
        let rid = g.next_rid;
        g.calls.insert(rid, done);
        payload["rid"] = rid.into();
        payload["prio"] = (prio as u8).into();
        let line = format!("{payload}\n");
        let ok = g.proc.as_mut().map(|p| p.stdin.write_all(line.as_bytes()).and_then(|_| p.stdin.flush()).is_ok());
        if ok != Some(true) {
            g.proc = None;
            let d = g.calls.remove(&rid);
            drop(g);
            if let Some(d) = d {
                d(Err("çözücüye yazılamadı".into()));
            }
        }
    }

    /// Program kapanırken yardımcıyı kapat ve önbelleği yaz.
    pub fn shutdown(&self) {
        let mut g = self.inner.lock().unwrap();
        if let Some(mut p) = g.proc.take() {
            let _ = p.child.kill();
            let _ = p.child.wait();
        }
        if let Ok(raw) = serde_json::to_vec(&g.cache) {
            let _ = std::fs::write(cache_file(), raw);
        }
    }
}

#[derive(Deserialize)]
struct Reply {
    rid: Option<u64>,
    #[serde(default)]
    ok: bool,
    #[serde(default)]
    tracks: Option<Vec<RadioTrack>>,
    #[serde(default)]
    data: Option<serde_json::Value>,
    stream: Option<String>,
    expire: Option<u64>,
    ext: Option<String>,
    error: Option<String>,
}

fn read_loop(inner: Arc<Mutex<Inner>>, stdout: std::process::ChildStdout, generation: u64) {
    for line in BufReader::new(stdout).lines() {
        let Ok(line) = line else { break };
        let Ok(rep) = serde_json::from_str::<Reply>(&line) else { continue };
        let Some(rid) = rep.rid else { continue }; // "ready" satırı
        let mut g = inner.lock().unwrap();
        if let Some(done) = g.calls.remove(&rid) {
            g.last_used = Some(Instant::now());
            drop(g);
            match (rep.ok, rep.data) {
                (true, Some(d)) => done(Ok(d)),
                _ => done(Err(rep.error.unwrap_or_else(|| "istek başarısız".into()))),
            }
            continue;
        }
        if let Some(done) = g.radios.remove(&rid) {
            g.last_used = Some(Instant::now());
            drop(g);
            match (rep.ok, rep.tracks) {
                (true, Some(t)) => done(Ok(t)),
                _ => done(Err(rep.error.unwrap_or_else(|| "radyo alınamadı".into()))),
            }
            continue;
        }
        let Some(key) = g.rids.remove(&rid) else { continue };
        let result = match (rep.ok, rep.stream) {
            (true, Some(stream)) => {
                let r = Resolved { stream, expire: rep.expire.unwrap_or(now() + 3600), ext: rep.ext };
                g.cache.insert(key.clone(), r.clone());
                g.dirty = true;
                Ok(r)
            }
            _ => Err(rep.error.unwrap_or_else(|| "çözülemedi".into())),
        };
        // Aynı parça için öncelikli bir kopya da yoldaysa ilk yanıt herkesi bitirir.
        let waiters = g.waiting.remove(&key).unwrap_or_default();
        g.last_used = Some(Instant::now());
        drop(g);
        for d in waiters {
            d(result.clone());
        }
    }
    // Yardımcı öldü. Yerine yenisi başlatıldıysa ona dokunma.
    let mut g = inner.lock().unwrap();
    if g.proc.as_ref().is_some_and(|p| p.generation != generation) {
        return;
    }
    g.proc = None;
    let waiting: Vec<_> = g.waiting.drain().collect();
    let radios: Vec<_> = g.radios.drain().map(|(_, d)| d).collect();
    let calls: Vec<_> = g.calls.drain().map(|(_, d)| d).collect();
    for d in calls {
        d(Err("çözücü kapandı".into()));
    }
    g.rids.clear();
    drop(g);
    for d in radios {
        d(Err("çözücü kapandı".into()));
    }
    for (_, list) in waiting {
        for d in list {
            d(Err("çözücü kapandı".into()));
        }
    }
}
