// Arka plan indiricisi: dinlenen parçaları disk önbelleğine indirir.
//
// Kullanıcının internetini zorlamamak için:
// - Aynı anda tek indirme, 512 KB'lık parçalarla.
// - Hız sınırı (ayarlardan; 0 = sınırsız).
// - Bilgisayardaki toplam gelen trafik, bizim indirmemiz dışında yüksekse
//   (başka bir program interneti kullanıyorsa) bekler.
// - Kotalı bağlantıda hiç indirmez.

use std::collections::{HashSet, VecDeque};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::cache;

const CHUNK: u64 = 512 * 1024;
/// Bizim dışımızdaki trafik bunu aşarsa beklenir (bayt/sn).
const BUSY_RX: u64 = 1_500_000;

struct Job {
    key: String,
    url: String,
}

#[derive(Default)]
struct Queue {
    jobs: VecDeque<Job>,
    known: HashSet<String>,
}

#[derive(Clone)]
pub struct Fetcher {
    q: Arc<(Mutex<Queue>, Condvar)>,
    /// bayt/sn; 0 = sınırsız
    limit: Arc<AtomicU64>,
    cache_limit: Arc<AtomicU64>,
    metered: Arc<AtomicBool>,
}

impl Fetcher {
    pub fn new(limit: u64, cache_limit: u64, metered: Arc<AtomicBool>) -> Fetcher {
        let f = Fetcher {
            q: Arc::new((Mutex::new(Queue::default()), Condvar::new())),
            limit: Arc::new(AtomicU64::new(limit)),
            cache_limit: Arc::new(AtomicU64::new(cache_limit)),
            metered,
        };
        let w = f.clone();
        thread::Builder::new().name("vynz-fetch".into()).spawn(move || w.run()).ok();
        f
    }

    pub fn set_limits(&self, rate: u64, cache_limit: u64) {
        self.limit.store(rate, Ordering::Relaxed);
        self.cache_limit.store(cache_limit, Ordering::Relaxed);
    }

    /// Önbelleğe alınmak üzere sıraya koy (zaten varsa ya da sıradaysa yok sayılır).
    pub fn enqueue(&self, key: &str, url: &str) {
        if self.cache_limit.load(Ordering::Relaxed) == 0 || cache::exists(key).is_some() {
            return;
        }
        let (m, cv) = &*self.q;
        let mut q = m.lock().unwrap();
        if !q.known.insert(key.to_string()) {
            return;
        }
        q.jobs.push_back(Job { key: key.into(), url: url.into() });
        cv.notify_one();
    }

    fn run(&self) {
        loop {
            let job = {
                let (m, cv) = &*self.q;
                let mut q = m.lock().unwrap();
                while q.jobs.is_empty() {
                    q = cv.wait(q).unwrap();
                }
                q.jobs.pop_front().unwrap()
            };
            let ok = self.download(&job);
            cache::finish(&job.key, ok.is_ok());
            if let Err(e) = ok {
                eprintln!("önbelleğe alınamadı {}: {e}", job.key);
                // Bir sonraki dinlemede yeniden denenebilsin.
                self.q.0.lock().unwrap().known.remove(&job.key);
            }
            cache::enforce(self.cache_limit.load(Ordering::Relaxed));
        }
    }

    fn download(&self, job: &Job) -> Result<(), String> {
        let path = cache::prepare(&job.key, true).ok_or("önbellek klasörü yok")?;
        let mut file = std::fs::File::create(&path).map_err(|e| e.to_string())?;
        let mut offset: u64 = 0;
        let mut total: Option<u64> = None;
        let mut buf = vec![0u8; 64 * 1024];
        let mut rx = RxMeter::new();
        loop {
            if self.metered.load(Ordering::Relaxed) {
                return Err("kotalı bağlantı".into());
            }
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
                let started = Instant::now();
                let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
                got += n as u64;
                rx.ours += n as u64;
                // Hız sınırı: bu okumanın hak ettiği süreyi bekle.
                let limit = self.limit.load(Ordering::Relaxed);
                if limit > 0 {
                    let want = Duration::from_secs_f64(n as f64 / limit as f64);
                    if let Some(rest) = want.checked_sub(started.elapsed()) {
                        thread::sleep(rest);
                    }
                }
            }
            if got == 0 {
                break;
            }
            offset += got;
            // Başka bir program interneti yoğun kullanıyorsa geri çekil.
            while rx.others_busy() {
                thread::sleep(Duration::from_secs(3));
            }
            if total.is_none() && got < CHUNK {
                break; // sunucu aralık bilgisi vermedi; parça kısa geldiyse son
            }
        }
        file.flush().map_err(|e| e.to_string())?;
        match total {
            Some(t) if offset < t => Err(format!("eksik: {offset}/{t}")),
            _ => Ok(()),
        }
    }
}

/// Ağ arayüzleri üzerinden toplam gelen trafiği ölçer; bizimkini çıkarır.
struct RxMeter {
    networks: sysinfo::Networks,
    last_total: u64,
    last_ours: u64,
    last_at: Instant,
    ours: u64,
}

impl RxMeter {
    fn new() -> RxMeter {
        let networks = sysinfo::Networks::new_with_refreshed_list();
        let last_total = rx_total(&networks);
        RxMeter { networks, last_total, last_ours: 0, last_at: Instant::now(), ours: 0 }
    }

    fn others_busy(&mut self) -> bool {
        let dt = self.last_at.elapsed().as_secs_f64();
        if dt < 1.0 {
            return false;
        }
        self.networks.refresh(false);
        let total = rx_total(&self.networks);
        let others = total.saturating_sub(self.last_total).saturating_sub(self.ours - self.last_ours);
        self.last_total = total;
        self.last_ours = self.ours;
        self.last_at = Instant::now();
        (others as f64 / dt) as u64 > BUSY_RX
    }
}

fn rx_total(networks: &sysinfo::Networks) -> u64 {
    networks
        .iter()
        .filter(|(name, _)| {
            let n = name.to_ascii_lowercase();
            n != "lo" && !n.starts_with("loopback")
        })
        .map(|(_, data)| data.total_received())
        .sum()
}
