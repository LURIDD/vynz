// İndirilenler: kullanıcının çevrimdışı dinlemek için kalıcı olarak indirdiği şarkılar.
//
// Dosyalar veri klasöründe (~/.local/share/vynz/downloads) durur; önbellek sınırı ve
// süre temizliği bunlara dokunmaz. cache::exists/lookup önce buraya bakar, böylece
// çalma, ön-hazırlık ve arka plan önbelleği indirilmiş şarkıyı kendiliğinden kullanır.
// Kapaklar da yanına kaydedilir (çevrimdışıyken kapak önbelleği silinmiş olabilir).
//
// İndirme tek iş parçacığında, sırayla; kullanıcı istediği için hız sınırı yoktur.
// Akış adresi ana iş parçacığında çözülür (resolver), buraya adresle gelir.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

use crate::model::TrackInfo;

const CHUNK: u64 = 1024 * 1024;

pub fn dir() -> PathBuf {
    crate::paths::data_dir().join("downloads")
}

fn index_path() -> PathBuf {
    dir().join("index.json")
}

pub fn safe(key: &str) -> String {
    key.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
}

pub fn audio_path(key: &str) -> PathBuf {
    dir().join(format!("{}.mka", safe(key)))
}

pub fn cover_path(key: &str) -> PathBuf {
    dir().join(format!("{}.jpg", safe(key)))
}

/// İndirilmiş ses dosyası (tamamlanmışsa).
pub fn path(key: &str) -> Option<PathBuf> {
    let p = audio_path(key);
    (std::fs::metadata(&p).ok()?.len() >= 64 * 1024).then_some(p)
}

pub fn load() -> Vec<TrackInfo> {
    std::fs::read(index_path())
        .ok()
        .and_then(|b| serde_json::from_slice::<Vec<TrackInfo>>(&b).ok())
        .unwrap_or_default()
        .into_iter()
        .filter(|t| path(&t.key()).is_some())
        .collect()
}

pub fn save(list: &[TrackInfo]) {
    let _ = std::fs::create_dir_all(dir());
    if let Ok(b) = serde_json::to_vec_pretty(list) {
        let tmp = index_path().with_extension("json.tmp");
        if std::fs::write(&tmp, b).is_ok() {
            let _ = std::fs::rename(tmp, index_path());
        }
    }
}

pub fn remove_files(key: &str) {
    let _ = std::fs::remove_file(audio_path(key));
    let _ = std::fs::remove_file(cover_path(key));
}

/// Toplam boyut (bayt).
pub fn usage() -> u64 {
    std::fs::read_dir(dir())
        .map(|rd| rd.flatten().filter_map(|e| e.metadata().ok()).filter(|m| m.is_file()).map(|m| m.len()).sum())
        .unwrap_or(0)
}

pub enum Event {
    /// (anahtar, 0..1)
    Progress(String, f32),
    Done(TrackInfo),
    Failed(TrackInfo, String),
}

struct Job {
    track: TrackInfo,
    url: String,
}

#[derive(Clone)]
pub struct Downloader {
    q: Arc<(Mutex<VecDeque<Job>>, Condvar)>,
}

impl Downloader {
    pub fn new(on_event: impl Fn(Event) + Send + 'static) -> Downloader {
        let d = Downloader { q: Arc::new((Mutex::new(VecDeque::new()), Condvar::new())) };
        let w = d.clone();
        thread::Builder::new().name("vynz-download".into()).spawn(move || w.run(on_event)).ok();
        d
    }

    pub fn enqueue(&self, track: TrackInfo, url: String) {
        let (m, cv) = &*self.q;
        m.lock().unwrap().push_back(Job { track, url });
        cv.notify_one();
    }

    fn run(&self, on_event: impl Fn(Event)) {
        loop {
            let job = {
                let (m, cv) = &*self.q;
                let mut q = m.lock().unwrap();
                while q.is_empty() {
                    q = cv.wait(q).unwrap();
                }
                q.pop_front().unwrap()
            };
            let key = job.track.key();
            let res = fetch(&job, |p| on_event(Event::Progress(key.clone(), p)));
            match res {
                Ok(()) => {
                    save_cover(&job.track);
                    on_event(Event::Done(job.track));
                }
                Err(e) => {
                    let _ = std::fs::remove_file(audio_path(&key).with_extension("part"));
                    on_event(Event::Failed(job.track, e));
                }
            }
        }
    }
}

fn save_cover(t: &TrackInfo) {
    if t.thumb.is_empty() {
        return;
    }
    if let Ok(mut r) = ureq::get(&t.thumb).call() {
        if let Ok(bytes) = r.body_mut().read_to_vec() {
            let _ = std::fs::write(cover_path(&t.key()), bytes);
        }
    }
}

fn fetch(job: &Job, progress: impl Fn(f32)) -> Result<(), String> {
    std::fs::create_dir_all(dir()).map_err(|e| e.to_string())?;
    let key = job.track.key();
    let dest = audio_path(&key);
    let part = dest.with_extension("part");
    // Önbellekte tam kayıt varsa indirmeye gerek yok.
    if let Some(c) = crate::cache::exists_in_cache(&key) {
        std::fs::copy(&c, &part).map_err(|e| e.to_string())?;
        std::fs::rename(&part, &dest).map_err(|e| e.to_string())?;
        progress(1.0);
        return Ok(());
    }
    let mut file = std::fs::File::create(&part).map_err(|e| e.to_string())?;
    let mut offset: u64 = 0;
    let mut total: Option<u64> = None;
    let mut buf = vec![0u8; 64 * 1024];
    let mut last_report = 0u64;
    loop {
        if let Some(t) = total {
            if offset >= t {
                break;
            }
        }
        let end = offset + CHUNK - 1;
        let mut resp = ureq::get(&job.url)
            .header("Range", &format!("bytes={offset}-{end}"))
            .call()
            .map_err(|e| e.to_string())?;
        if total.is_none() {
            total = resp
                .headers()
                .get("content-range")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.rsplit('/').next())
                .and_then(|v| v.parse().ok());
        }
        let mut reader = resp.body_mut().as_reader();
        let mut got = 0u64;
        loop {
            let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            got += n as u64;
        }
        if got == 0 {
            break;
        }
        offset += got;
        if let Some(t) = total {
            if offset - last_report >= 512 * 1024 || offset >= t {
                last_report = offset;
                progress((offset as f32 / t as f32).min(1.0));
            }
        }
        if total.is_none() && got < CHUNK {
            break;
        }
    }
    file.flush().map_err(|e| e.to_string())?;
    match total {
        Some(t) if offset < t => Err(format!("eksik: {offset}/{t}")),
        _ if offset < 64 * 1024 => Err("dosya çok küçük".into()),
        _ => std::fs::rename(&part, &dest).map_err(|e| e.to_string()),
    }
}
