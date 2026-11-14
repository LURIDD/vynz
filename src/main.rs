// vynz: çok kaynaklı müzik çalar.
// Oynatma yolu (hızlıdan yavaşa):
//   1. Disk önbelleği (daha önce sonuna kadar dinlenmiş parça)   ~0.3 sn
//   2. Önceden çözülmüş akış adresi                               ~1.2 sn
//   3. Çözücü yardımcısı ("şimdi" önceliği)                       ~3-5 sn
//   4. Yedek: mpv'nin yt-dlp kancası                              ~8-10 sn

mod art;
mod cache;
mod fetcher;
mod model;
mod player;
mod resolve;
mod settings;
mod sources;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

use slint::{Color, ComponentHandle, Image, Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, Timer, TimerMode, VecModel, Weak};

use model::{Section, Source, TrackInfo};
use player::{Cmd, Ev, Player};
use resolve::{Prio, Resolver};
use settings::Settings;
use sources::{soundcloud::SoundCloud, ytm::Ytm, Provider};

slint::include_modules!();

const NEUTRAL: Color = Color::from_rgb_u8(0xE4, 0xE6, 0xEB);
const HOVER_DELAY: Duration = Duration::from_millis(450);
/// Kuyrukta önceden çözülecek parça sayısı.
const PREFETCH_AHEAD: usize = 2;

struct State {
    providers: Vec<Arc<dyn Provider>>,
    player: Player,
    resolver: Resolver,
    settings: Settings,
    art_tx: mpsc::Sender<(String, String)>,
    shelves: Vec<Section>,
    results: Vec<TrackInfo>,
    queue: Vec<TrackInfo>,
    index: Option<usize>,
    /// mpv listesine sıradaki olarak eklenen kuyruk indeksi.
    appended: Option<usize>,
    fetcher: fetcher::Fetcher,
    /// Çalan parça önbelleğe alma sırasına kondu mu (play_seq ile).
    cache_queued: u64,
    play_seq: u64,
    /// Çalan parça mpv'ye yüklendi mi? Sıradaki ancak bundan sonra eklenir;
    /// yoksa eski parçanın arkasına eklenip yeni yüklemede silinirdi.
    loaded: bool,
    fallback_tried: bool,
    search_seq: u64,
    requested: HashSet<String>,
    toast_timer: Timer,
    hover_timer: Timer,
    metered: Arc<AtomicBool>,
    on_battery: Arc<AtomicBool>,
}

/// VYNZ_DEBUG=1 ile zaman damgalı olay günlüğü (stderr): hız ölçümü için.
fn dbg(msg: &str) {
    use std::sync::OnceLock;
    static T0: OnceLock<std::time::Instant> = OnceLock::new();
    static ON: OnceLock<bool> = OnceLock::new();
    if *ON.get_or_init(|| std::env::var_os("VYNZ_DEBUG").is_some()) {
        let t = T0.get_or_init(std::time::Instant::now).elapsed().as_secs_f64();
        eprintln!("[{t:8.3}] {msg}");
    }
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    static ARTS: RefCell<HashMap<String, (Image, Color)>> = RefCell::new(HashMap::new());
}

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> R {
    STATE.with(|s| f(s.borrow_mut().as_mut().expect("durum hazır değil")))
}

fn source_name(src: Source) -> &'static str {
    match src {
        Source::Ytm => "YouTube Music",
        Source::SoundCloud => "SoundCloud",
        Source::Spotify => "Spotify",
    }
}

fn provider(st: &State, src: Source) -> Option<Arc<dyn Provider>> {
    st.providers.iter().find(|p| p.source() == src).cloned()
}

fn page_url(st: &State, t: &TrackInfo) -> Option<String> {
    provider(st, t.source)?.stream_url(t).ok()
}

fn ui_track(t: &TrackInfo) -> Track {
    let (art, has_art) = ARTS
        .with(|a| a.borrow().get(&t.key()).map(|(i, _)| (i.clone(), true)))
        .unwrap_or_default();
    Track {
        key: t.key().into(),
        title: t.title.as_str().into(),
        artist: t.artist.as_str().into(),
        album: t.album.as_str().into(),
        duration: t.duration,
        source: t.source.index(),
        art,
        has_art,
    }
}

fn model_of(tracks: &[TrackInfo]) -> ModelRc<Track> {
    ModelRc::new(VecModel::from(tracks.iter().map(ui_track).collect::<Vec<_>>()))
}

fn request_art(st: &mut State, t: &TrackInfo) {
    let key = t.key();
    if t.thumb.is_empty() || ARTS.with(|a| a.borrow().contains_key(&key)) || !st.requested.insert(key.clone()) {
        return;
    }
    let _ = st.art_tx.send((key, t.thumb.clone()));
}

fn toast(ui: &AppWindow, msg: &str) {
    ui.set_toast(msg.into());
    let weak = ui.as_weak();
    with_state(|st| {
        st.toast_timer.start(TimerMode::SingleShot, Duration::from_secs(4), move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_toast("".into());
            }
        })
    });
}

// ---------- Kapaklar ----------

fn start_art_pool(weak: Weak<AppWindow>) -> mpsc::Sender<(String, String)> {
    let (tx, rx) = mpsc::channel::<(String, String)>();
    let rx = Arc::new(Mutex::new(rx));
    for _ in 0..6 {
        let rx = rx.clone();
        let weak = weak.clone();
        thread::spawn(move || loop {
            let job = rx.lock().unwrap().recv();
            let Ok((key, url)) = job else { break };
            match art::load(&key, &url) {
                Ok(a) => {
                    let _ = weak.upgrade_in_event_loop(move |ui| apply_art(&ui, key, a));
                }
                Err(e) => eprintln!("kapak {key}: {e}"),
            }
        });
    }
    tx
}

fn update_rows(model: &ModelRc<Track>, key: &str, img: &Image) {
    for i in 0..model.row_count() {
        if let Some(mut t) = model.row_data(i) {
            if t.key == key {
                t.art = img.clone();
                t.has_art = true;
                model.set_row_data(i, t);
            }
        }
    }
}

fn apply_art(ui: &AppWindow, key: String, a: art::Art) {
    let buf = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&a.rgba, a.width, a.height);
    let img = Image::from_rgba8(buf);
    let color = Color::from_rgb_u8(a.accent.0, a.accent.1, a.accent.2);
    ARTS.with(|m| m.borrow_mut().insert(key.clone(), (img.clone(), color)));

    let shelves = ui.get_shelves();
    for i in 0..shelves.row_count() {
        if let Some(s) = shelves.row_data(i) {
            update_rows(&s.items, &key, &img);
        }
    }
    update_rows(&ui.get_results(), &key, &img);
    update_rows(&ui.get_queue(), &key, &img);

    let mut now = ui.get_now();
    if ui.get_has_now() && now.key == key {
        now.art = img;
        now.has_art = true;
        ui.set_now(now);
        ui.global::<Theme>().set_accent(color);
    }
}

// ---------- Oynatma ----------

fn set_now_ui(ui: &AppWindow, t: &TrackInfo, i: usize) {
    ui.set_has_now(true);
    ui.set_now(ui_track(t));
    ui.set_queue_index(i as i32);
    ui.set_position(0.0);
    ui.set_duration(t.duration);
    ui.set_playing(true);
    let accent = ARTS.with(|a| a.borrow().get(&t.key()).map(|(_, c)| *c)).unwrap_or(NEUTRAL);
    ui.global::<Theme>().set_accent(accent);
}

fn play_index(ui: &AppWindow, i: usize) {
    let weak = ui.as_weak();
    let res = with_state(|st| {
        let t = st.queue.get(i)?.clone();
        st.index = Some(i);
        st.appended = None;
        st.loaded = false;
        st.fallback_tried = false;
        st.play_seq += 1;
        request_art(st, &t);
        let key = t.key();
        if let Some(path) = cache::lookup(&key) {
            dbg(&format!("çal {key}: disk önbelleği"));
            st.player.send(Cmd::Load { url: path.to_string_lossy().into_owned(), record: None });
            st.loaded = true;
        } else if let Some(r) = st.resolver.cached(&key) {
            dbg(&format!("çal {key}: önceden çözülmüş"));
            st.player.send(Cmd::Load { url: r.stream, record: None });
            st.loaded = true;
        } else {
            dbg(&format!("çal {key}: çözülüyor"));
            let page = page_url(st, &t)?;
            let seq = st.play_seq;
            let w = weak.clone();
            let k = key.clone();
            st.resolver.resolve(
                &key,
                &page,
                Prio::Now,
                Some(Box::new(move |res| {
                    let _ = w.upgrade_in_event_loop(move |ui| on_resolved_now(&ui, seq, k, res));
                })),
            );
        }
        Some(t)
    });
    let Some(t) = res else { return };
    set_now_ui(ui, &t, i);
    ui.set_buffering(true);
    schedule_prefetch(ui);
}

fn on_resolved_now(ui: &AppWindow, seq: u64, key: String, res: Result<resolve::Resolved, String>) {
    let stale = with_state(|st| st.play_seq != seq);
    if stale {
        return;
    }
    dbg(&format!("çözüldü {key}: {}", if res.is_ok() { "tamam" } else { "hata" }));
    with_state(|st| match res {
        Ok(r) => {
            st.player.send(Cmd::Load { url: r.stream, record: None });
            st.loaded = true;
        }
        Err(e) => {
            eprintln!("çözülemedi ({key}): {e}; yedek yol");
            st.fallback_tried = true;
            let page = st.index.and_then(|i| st.queue.get(i).cloned()).and_then(|t| page_url(st, &t));
            if let Some(page) = page {
                st.player.send(Cmd::Load { url: page, record: None });
                st.loaded = true;
            }
        }
    });
    maybe_append_next(ui);
}

/// Kuyruktaki sonraki parçaları önceden çöz; ilki hazır olunca mpv'ye sıradaki olarak ekle.
fn schedule_prefetch(ui: &AppWindow) {
    let weak = ui.as_weak();
    with_state(|st| {
        if !st.settings.prefetch {
            return;
        }
        let Some(i) = st.index else { return };
        for j in i + 1..=(i + PREFETCH_AHEAD).min(st.queue.len().saturating_sub(1)) {
            let t = st.queue[j].clone();
            let key = t.key();
            request_art(st, &t);
            if cache::lookup(&key).is_some() || st.resolver.cached(&key).is_some() {
                continue;
            }
            let Some(page) = page_url(st, &t) else { continue };
            let w = weak.clone();
            st.resolver.resolve(
                &key,
                &page,
                Prio::Soon,
                Some(Box::new(move |_| {
                    let _ = w.upgrade_in_event_loop(|ui| maybe_append_next(&ui));
                })),
            );
        }
    });
    maybe_append_next(ui);
}

fn maybe_append_next(_ui: &AppWindow) {
    with_state(|st| {
        if !st.settings.prefetch || st.metered.load(Ordering::Relaxed) {
            return;
        }
        let Some(i) = st.index else { return };
        let next = i + 1;
        if !st.loaded || st.appended == Some(next) || next >= st.queue.len() {
            return;
        }
        let key = st.queue[next].key();
        let (url, record) = if let Some(p) = cache::lookup(&key) {
            (p.to_string_lossy().into_owned(), None)
        } else if let Some(r) = st.resolver.cached(&key) {
            (r.stream, None)
        } else {
            return; // çözülünce tekrar çağrılır
        };
        dbg(&format!("sıradaki eklendi: {}", st.queue[next].key()));
        st.player.send(Cmd::Append { url, record });
        st.appended = Some(next);
    });
}

fn step(ui: &AppWindow, delta: i64) {
    let (idx, len) = with_state(|st| (st.index, st.queue.len()));
    let Some(idx) = idx else { return };
    let next = idx as i64 + delta;
    if next >= 0 && (next as usize) < len {
        play_index(ui, next as usize);
    } else if delta > 0 {
        ui.set_playing(false);
    }
}

fn on_ended(ui: &AppWindow) {
    let advanced = with_state(|st| {
        let i = st.index?;
        if st.appended == Some(i + 1) {
            // mpv sıradakine kendisi geçti (ön-tampon): yalnız durumu güncelle.
            dbg("ön-tampondan sıradakine geçildi");
            st.index = Some(i + 1);
            st.appended = None;
            st.play_seq += 1;
            st.loaded = true; // mpv zaten yükledi
            st.fallback_tried = false;
            return Some(Some((st.queue[i + 1].clone(), i + 1)));
        }
        Some(None)
    });
    match advanced {
        Some(Some((t, i))) => {
            set_now_ui(ui, &t, i);
            schedule_prefetch(ui);
        }
        Some(None) => step(ui, 1),
        None => {}
    }
}

fn on_error(ui: &AppWindow, e: String) {
    let retried = with_state(|st| {
        if st.fallback_tried {
            return false;
        }
        st.fallback_tried = true;
        st.appended = None;
        let page = st.index.and_then(|i| st.queue.get(i).cloned()).and_then(|t| page_url(st, &t));
        match page {
            Some(page) => {
                st.player.send(Cmd::Load { url: page, record: None });
                st.loaded = true;
                true
            }
            None => false,
        }
    });
    if retried {
        eprintln!("akış açılamadı ({e}), yedek yol deneniyor");
    } else {
        toast(ui, &format!("Çalınamadı: {e}"));
        step(ui, 1);
    }
}

fn on_player_event(ui: &AppWindow, ev: Ev) {
    match ev {
        Ev::Time(t) => {
            ui.set_position(t as f32);
            if t >= 20.0 {
                queue_for_cache();
            }
        }
        Ev::Duration(d) => ui.set_duration(d as f32),
        Ev::Paused(p) => ui.set_playing(!p && ui.get_has_now()),
        Ev::Buffering(b) => {
            if !b {
                dbg("ses başladı");
            }
            ui.set_buffering(b)
        }
        Ev::Started => {}
        Ev::Seeked => {}
        Ev::Ended => {
            dbg("parça bitti");
            on_ended(ui)
        }
        Ev::Error(e) => on_error(ui, e),
    }
}

/// 20 saniyeden uzun dinlenen parçayı arka planda önbelleğe al.
fn queue_for_cache() {
    with_state(|st| {
        if st.cache_queued == st.play_seq || st.settings.cache_gb <= 0.0 {
            return;
        }
        st.cache_queued = st.play_seq;
        let Some(t) = st.index.and_then(|i| st.queue.get(i)) else { return };
        let key = t.key();
        if let Some(r) = st.resolver.cached(&key) {
            // HLS (SoundCloud) parça listesidir, doğrudan indirilemez; şimdilik atlanır.
            if !r.stream.contains(".m3u8") && !r.stream.contains("/playlist") {
                dbg(&format!("önbelleğe alınacak: {key}"));
                st.fetcher.enqueue(&key, &r.stream);
            }
        }
    });
}

// ---------- Tahmine dayalı ön-çözme ----------

/// Kullanıcının tıklayabileceği parçayı arka planda çöz (pilde ve kotalı bağlantıda yapılmaz).
fn speculate(st: &mut State, t: &TrackInfo) {
    dbg(&format!("tahmin isteği {}", t.key()));
    if !st.settings.prefetch || st.on_battery.load(Ordering::Relaxed) || st.metered.load(Ordering::Relaxed) {
        return;
    }
    let key = t.key();
    if cache::lookup(&key).is_some() || st.resolver.cached(&key).is_some() || st.resolver.is_pending(&key) {
        return;
    }
    if let Some(page) = page_url(st, t) {
        st.resolver.resolve(&key, &page, Prio::Maybe, None);
    }
}

fn on_hover(t: Option<TrackInfo>) {
    with_state(|st| match t {
        Some(t) => st.hover_timer.start(TimerMode::SingleShot, HOVER_DELAY, move || {
            with_state(|st| speculate(st, &t));
        }),
        None => st.hover_timer.stop(),
    });
}

fn start_env_monitor(metered: Arc<AtomicBool>, on_battery: Arc<AtomicBool>) {
    thread::spawn(move || loop {
        let m = std::process::Command::new("nmcli")
            .args(["-t", "-f", "GENERAL.METERED", "dev", "show"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).lines().any(|l| l.contains(":yes")))
            .unwrap_or(false);
        metered.store(m, Ordering::Relaxed);
        let mut mains_seen = false;
        let mut mains_online = false;
        if let Ok(rd) = std::fs::read_dir("/sys/class/power_supply") {
            for e in rd.flatten() {
                let p = e.path();
                if std::fs::read_to_string(p.join("type")).is_ok_and(|t| t.trim() == "Mains") {
                    mains_seen = true;
                    mains_online |= std::fs::read_to_string(p.join("online")).is_ok_and(|v| v.trim() == "1");
                }
            }
        }
        on_battery.store(mains_seen && !mains_online, Ordering::Relaxed);
        thread::sleep(Duration::from_secs(60));
    });
}

// ---------- Veri yükleme ----------

fn load_home(ui: &AppWindow) {
    let providers = with_state(|st| st.providers.clone());
    let weak = ui.as_weak();
    thread::spawn(move || {
        let mut all = Vec::new();
        let mut errors = Vec::new();
        for p in providers {
            match p.home() {
                Ok(mut s) => all.append(&mut s),
                Err(e) => errors.push(format!("{}: {e}", source_name(p.source()))),
            }
        }
        let _ = weak.upgrade_in_event_loop(move |ui| {
            let shelves: Vec<Shelf> = all
                .iter()
                .map(|s| Shelf { title: s.title.as_str().into(), items: model_of(&s.items) })
                .collect();
            with_state(|st| {
                for s in &all {
                    for t in s.items.iter().take(8) {
                        request_art(st, t);
                    }
                }
                // İlk rafın ilk birkaç parçası büyük ihtimalle tıklanır.
                if let Some(first) = all.first() {
                    for t in first.items.iter().take(3) {
                        speculate(st, t);
                    }
                }
                st.shelves = all;
            });
            ui.set_shelves(ModelRc::new(VecModel::from(shelves)));
            ui.set_home_loading(false);
            if let Some(e) = errors.first() {
                toast(&ui, &format!("Öneriler alınamadı: {e}"));
            }
        });
    });
}

fn search(ui: &AppWindow, query: String) {
    let (providers, seq) = with_state(|st| {
        st.search_seq += 1;
        (st.providers.clone(), st.search_seq)
    });
    ui.set_searching(true);
    ui.set_last_query(query.as_str().into());
    let weak = ui.as_weak();
    thread::spawn(move || {
        // Kaynaklar paralel sorgulanır; sonuçlar kaynak sırasıyla birleşir.
        let outcomes: Vec<_> = thread::scope(|s| {
            let handles: Vec<_> = providers
                .iter()
                .map(|p| s.spawn(|| (p.source(), p.search(&query, 20))))
                .collect();
            handles.into_iter().filter_map(|h| h.join().ok()).collect()
        });
        let mut found = Vec::new();
        let mut errors = Vec::new();
        for (src, r) in outcomes {
            match r {
                Ok(mut r) => found.append(&mut r),
                Err(e) => errors.push(format!("{}: {e}", source_name(src))),
            }
        }
        let _ = weak.upgrade_in_event_loop(move |ui| {
            if with_state(|st| st.search_seq) != seq {
                return; // daha yeni bir arama var
            }
            let model = model_of(&found);
            with_state(|st| {
                for t in &found {
                    request_art(st, t);
                }
                for t in found.iter().take(2) {
                    speculate(st, t);
                }
                st.results = found;
            });
            ui.set_results(model);
            ui.set_searching(false);
            if let Some(e) = errors.first() {
                toast(&ui, &format!("Arama hatası: {e}"));
            }
        });
    });
}

// ---------- Ayarlar ----------

fn human_bytes(b: u64) -> String {
    let mb = b as f64 / (1024.0 * 1024.0);
    if mb >= 1024.0 { format!("{:.1} GB", mb / 1024.0) } else { format!("{mb:.0} MB") }
}

fn refresh_usage(ui: &AppWindow) {
    let weak = ui.as_weak();
    thread::spawn(move || {
        let used = cache::usage();
        let _ = weak.upgrade_in_event_loop(move |ui| ui.set_cache_usage(human_bytes(used).into()));
    });
}

fn load_settings_ui(ui: &AppWindow, s: &Settings) {
    ui.set_cache_index(s.cache_index());
    ui.set_prefetch_on(s.prefetch);
    ui.set_readahead_index(s.readahead_index());
    ui.set_cookies_index(s.cookies_index());
    ui.set_download_index(s.download_index());
}

fn apply_settings(ui: &AppWindow) {
    let s = Settings {
        cache_gb: settings::CACHE_GB[ui.get_cache_index().clamp(0, 4) as usize],
        prefetch: ui.get_prefetch_on(),
        readahead_secs: settings::READAHEAD_SECS[ui.get_readahead_index().clamp(0, 3) as usize],
        cookies_browser: settings::COOKIE_BROWSERS[ui.get_cookies_index().clamp(0, 1) as usize].map(str::to_string),
        download_kbps: settings::DOWNLOAD_KBPS[ui.get_download_index().clamp(0, 3) as usize],
    };
    with_state(|st| {
        st.player.send(Cmd::Readahead(s.readahead_secs));
        st.resolver.set_cookies(s.cookies_browser.clone());
        if !s.prefetch && st.appended.is_some() {
            st.player.send(Cmd::ClearNext);
            st.appended = None;
        }
        let limit = s.cache_bytes();
        st.fetcher.set_limits(s.download_kbps as u64 * 1024, limit);
        thread::spawn(move || cache::enforce(limit));
        if let Err(e) = s.save() {
            eprintln!("ayarlar kaydedilemedi: {e}");
        }
        st.settings = s;
    });
    refresh_usage(ui);
}

fn play_list(ui: &AppWindow, list: Vec<TrackInfo>, i: usize) {
    let model = model_of(&list);
    with_state(|st| {
        for t in &list {
            request_art(st, t);
        }
        st.queue = list;
    });
    ui.set_queue(model);
    play_index(ui, i);
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ui = AppWindow::new()?;
    if let Ok(t) = std::env::var("VYNZ_TITLE") {
        ui.set_window_title(t.into()); // testte pencereyi ayırt etmek için
    }
    let weak = ui.as_weak();
    let settings = Settings::load();
    load_settings_ui(&ui, &settings);

    let player = {
        let weak = weak.clone();
        Player::spawn(settings.readahead_secs, move |ev| {
            let _ = weak.upgrade_in_event_loop(move |ui| on_player_event(&ui, ev));
        })?
    };
    player.send(Cmd::Volume(ui.get_volume() as f64));

    let metered = Arc::new(AtomicBool::new(false));
    let on_battery = Arc::new(AtomicBool::new(false));
    start_env_monitor(metered.clone(), on_battery.clone());
    let resolver = Resolver::new(settings.cookies_browser.clone());
    let fetcher = fetcher::Fetcher::new(settings.download_kbps as u64 * 1024, settings.cache_bytes(), metered.clone());
    {
        let limit = settings.cache_bytes();
        thread::spawn(move || cache::enforce(limit));
    }

    STATE.with(|s| {
        *s.borrow_mut() = Some(State {
            providers: vec![Arc::new(Ytm), Arc::new(SoundCloud)],
            player,
            resolver: resolver.clone(),
            settings,
            art_tx: start_art_pool(weak.clone()),
            shelves: Vec::new(),
            results: Vec::new(),
            queue: Vec::new(),
            index: None,
            appended: None,
            fetcher: fetcher.clone(),
            cache_queued: 0,
            play_seq: 0,
            loaded: false,
            fallback_tried: false,
            search_seq: 0,
            requested: HashSet::new(),
            toast_timer: Timer::default(),
            hover_timer: Timer::default(),
            metered,
            on_battery,
        })
    });

    ui.on_play_shelf({
        let weak = weak.clone();
        move |si, i| {
            let Some(ui) = weak.upgrade() else { return };
            let list = with_state(|st| st.shelves.get(si as usize).map(|s| s.items.clone()));
            if let Some(list) = list {
                play_list(&ui, list, i as usize);
            }
        }
    });
    ui.on_play_result({
        let weak = weak.clone();
        move |i| {
            let Some(ui) = weak.upgrade() else { return };
            let list = with_state(|st| st.results.clone());
            play_list(&ui, list, i as usize);
        }
    });
    ui.on_play_queue({
        let weak = weak.clone();
        move |i| {
            if let Some(ui) = weak.upgrade() {
                play_index(&ui, i as usize);
            }
        }
    });
    ui.on_hover_shelf(|si, i, on| {
        let t = if on {
            with_state(|st| st.shelves.get(si as usize).and_then(|s| s.items.get(i as usize).cloned()))
        } else {
            None
        };
        on_hover(t);
    });
    ui.on_hover_result(|i, on| {
        let t = if on { with_state(|st| st.results.get(i as usize).cloned()) } else { None };
        on_hover(t);
    });
    ui.on_search({
        let weak = weak.clone();
        move |q| {
            if let Some(ui) = weak.upgrade() {
                search(&ui, q.trim().to_string());
            }
        }
    });
    ui.on_toggle_play({
        let weak = weak.clone();
        move || {
            let Some(ui) = weak.upgrade() else { return };
            let playing = ui.get_playing();
            with_state(|st| st.player.send(Cmd::SetPause(playing)));
        }
    });
    ui.on_seek(|secs| with_state(|st| st.player.send(Cmd::Seek(secs as f64))));
    ui.on_set_volume(|v| with_state(|st| st.player.send(Cmd::Volume(v as f64))));
    ui.on_next({
        let weak = weak.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                step(&ui, 1)
            }
        }
    });
    ui.on_prev({
        let weak = weak.clone();
        move || {
            let Some(ui) = weak.upgrade() else { return };
            if ui.get_position() > 3.0 {
                with_state(|st| st.player.send(Cmd::Seek(0.0)));
            } else {
                step(&ui, -1)
            }
        }
    });
    ui.on_settings_changed({
        let weak = weak.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                apply_settings(&ui);
            }
        }
    });
    ui.on_clear_cache({
        let weak = weak.clone();
        move || {
            let Some(ui) = weak.upgrade() else { return };
            thread::spawn(cache::clear);
            ui.set_cache_usage("0 MB".into());
            toast(&ui, "Önbellek temizlendi");
        }
    });

    refresh_usage(&ui);
    load_home(&ui);
    ui.run()?;
    resolver.shutdown();
    Ok(())
}
