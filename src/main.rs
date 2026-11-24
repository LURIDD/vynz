#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// vynz: çok kaynaklı müzik çalar.
// Oynatma yolu (hızlıdan yavaşa):
//   1. Disk önbelleği (daha önce sonuna kadar dinlenmiş parça)   ~0.3 sn
//   2. Önceden çözülmüş akış adresi                               ~1.2 sn
//   3. Çözücü yardımcısı ("şimdi" önceliği)                       ~3-5 sn
//   4. Yedek: mpv'nin yt-dlp kancası                              ~8-10 sn

mod ambience;
mod art;
mod cache;
mod discord;
mod downloads;
mod fetcher;
mod history;
mod import;
mod library;
mod lyrics;
mod measure;
mod model;
mod mpris;
mod paths;
mod player;
mod recommend;
mod resolve;
mod runtime;
mod settings;
mod sources;
mod stats;
mod tray;
mod updater;
mod viz;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

use slint::{Color, ComponentHandle, Image, Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, Timer, TimerMode, VecModel, Weak};

use model::{Card, Section, Source, TrackInfo};
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
    /// Açık sayfanın kapakları: arka plan kuyruğunun önüne geçer
    art_hot: mpsc::Sender<(String, String)>,
    shelves: Vec<Section>,
    results: Vec<TrackInfo>,
    queue: Vec<TrackInfo>,
    index: Option<usize>,
    /// mpv listesine sıradaki olarak eklenen kuyruk indeksi.
    appended: Option<usize>,
    fetcher: fetcher::Fetcher,
    /// İndirilenler (kalıcı, çevrimdışı) ve süren indirmeler (anahtar -> 0..1)
    dl_list: Vec<TrackInfo>,
    dl_pending: Vec<(TrackInfo, f32)>,
    downloader: downloads::Downloader,
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
    /// Son yıkıcı işlemin tersi; bildirim görünür kaldıkça "Geri al" ile uygulanır.
    undo: Option<Undo>,
    hover_timer: Timer,
    metered: Arc<AtomicBool>,
    on_battery: Arc<AtomicBool>,
    imported: Option<import::Fetched>,
    import_origin: String,
    playlists: Vec<library::Playlist>,
    current_pl: Option<usize>,
    /// Liste içi filtre aktifken ekrandaki satır -> gerçek playlist.tracks indeksi
    pl_visible: Vec<usize>,
    pl_filter: String,
    /// Söz önbelleği: None = bulunamadı.
    lyrics: HashMap<String, Option<lyrics::Lyrics>>,
    lyr_pending: HashSet<String>,
    lyr_line: Option<usize>,
    lyr_spoken: usize,
    /// Son yazılan dolum bölümü: (kelime, hedef‰, duraklatılmış mı)
    lyr_seg: Option<(usize, i32, bool)>,
    /// Sözlerde kullanılan son konum: ses saatinin küçük geri sıçramaları dolumu geri sarmasın.
    lyr_hi: f32,
    /// Etkin satır yeniden kurulsun (yeni söz, genişlik değişti)
    lyr_force: bool,
    /// Şarkıya özel söz kaydırma (sn).
    lyr_offsets: HashMap<String, f32>,
    history: history::History,
    /// Çalan parçanın dinlenme sayacı: (parça, dinlenen sn, son konum)
    listen: Option<(TrackInfo, f32, f32)>,
    /// Karıştırma açıkken kuyruğun özgün sırası
    shuffle_orig: Option<Vec<TrackInfo>>,
    /// 0 kapalı, 1 tüm liste, 2 tek şarkı
    repeat: i32,
    radio_busy: bool,
    /// Sağ tık menüsünün hedefi
    menu: Option<TrackInfo>,
    /// Sanatçı seçim menüsü: (adlar, şarkının videoId'si)
    artist_pick: (Vec<String>, String),
    /// Kurulmakta olan güncelleme (yeniden dene / atla için)
    pending_update: Option<updater::Update>,
    /// Kişisel karışım ve keşif listesi (ana sayfadaki vitrin kartları)
    mix: Vec<TrackInfo>,
    discover: Vec<TrackInfo>,
    /// Açık koleksiyon (liste/albüm/sanatçı): başlık, kart, şarkılar
    coll: Option<(Card, Vec<TrackInfo>)>,
    /// Açık sanatçı sayfası: kart, popüler şarkılar, raflar
    artist: Option<(Card, Vec<TrackInfo>, Vec<Section>)>,
    /// Yüklenmesi beklenen afişin anahtarı
    banner_key: String,
    /// Ruh hâli sayfasındaki listeler
    browse: Vec<Card>,
    /// Son ana sayfa verisi (kayıtlı listeler değişince yeniden dizmek için)
    home: HomeData,
    /// Canlı spektrum (yalnız profil ekranında, çalarken)
    viz: Option<viz::Viz>,
    /// Profildeki en çok dinlenen şarkılar
    prof_tracks: Vec<TrackInfo>,
    /// Seçilen dosyadaki listeler (birden çoksa kullanıcı seçer)
    file_lists: Vec<import::FileList>,
    mpris: mpris::Mpris,
    /// Atlama sonrası ilk konumda MPRIS'e "Seeked" bildirilecek
    seek_pending: bool,
    /// Son konum bilgisi ve geldiği an: sözlerde ara konum hesabı için
    last_pos: (f32, std::time::Instant),
    /// Sayfa geçmişi (geri düğmesi)
    nav: Vec<String>,
    /// Üst sayfa yığını (en çok 2: ruh hâli → liste)
    sheets: Vec<String>,
    /// Gelen koleksiyon yanıtı hâlâ açık sayfaya mı ait
    coll_gen: u64,
    nav_skip: bool,
    /// Arama: en iyi sonuç, bölümler, yazarken gecikme
    top: Option<Top>,
    search_shelves: Vec<Section>,
    search_timer: Timer,
    /// Söz çevirileri: (şarkı, hedef dil) -> (kaynak dil, satırlar); None = alınamadı
    translations: HashMap<(String, i32), Option<(String, Vec<String>)>>,
    tr_pending: HashSet<(String, i32)>,
    /// Bekçi: yüklenen parça ne zaman yüklendi, ilerledi mi
    load_at: Option<std::time::Instant>,
    progressed: bool,
    watchdog: Timer,
    /// Atlama hedefi: mpv'den gelen eski konumlar kısa süre yok sayılır (görsel sıçrama olmasın)
    seek_guard: Option<(f32, std::time::Instant)>,
    /// Kaldığın yerden: yüklenecek parçanın başlangıç saniyesi
    pending_start: f64,
    /// Açılışta geri yüklenen oturum: oynat'a basınca bu konumdan başlar
    resume_pos: Option<f64>,
    last_session_save: std::time::Instant,
    /// Zevkler ekranı taslak durumu ve tür -> örnek şarkılar önbelleği
    taste_cache: HashMap<String, Vec<TrackInfo>>,
    taste_samples: Vec<TrackInfo>,
    taste_draft_genres: Vec<String>,
    taste_draft_blocked: Vec<String>,
    taste_draft_tracks: Vec<TrackInfo>,
    /// Ortam zemini sürekli zamanlayıcısı (33 ms, yalnız gerekince çalışır)
    amb_timer: Timer,
    amb_running: bool,
}

/// Arama "En iyi sonuç"u: çalınabilir şarkı ya da açılabilir kart.
#[derive(Clone)]
enum Top {
    Song(TrackInfo),
    Card(Card),
}

/// Ana sayfanın ham parçaları; ~/.cache/vynz/home.json içinde saklanır.
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
struct HomeData {
    personal: Vec<Section>,
    external: Vec<Section>,
    explore: Vec<Section>,
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
    static ART_ORDER: RefCell<VecDeque<String>> = RefCell::new(VecDeque::new());
    static BANNER_ORDER: RefCell<VecDeque<String>> = RefCell::new(VecDeque::new());
    /// Söz ekranı ve arka plan zemini: önceden bulanıklaştırılmış kapaklar.
    static BLURS: RefCell<HashMap<String, Image>> = RefCell::new(HashMap::new());
    /// Sıvı katman ve hale için radyal eriyen ikinci bulanık kapaklar.
    static BLUR2S: RefCell<HashMap<String, Image>> = RefCell::new(HashMap::new());
    /// Kapak anahtarı → ortam zemini renkleri (küçük; kapak temizliğinde silinir)
    static PALETTES: RefCell<HashMap<String, [Color; 3]>> = RefCell::new(HashMap::new());
    static BLUR_ORDER: RefCell<VecDeque<String>> = RefCell::new(VecDeque::new());
    static BLUR2_ORDER: RefCell<VecDeque<String>> = RefCell::new(VecDeque::new());
    /// Yalnız çalan şarkı için büyük kapaklar (kahraman, söz çekmecesi).
    static BIGS: RefCell<HashMap<String, Image>> = RefCell::new(HashMap::new());
    /// Listelere özel kapaklar (dosya adı -> görüntü, vurgu); liste sayısı kadar, ~400 KB/adet.
    static PL_COVERS: RefCell<HashMap<String, (Image, Color)>> = RefCell::new(HashMap::new());
    static BIG_ORDER: RefCell<VecDeque<String>> = RefCell::new(VecDeque::new());
}

const MAX_ARTS_RAM: usize = 96;
const MAX_BANNERS_RAM: usize = 1;
const MAX_BLURS_RAM: usize = 24;
const MAX_BIGS_RAM: usize = 12;

fn cache_art_lru(key: String, img: Image, color: Color) {
    let is_banner = key.starts_with("W|");
    let evicted = ARTS.with(|m| {
        let mut map = m.borrow_mut();
        let existed = map.insert(key.clone(), (img, color)).is_some();
        let order_cell = if is_banner { &BANNER_ORDER } else { &ART_ORDER };
        let cap = if is_banner { MAX_BANNERS_RAM } else { MAX_ARTS_RAM };
        order_cell.with(|o| {
            let mut q = o.borrow_mut();
            if existed {
                q.retain(|k| k != &key);
            }
            q.push_back(key);
            let mut out = Vec::new();
            while q.len() > cap {
                if let Some(old) = q.pop_front() {
                    map.remove(&old);
                    out.push(old);
                }
            }
            out
        })
    });
    if !evicted.is_empty() {
        with_state(|st| {
            for k in evicted {
                st.requested.remove(&k);
            }
        });
    }
}

fn cache_blur_lru(key: String, img: Image) {
    BLURS.with(|m| {
        let mut map = m.borrow_mut();
        let existed = map.insert(key.clone(), img).is_some();
        BLUR_ORDER.with(|o| {
            let mut q = o.borrow_mut();
            if existed {
                q.retain(|k| k != &key);
            }
            q.push_back(key);
            while q.len() > MAX_BLURS_RAM {
                if let Some(old) = q.pop_front() {
                    map.remove(&old);
                }
            }
        });
    });
}

fn cache_blur2_lru(key: String, img: Image) {
    BLUR2S.with(|m| {
        let mut map = m.borrow_mut();
        let existed = map.insert(key.clone(), img).is_some();
        BLUR2_ORDER.with(|o| {
            let mut q = o.borrow_mut();
            if existed {
                q.retain(|k| k != &key);
            }
            q.push_back(key);
            while q.len() > MAX_BLURS_RAM {
                if let Some(old) = q.pop_front() {
                    map.remove(&old);
                }
            }
        });
    });
}

fn cache_big_lru(key: String, img: Image) {
    BIGS.with(|m| {
        let mut map = m.borrow_mut();
        let existed = map.insert(key.clone(), img).is_some();
        BIG_ORDER.with(|o| {
            let mut q = o.borrow_mut();
            if existed {
                q.retain(|k| k != &key);
            }
            q.push_back(key);
            while q.len() > MAX_BIGS_RAM {
                if let Some(old) = q.pop_front() {
                    map.remove(&old);
                }
            }
        });
    });
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

/// MPRIS'te görünen ad.
fn media_title(t: &TrackInfo) -> String {
    if t.artist.is_empty() { t.title.clone() } else { format!("{} — {}", t.artist, t.title) }
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

fn card_kind(kind: &str) -> i32 {
    match kind {
        "playlist" => 0,
        "album" => 1,
        "artist" => 2,
        "mood" => 3,
        _ => 4,
    }
}

fn ui_card(c: &Card) -> CardUi {
    let key = c.art_key();
    let (art, has_art) = ARTS.with(|a| a.borrow().get(&key).map(|(i, _)| (i.clone(), true))).unwrap_or_default();
    CardUi { key: key.into(), kind: card_kind(&c.kind), title: c.title.as_str().into(), sub: c.sub.as_str().into(), art, has_art }
}

fn cards_of(cards: &[Card]) -> ModelRc<CardUi> {
    ModelRc::new(VecModel::from(cards.iter().map(ui_card).collect::<Vec<_>>()))
}

fn request_card_art(st: &mut State, c: &Card) {
    if c.thumb.is_empty() {
        return;
    }
    let key = c.art_key();
    if ARTS.with(|a| a.borrow().contains_key(&key)) || !st.requested.insert(key.clone()) {
        return;
    }
    let _ = st.art_tx.send((key, c.thumb.clone()));
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

/// Açık sayfadaki kapak: arka plan kuyruğunu beklemeden yüklensin.
fn request_art_hot(st: &mut State, key: String, url: &str) {
    if url.is_empty() || ARTS.with(|a| a.borrow().contains_key(&key)) {
        return;
    }
    st.requested.insert(key.clone());
    let _ = st.art_hot.send((key, url.to_string()));
}

fn toast(ui: &AppWindow, msg: &str) {
    show_toast(ui, msg, None);
}

/// Bildirim + "Geri al": yıkıcı işlemden sonra 8 sn boyunca geri alınabilir.
fn toast_undo(ui: &AppWindow, msg: &str, u: Undo) {
    show_toast(ui, msg, Some(u));
}

fn show_toast(ui: &AppWindow, msg: &str, u: Option<Undo>) {
    let has_undo = u.is_some();
    ui.set_toast(msg.into());
    ui.set_toast_undo(has_undo);
    let weak = ui.as_weak();
    with_state(|st| {
        st.undo = u; // yeni bildirim eskisinin geri alma hakkını kapatır
        let secs = if has_undo { 8 } else { 4 };
        st.toast_timer.start(TimerMode::SingleShot, Duration::from_secs(secs), move || {
            with_state(|st| st.undo = None);
            if let Some(ui) = weak.upgrade() {
                ui.set_toast("".into());
                ui.set_toast_undo(false);
            }
        })
    });
}

/// Geri alınabilir işlemler (tek seviye).
enum Undo {
    Playlist { idx: usize, pl: library::Playlist },
    PlaylistTrack { pl: usize, pos: usize, t: TrackInfo },
    QueueItem { pos: usize, t: TrackInfo },
    Queue { queue: Vec<TrackInfo>, index: Option<usize> },
    Import(import::Fetched),
    ImportRow { pos: usize, row: import::Row },
    Dislike { t: TrackInfo, liked: bool },
    DislikeArtist(String),
    Reset {
        boosts: Vec<TrackInfo>,
        disliked_tracks: Vec<TrackInfo>,
        disliked_artists: Vec<String>,
        listens: Option<Vec<history::Listen>>,
        taste_genres: Vec<String>,
        blocked_genres: Vec<String>,
        taste_tracks: Vec<TrackInfo>,
        tastes_done: bool,
    },
}

fn apply_undo(ui: &AppWindow) {
    let Some(u) = with_state(|st| st.undo.take()) else { return };
    with_state(|st| st.toast_timer.stop());
    ui.set_toast("".into());
    ui.set_toast_undo(false);
    match u {
        Undo::Playlist { idx, pl } => {
            with_state(|st| {
                let idx = idx.min(st.playlists.len());
                st.playlists.insert(idx, pl);
                let _ = library::save(&st.playlists);
            });
            refresh_playlists(ui);
        }
        Undo::PlaylistTrack { pl, pos, t } => {
            let ok = with_state(|st| {
                let p = st.playlists.get_mut(pl)?;
                let pos = pos.min(p.tracks.len());
                p.tracks.insert(pos, t);
                let _ = library::save(&st.playlists);
                Some(())
            });
            if ok.is_some() {
                if with_state(|st| st.current_pl) == Some(pl) {
                    sync_playlist_view(ui);
                }
                refresh_playlists_ui_only(ui);
            }
        }
        Undo::QueueItem { pos, t } => {
            with_state(|st| {
                let pos = pos.min(st.queue.len());
                st.queue.insert(pos, t);
                if let Some(cur) = st.index {
                    if pos <= cur {
                        st.index = Some(cur + 1);
                    }
                }
            });
            queue_changed(ui);
        }
        Undo::Queue { queue, index } => {
            with_state(|st| {
                // Çalan şarkı o arada değiştiyse eski kuyrukta onun yerini bul.
                let now = st.index.and_then(|i| st.queue.get(i)).map(TrackInfo::key);
                let at = match now {
                    Some(k) => queue.iter().position(|t| t.key() == k).or(index),
                    None => None,
                };
                st.queue = queue;
                st.index = at.filter(|i| *i < st.queue.len());
            });
            queue_changed(ui);
        }
        Undo::Import(f) => {
            with_state(|st| st.imported = Some(f));
            refresh_import(ui);
        }
        Undo::ImportRow { pos, row } => {
            with_state(|st| {
                if let Some(f) = st.imported.as_mut() {
                    let pos = pos.min(f.rows.len());
                    f.rows.insert(pos, row);
                }
            });
            refresh_import(ui);
        }
        Undo::Dislike { t, liked } => {
            with_state(|st| {
                st.history.undislike_track(&t.key());
                // İlgilenmiyorum beğeniyi de siliyordu: geri koy.
                if liked && !st.history.is_liked(&t.key()) {
                    st.history.toggle_like(&t);
                }
            });
            if ui.get_has_now() && ui.get_now().key == t.key().as_str() {
                ui.set_liked(liked);
            }
            refresh_hidden(ui);
            refresh_liked(ui);
        }
        Undo::DislikeArtist(a) => {
            with_state(|st| st.history.undislike_artist(&a));
            refresh_hidden(ui);
        }
        Undo::Reset { boosts, disliked_tracks, disliked_artists, listens, taste_genres, blocked_genres, taste_tracks, tastes_done } => {
            with_state(|st| {
                st.history.restore_profile(boosts, disliked_tracks, disliked_artists);
                if let Some(l) = listens {
                    st.history.restore_listens(l);
                }
                st.settings.taste_genres = taste_genres;
                st.settings.blocked_genres = blocked_genres;
                st.settings.taste_tracks = taste_tracks;
                st.settings.tastes_done = tastes_done;
                let _ = st.settings.save();
            });
            ui.set_tastes_open(false);
            ui.set_tastes_step(0);
            refresh_hidden(ui);
            refresh_taste_summary(ui);
            refresh_profile(ui);
            load_home(ui);
        }
    }
    toast(ui, "Geri alındı");
}

// ---------- Kapaklar ----------

type ArtTx = mpsc::Sender<(String, String)>;

/// Kapak işçileri: önce "sıcak" kuyruk (açık sayfa), sonra arka plan kuyruğu.
fn start_art_pool(weak: Weak<AppWindow>) -> (ArtTx, ArtTx) {
    let (tx, rx) = mpsc::channel::<(String, String)>();
    let (hot_tx, hot_rx) = mpsc::channel::<(String, String)>();
    let rx = Arc::new(Mutex::new(rx));
    let hot_rx = Arc::new(Mutex::new(hot_rx));
    for _ in 0..6 {
        let rx = rx.clone();
        let hot_rx = hot_rx.clone();
        let weak = weak.clone();
        thread::spawn(move || loop {
            let hot = hot_rx.lock().unwrap().try_recv();
            let job = match hot {
                Ok(j) => Ok(j),
                Err(mpsc::TryRecvError::Disconnected) => break,
                Err(mpsc::TryRecvError::Empty) => match rx.lock().unwrap().recv_timeout(Duration::from_millis(60)) {
                    Ok(j) => Ok(j),
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(mpsc::RecvTimeoutError::Disconnected) => Err(()),
                },
            };
            let Ok((key, url)) = job else { break };
            // "B|" önekli istek: çalan şarkının büyük kapağı (aynı önbellek dosyası).
            let res = if let Some(k) = key.strip_prefix("W|") {
                art::load_wide(k, &url, 1440, 600)
            } else {
                let (file_key, size) = match key.strip_prefix("B|") {
                    Some(k) => (k.to_string(), art::BIG),
                    None => (key.clone(), art::SMALL),
                };
                art::load(&file_key, &url, size)
            };
            match res {
                Ok(a) => {
                    let _ = weak.upgrade_in_event_loop(move |ui| apply_art(&ui, key, a));
                }
                Err(e) => eprintln!("kapak {key}: {e}"),
            }
        });
    }
    (tx, hot_tx)
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

fn update_cards(model: &ModelRc<CardUi>, key: &str, img: &Image) {
    for i in 0..model.row_count() {
        if let Some(mut c) = model.row_data(i) {
            if c.key == key {
                c.art = img.clone();
                c.has_art = true;
                model.set_row_data(i, c);
            }
        }
    }
}

fn apply_art(ui: &AppWindow, key: String, a: art::Art) {
    if let Some(k) = key.strip_prefix("W|") {
        if with_state(|st| st.banner_key == k) {
            let img = Image::from_rgba8(SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&a.rgba, a.width, a.height));
            ui.set_artist_banner(img);
            ui.set_has_banner(true);
        }
        return;
    }
    if let Some(k) = key.strip_prefix("B|") {
        let img = Image::from_rgba8(SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&a.rgba, a.width, a.height));
        cache_big_lru(k.to_string(), img.clone());
        let mut now = ui.get_now();
        if ui.get_has_now() && now.key == k {
            now.art = img;
            now.has_art = true;
            ui.set_now(now);
        }
        return;
    }
    let buf = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&a.rgba, a.width, a.height);
    let img = Image::from_rgba8(buf);
    let color = Color::from_rgb_u8(a.accent.0, a.accent.1, a.accent.2);
    cache_art_lru(key.clone(), img.clone(), color);

    let shelves = ui.get_shelves();
    for i in 0..shelves.row_count() {
        if let Some(s) = shelves.row_data(i) {
            update_rows(&s.items, &key, &img);
            update_cards(&s.cards, &key, &img);
        }
    }
    update_cards(&ui.get_browse_cards(), &key, &img);
    let ss = ui.get_search_shelves();
    for i in 0..ss.row_count() {
        if let Some(s) = ss.row_data(i) {
            update_cards(&s.cards, &key, &img);
        }
    }
    if ui.get_has_top() && with_state(|st| st.top.as_ref().map(top_key)) == Some(key.clone()) {
        let mut t = ui.get_search_top();
        t.art = img.clone();
        t.has_art = true;
        ui.set_search_top(t);
    }
    update_rows(&ui.get_coll_tracks(), &key, &img);
    update_rows(&ui.get_artist_tracks(), &key, &img);
    let ash = ui.get_artist_shelves();
    for i in 0..ash.row_count() {
        if let Some(s) = ash.row_data(i) {
            update_cards(&s.cards, &key, &img);
        }
    }
    update_rows(&ui.get_results(), &key, &img);
    update_rows(&ui.get_queue(), &key, &img);
    update_rows(&ui.get_playlist_tracks(), &key, &img);
    if with_state(|st| st.playlists.iter().any(|p| p.tracks.iter().take(12).any(|t| t.key() == key))) {
        refresh_playlists_ui_only(ui);
    }
    update_rows(&ui.get_liked_tracks(), &key, &img);
    update_rows(&ui.get_taste_samples(), &key, &img);
    let rows = ui.get_import_rows();
    for i in 0..rows.row_count() {
        if let Some(mut r) = rows.row_data(i) {
            if r.key == key {
                r.art = img.clone();
                r.has_art = true;
                rows.set_row_data(i, r);
            }
        }
    }

    let blur = Image::from_rgba8(SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&a.blur, art::BLUR, art::BLUR));
    cache_blur_lru(key.clone(), blur.clone());
    let blur2 = Image::from_rgba8(SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&a.blur2, art::BLUR, art::BLUR));
    cache_blur2_lru(key.clone(), blur2.clone());
    let pal = a.palette.map(|(r, g, b)| Color::from_rgb_u8(r, g, b));
    PALETTES.with(|m| m.borrow_mut().insert(key.clone(), pal));

    let mut now = ui.get_now();
    if ui.get_has_now() && now.key == key {
        // Büyük kapak hazırsa onu koru.
        if !BIGS.with(|b| b.borrow().contains_key(&key)) {
            now.art = img;
            now.has_art = true;
            ui.set_now(now);
        }
        set_cover_accent(&ui, color);
        ui.set_now_blur(blur);
        ui.set_has_blur(true);
        ui.set_now_blur2(blur2);
        ui.set_has_blur2(true);
        set_ambience(&ui, Some(pal));
    }
}

/// Ortam zemini renkleri: kapak paleti; yoksa vurgu renginden türetilir (Slint tarafı).
fn set_ambience(ui: &AppWindow, pal: Option<[Color; 3]>) {
    match pal {
        Some(p) => {
            ui.set_amb_c1(p[0]);
            ui.set_amb_c2(p[1]);
            ui.set_amb_c3(p[2]);
            ui.set_has_amb(true);
        }
        None => ui.set_has_amb(false),
    }
    refresh_amb_discs(ui);
}

thread_local! {
    static AMB_DISC: Vec<u8> = ambience::disc_rgba();
    static AMB_LAST: std::cell::Cell<[u32; 3]> = const { std::cell::Cell::new([u32::MAX; 3]) };
}

/// Zemin renkleri (amb1..3, palet ya da vurgudan) değişince boyalı diskleri yeniden üret.
fn refresh_amb_discs(ui: &AppWindow) {
    let cols = [ui.get_amb1(), ui.get_amb2(), ui.get_amb3()];
    let key = cols.map(|c| c.as_argb_encoded());
    if AMB_LAST.with(|l| l.get()) == key {
        return;
    }
    AMB_LAST.with(|l| l.set(key));
    let imgs: Vec<Image> = AMB_DISC.with(|base| {
        cols.iter()
            .map(|c| {
                let px = ambience::tinted_disc(base, (c.red(), c.green(), c.blue()));
                Image::from_rgba8(SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&px, ambience::DISC, ambience::DISC))
            })
            .collect()
    });
    ui.set_amb_d1(imgs[0].clone());
    ui.set_amb_d2(imgs[1].clone());
    ui.set_amb_d3(imgs[2].clone());
}

// ---------- Oynatma ----------

/// MPRIS'e ileri/geri durumunu bildir.
fn mpris_caps() {
    with_state(|st| {
        let next = st.index.is_some_and(|i| i + 1 < st.queue.len()) || (st.index.is_some() && st.settings.autoradio);
        st.mpris.send(mpris::Msg::Caps { next, prev: st.index.is_some() });
    });
}

/// Önceki parçanın dinlemesini geçmişe yaz, yenisini başlat.
fn start_listen(t: Option<&TrackInfo>) {
    with_state(|st| {
        if let Some((prev, played, _)) = st.listen.take() {
            dbg(&format!("dinleme: {} {:.0} sn", prev.key(), played));
            st.history.record(prev, played);
        }
        st.listen = t.map(|t| (t.clone(), 0.0, 0.0));
    });
}

/// Çalan şarkının büyük kapağını iste.
fn request_big(st: &mut State, t: &TrackInfo) {
    let key = format!("B|{}", t.key());
    if t.thumb.is_empty() || BIGS.with(|b| b.borrow().contains_key(&t.key())) {
        return;
    }
    let _ = st.art_tx.send((key, t.thumb.clone()));
}

fn set_now_ui(ui: &AppWindow, t: &TrackInfo, i: usize) {
    start_listen(Some(t));
    with_state(|st| {
        st.load_at = Some(std::time::Instant::now());
        st.progressed = false;
    });
    with_state(|st| {
        st.mpris.send(mpris::Msg::Track {
            key: t.key(),
            title: t.title.clone(),
            artist: t.artist.clone(),
            album: t.album.clone(),
            art: t.thumb.clone(),
            length: t.duration as f64,
        });
        st.mpris.send(mpris::Msg::Playing(true));
    });
    mpris_caps();
    with_state(|st| request_big(st, t));
    ui.set_liked(with_state(|st| st.history.is_liked(&t.key())));
    ui.set_has_now(true);
    let mut nt = ui_track(t);
    if let Some(big) = BIGS.with(|b| b.borrow().get(&t.key()).cloned()) {
        nt.art = big;
        nt.has_art = true;
    }
    ui.set_now(nt);
    ui.set_queue_index(i as i32);
    ui.set_position(with_state(|st| st.pending_start) as f32);
    ui.set_duration(t.duration);
    ui.set_playing(true);
    let accent = ARTS.with(|a| a.borrow().get(&t.key()).map(|(_, c)| *c)).unwrap_or(NEUTRAL);
    set_cover_accent(ui, accent);
    match BLURS.with(|b| b.borrow().get(&t.key()).cloned()) {
        Some(b) => {
            ui.set_now_blur(b);
            ui.set_has_blur(true);
        }
        None => ui.set_has_blur(false),
    }
    match BLUR2S.with(|b| b.borrow().get(&t.key()).cloned()) {
        Some(b) => {
            ui.set_now_blur2(b);
            ui.set_has_blur2(true);
        }
        None => ui.set_has_blur2(false),
    }
    set_ambience(ui, PALETTES.with(|m| m.borrow().get(&t.key()).copied()));
    request_lyrics(ui, t);
    render_lyrics(ui);
    save_session(ui);
}

// ---------- Sözler ----------

fn request_lyrics(ui: &AppWindow, t: &TrackInfo) {
    let key = t.key();
    let resolver = with_state(|st| {
        if !st.lyrics.contains_key(&key) && st.lyr_pending.insert(key.clone()) {
            Some(st.resolver.clone())
        } else {
            None
        }
    });
    let Some(resolver) = resolver else {
        return;
    };
    let t = t.clone();
    let weak = ui.as_weak();
    thread::spawn(move || {
        let res = lyrics::fetch(&t, &resolver).ok();
        let _ = weak.upgrade_in_event_loop(move |ui| {
            dbg(&format!("sözler {key}: {}", if res.is_some() { "bulundu" } else { "yok" }));
            with_state(|st| {
                st.lyr_pending.remove(&key);
                st.lyrics.insert(key.clone(), res);
            });
            if ui.get_now().key == key {
                render_lyrics(&ui);
            }
        });
    });
}

fn offsets_path() -> std::path::PathBuf {
    paths::data_dir().join("lyric-offsets.json")
}

fn load_offsets() -> HashMap<String, f32> {
    std::fs::read(offsets_path()).ok().and_then(|r| serde_json::from_slice(&r).ok()).unwrap_or_default()
}

/// Sözler ekranı açıkken saniyede ~20 kez: son konumdan geçen süreyi ekleyerek akıcı dolum.
fn lyric_tick(ui: &AppWindow) {
    if !ui.get_playing() || ui.get_buffering() {
        return;
    }
    let (p, at) = with_state(|st| st.last_pos);
    let pos = p + at.elapsed().as_secs_f32().min(0.6);
    update_lyrics(ui, pos);
}

const TR_TARGET: [&str; 3] = ["", "tr", "en"];

/// Çeviri açıksa ve henüz yoksa arka planda iste.
fn request_translation(ui: &AppWindow) {
    let mode = ui.get_lyrics_translate();
    if mode <= 0 {
        return;
    }
    let key = ui.get_now().key.to_string();
    let lines = with_state(|st| {
        let k = (key.clone(), mode);
        if st.translations.contains_key(&k) || st.tr_pending.contains(&k) {
            return None;
        }
        let l = st.lyrics.get(&key)?.as_ref()?;
        st.tr_pending.insert(k);
        Some(l.lines.iter().map(|x| x.text.clone()).collect::<Vec<_>>())
    });
    let Some(lines) = lines else { return };
    ui.set_lyric_tr_note("Çevriliyor…".into());
    let weak = ui.as_weak();
    thread::spawn(move || {
        let res = lyrics::translate(&lines, TR_TARGET[mode as usize]).ok();
        let _ = weak.upgrade_in_event_loop(move |ui| {
            with_state(|st| {
                st.tr_pending.remove(&(key.clone(), mode));
                st.translations.insert((key.clone(), mode), res);
            });
            if ui.get_now().key == key.as_str() {
                render_lyrics(&ui);
            }
        });
    });
}

/// Söz kelimeleri arasındaki boşluk. Modern stilde (Jakarta, eksi harf aralığı) düz boşluk
/// ~0.14em kalıyor ve kelimeler birbirine yapışık görünüyor; ince boşluk (U+2009) eklenince ~0.3em.
/// Pasif satır metni ve etkin satırın kelime aralığı aynı dizgeden hesaplanır (satır zıplamasın).
fn lyric_joiner(style: i32) -> &'static str {
    if style == 0 { "\u{2009} " } else { " " }
}

/// Çalan şarkının söz durumunu baştan kur.
fn render_lyrics(ui: &AppWindow) {
    let key = ui.get_now().key.to_string();
    let style = ui.get_lyrics_style();
    let mode = ui.get_lyrics_translate();
    let credit = with_state(|st| st.lyrics.get(&key).and_then(|l| l.as_ref()).and_then(|l| l.credit.clone()));
    ui.set_lyric_credit_user(credit.as_ref().map(|c| c.user.as_str()).unwrap_or("").into());
    ui.set_has_lyric_credit(credit.is_some());
    let (state, source, lines, offset, note) = with_state(|st| {
        st.lyr_line = None;
        st.lyr_spoken = usize::MAX;
        st.lyr_seg = None;
        st.lyr_hi = 0.0;
        st.lyr_force = true;
        let offset = st.lyr_offsets.get(&key).copied().unwrap_or(0.0);
        // Çeviri satırları ve not
        let (trs, note): (Vec<String>, String) = if mode <= 0 {
            (Vec::new(), String::new())
        } else {
            match st.translations.get(&(key.clone(), mode)) {
                Some(Some((src, v))) if v.is_empty() => (Vec::new(), if src == "tr" { "Şarkı zaten Türkçe".into() } else { "Şarkı zaten bu dilde".into() }),
                Some(Some((_, v))) => (v.clone(), String::new()),
                Some(None) => (Vec::new(), "Çeviri alınamadı".into()),
                None => (Vec::new(), "Çevriliyor…".into()),
            }
        };
        let join = lyric_joiner(style);
        let mk = |l: &lyrics::Lyrics| -> Vec<LyricLine> {
            l.lines.iter().enumerate().map(|(i, x)| LyricLine {
                text: x.text.split_whitespace().collect::<Vec<_>>().join(join).into(),
                tr: trs.get(i).filter(|t| !t.is_empty() && **t != x.text).map(|t| t.as_str().into()).unwrap_or_default(),
                t: x.t,
                state: 1,
                dist: 9,
            }).collect()
        };
        match st.lyrics.get(&key) {
            None => (if st.lyr_pending.contains(&key) { 1 } else { 0 }, String::new(), Vec::new(), offset, String::new()),
            Some(None) => (4, String::new(), Vec::new(), offset, String::new()),
            Some(Some(l)) => (if l.synced { 2 } else { 3 }, l.source.clone(), mk(l), offset, note),
        }
    });
    ui.set_lyric_state(state);
    ui.set_lyric_source(source.into());
    ui.set_lyric_offset(offset);
    ui.set_lyric_tr_note(note.into());
    if state == 3 {
        ui.set_lyric_plain(ModelRc::new(VecModel::from(lines.clone())));
        ui.set_lyric_lines(ModelRc::new(VecModel::from(Vec::<LyricLine>::new())));
    } else {
        ui.set_lyric_plain(ModelRc::new(VecModel::from(Vec::<LyricLine>::new())));
        ui.set_lyric_lines(ModelRc::new(VecModel::from(lines)));
    }
    ui.set_lyric_active(-1);
    ui.set_lyric_gap(-1.0);
    ui.set_lyric_rows(ModelRc::new(VecModel::from(Vec::<LyricRow>::new())));
    update_lyrics(ui, ui.get_position());
    if state >= 2 && state <= 3 {
        request_translation(ui);
    }
}

/// Kelime dolumu ve satır geçişi biraz önden gelsin (Better Lyrics ile aynı: 150 / 115 ms).
const WORD_LEAD: f32 = 0.15;
const LINE_LEAD: f32 = 0.115;

/// Konuma göre etkin satırı, kelime dolumunu ve müzik arasını güncelle (yalnız değişenler).
/// Dolum Slint'te akar: burada yalnız hece/kelime sınırlarında "şu dolum noktasına şu sürede var" yazılır.
/// Duraklatılmışken (`hold`) dolum bulunduğu yerde durur.
fn update_lyrics(ui: &AppWindow, pos: f32) {
    if ui.get_lyric_state() != 2 {
        return;
    }
    let hold = !ui.get_playing() || ui.get_buffering();
    let key = ui.get_now().key.to_string();
    let style = ui.get_lyrics_style();
    let px = measure::font_size(style, ui.get_lyrics_size());
    // Satır genişliği: Slint'in pasif satırları kaydırdığı genişlikle aynı (1 px pay);
    // kelime arası yazı tipinin gerçek boşluk genişliği, böylece satır etkin olunca zıplamaz.
    let max = (ui.get_lyric_area() - 1.0).max(160.0);
    let word_gap = measure::width(style, lyric_joiner(style), px);
    ui.set_lyric_word_gap(word_gap);
    enum Change {
        None,
        Line { idx: Option<usize>, rows: Vec<Vec<LyricWord>> },
        Words(lyrics::Progress),
    }
    let (change, gap) = with_state(|st| {
        let offset = st.lyr_offsets.get(&key).copied().unwrap_or(0.0);
        let Some(Some(l)) = st.lyrics.get(&key) else { return (Change::None, None) };
        // Ses saati (mpv time-pos) ve olay sırası yüzünden konum birkaç on ms geri gelebilir;
        // bu, etkin kelimeyi "sırada"ya düşürüp dolumu baştan başlatıyordu. Küçük geri
        // sıçramalar yok sayılır, gerçek geri sarma (≥ 0.4 sn) kabul edilir.
        let pos = if pos < st.lyr_hi && st.lyr_hi - pos < 0.4 { st.lyr_hi } else { pos };
        st.lyr_hi = pos;
        let p = pos + offset;
        let gap = lyrics::gap(l, p);
        let pr = lyrics::progress(l, p + LINE_LEAD, p + WORD_LEAD);
        let idx = pr.map(|x| x.idx);
        let seg = pr.map(|x| (x.spoken, (x.target * 1000.0) as i32, hold));
        if idx != st.lyr_line || st.lyr_force {
            st.lyr_force = false;
            st.lyr_line = idx;
            // Yeni satırın kelimeleri yeni öğe olarak doğar; Slint ilk değeri canlandırmaz.
            // Etkin kelime hedef dolumla doğarsa bir anda dolu görünür (ilk kelime hatası).
            // Bu yüzden şimdiki dolumla, süresiz kur; hedefi bir sonraki tıkta (50 ms) yaz.
            st.lyr_seg = None;
            let _ = seg;
            let rows = match (idx, pr) {
                (Some(i), Some(pr)) => {
                    let line = &l.lines[i];
                    let ws: Vec<&str> = line.text.split_whitespace().collect();
                    let spans = lyrics::word_spans(line);
                    let row_of = measure::wrap(style, &ws, px, word_gap, max);
                    let mut rows: Vec<Vec<LyricWord>> = vec![Vec::new(); row_of.last().map(|r| r + 1).unwrap_or(0)];
                    for (k, w) in ws.iter().enumerate() {
                        let long = spans.get(k).is_some_and(|(a, b)| b - a >= 1.0);
                        rows[row_of[k]].push(word_at(k, w, long, &pr, true));
                    }
                    rows
                }
                _ => Vec::new(),
            };
            if std::env::var_os("VYNZ_LYRDBG").is_some() {
                eprintln!("LYR LINE pos={pos:.3} idx={idx:?}");
            }
            return (Change::Line { idx, rows }, gap);
        }
        if seg != st.lyr_seg {
            st.lyr_seg = seg;
            if let Some(pr) = pr {
                return (Change::Words(pr), gap);
            }
        }
        (Change::None, gap)
    });
    ui.set_lyric_gap(gap.unwrap_or(-1.0));
    match change {
        Change::None => {}
        Change::Line { idx, rows } => {
            let rows: Vec<LyricRow> = rows.into_iter().map(|r| LyricRow { words: ModelRc::new(VecModel::from(r)) }).collect();
            ui.set_lyric_rows(ModelRc::new(VecModel::from(rows)));
            // Satır durumları: geçmiş -1, etkin 0, gelecek 1; uzaklık solma için
            let lines = ui.get_lyric_lines();
            let a = idx.map(|i| i as i64).unwrap_or(-1);
            for j in 0..lines.row_count() {
                if let Some(mut ln) = lines.row_data(j) {
                    let jj = j as i64;
                    let state = if jj < a { -1 } else if jj == a { 0 } else { 1 };
                    let dist = (jj - a).unsigned_abs().min(9) as i32;
                    if ln.state != state || ln.dist != dist {
                        ln.state = state;
                        ln.dist = dist;
                        lines.set_row_data(j, ln);
                    }
                }
            }
            ui.set_lyric_active(a as i32);
        }
        Change::Words(pr) => {
            // Kelime sırası satırlar boyunca sürer; yalnız değişen kelimeler yazılır.
            let mut k = 0;
            let rows = ui.get_lyric_rows();
            for r in 0..rows.row_count() {
                let Some(row) = rows.row_data(r) else { continue };
                let m = row.words;
                for j in 0..m.row_count() {
                    if let Some(w) = m.row_data(j) {
                        let nw = word_at(k, &w.text, w.long, &pr, hold);
                        if w.state != nw.state || (w.fill - nw.fill).abs() > 0.001 || w.dur != nw.dur {
                            if std::env::var_os("VYNZ_LYRDBG").is_some() {
                                eprintln!("LYR {:.3} pos={pos:.3} k={k} '{}' st={} fill={:.3} dur={}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64() % 1000.0, w.text, nw.state, nw.fill, nw.dur);
                            }
                            m.set_row_data(j, nw);
                        }
                    }
                    k += 1;
                }
            }
        }
    }
}

/// k. kelimenin karaoke durumu: söylendi / şu an (hedef dolum + süre) / sırada.
fn word_at(k: usize, text: &str, long: bool, pr: &lyrics::Progress, hold: bool) -> LyricWord {
    let (state, fill, dur) = if k < pr.spoken {
        (0, 1.0, 0)
    } else if k == pr.spoken {
        if hold || pr.remain <= 0.0 { (1, pr.fill, 0) } else { (1, pr.target, (pr.remain * 1000.0) as i32) }
    } else {
        (2, 0.0, 0)
    };
    LyricWord { text: text.into(), state, fill, dur, long }
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
        st.pending_start = st.resume_pos.take().unwrap_or(0.0);
        request_art(st, &t);
        let key = t.key();
        if let Some(path) = cache::lookup(&key) {
            dbg(&format!("çal {key}: disk önbelleği"));
            st.player.send(Cmd::Load { url: path.to_string_lossy().into_owned(), title: media_title(&t), start: st.pending_start });
            st.loaded = true;
        } else if let Some(r) = st.resolver.cached(&key) {
            dbg(&format!("çal {key}: önceden çözülmüş"));
            st.player.send(Cmd::Load { url: r.stream, title: media_title(&t), start: st.pending_start });
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
    with_state(|st| {
        let Some(t) = st.index.and_then(|i| st.queue.get(i).cloned()) else { return };
        match res {
            Ok(r) => {
                st.player.send(Cmd::Load { url: r.stream, title: media_title(&t), start: st.pending_start });
                st.loaded = true;
            }
            Err(e) => {
                eprintln!("çözülemedi ({key}): {e}; yedek yol");
                st.fallback_tried = true;
                if let Some(page) = page_url(st, &t) {
                    st.player.send(Cmd::Load { url: page, title: media_title(&t), start: st.pending_start });
                    st.loaded = true;
                }
            }
        }
    });
    maybe_append_next(ui);
}

/// Kuyruktaki sonraki parçaları önceden çöz; ilki hazır olunca mpv'ye sıradaki olarak ekle.
fn schedule_prefetch(ui: &AppWindow) {
    // Liste bitmek üzereyse radyoyla önceden uzat (geçiş yine anında olsun).
    let near_end = with_state(|st| {
        st.settings.autoradio && st.repeat == 0 && !st.radio_busy && st.index.is_some_and(|i| i + 2 >= st.queue.len())
    });
    if near_end {
        extend_with_radio(ui, false);
    }
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
            if cache::exists(&key).is_some() || st.resolver.cached(&key).is_some() {
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
        let url = if let Some(p) = cache::lookup(&key) {
            p.to_string_lossy().into_owned()
        } else if let Some(r) = st.resolver.cached(&key) {
            r.stream
        } else {
            return; // çözülünce tekrar çağrılır
        };
        dbg(&format!("sıradaki eklendi: {key}"));
        let title = media_title(&st.queue[next]);
        st.player.send(Cmd::Append { url, title });
        st.appended = Some(next);
    });
}

fn step(ui: &AppWindow, delta: i64) {
    let (idx, len, repeat, autoradio) = with_state(|st| (st.index, st.queue.len(), st.repeat, st.settings.autoradio));
    let Some(idx) = idx else { return };
    let next = idx as i64 + delta;
    if next >= 0 && (next as usize) < len {
        play_index(ui, next as usize);
    } else if delta > 0 && repeat == 1 && len > 0 {
        play_index(ui, 0);
    } else if delta > 0 && autoradio {
        // Liste bitti: çalan şarkının benzerleriyle devam et.
        extend_with_radio(ui, true);
    } else if delta > 0 {
        ui.set_playing(false);
    }
}

/// mpv eklediğimiz sıradakine geçti (doğal bitiş ya da MPRIS "sonraki"): durumu güncelle.
fn on_started(ui: &AppWindow, pos: i64) {
    if pos < 1 {
        return; // kendi yüklediğimiz parça
    }
    let advanced = with_state(|st| {
        let i = st.index?;
        if st.appended != Some(i + 1) {
            return None;
        }
        dbg("ön-tampondan sıradakine geçildi");
        st.pending_start = 0.0;
        st.index = Some(i + 1);
        st.appended = None;
        st.play_seq += 1;
        st.loaded = true; // mpv zaten yükledi
        st.fallback_tried = false;
        Some((st.queue[i + 1].clone(), i + 1))
    });
    if let Some((t, i)) = advanced {
        set_now_ui(ui, &t, i);
        schedule_prefetch(ui);
        // Uyku: "şarkı bitince" — boşluksuz geçişte yeni parça başını bekletir
        if ui.get_sleep_eot() {
            ui.set_sleep_eot(false);
            with_state(|st| {
                st.player.send(Cmd::SetPause(true));
                st.player.send(Cmd::Seek(0.0));
            });
        }
    }
}

/// Parça bitti. Sıradaki eklenmişse mpv ona kendisi geçer (on_started); değilse biz geçeriz.
fn on_ended(ui: &AppWindow) {
    let has_next = with_state(|st| st.index.is_some_and(|i| st.appended == Some(i + 1)));
    if !has_next {
        step(ui, 1);
        if ui.get_sleep_eot() {
            ui.set_sleep_eot(false);
            with_state(|st| st.player.send(Cmd::SetPause(true)));
        }
    }
}

thread_local! {
    /// Mini oynatıcı penceresi, durum zamanlayıcısı ve ana pencere bağlantısı
    static MINI: RefCell<Option<(MiniWindow, Timer, Weak<AppWindow>)>> = const { RefCell::new(None) };
}

/// Mini oynatıcıyı aç (zaten açıksa öne getir). Durum ana pencereden 250 ms'de bir kopyalanır;
/// düğmeler ana pencerenin geri çağrılarını kullanır (tek doğruluk kaynağı ana pencere).
fn mini_open(ui: &AppWindow) {
    let existing = MINI.with(|m| m.borrow().as_ref().map(|(w, _, _)| w.clone_strong()));
    if let Some(w) = existing {
        let _ = w.show();
        return;
    }
    let Ok(mini) = MiniWindow::new() else { return };
    let main = ui.as_weak();
    macro_rules! on_main {
        ($setter:ident, |$u:ident| $body:expr) => {{
            let main = main.clone();
            mini.$setter(move || {
                if let Some($u) = main.upgrade() {
                    $body;
                }
            });
        }};
    }
    on_main!(on_toggle, |u| u.invoke_toggle_play());
    on_main!(on_next, |u| u.invoke_next());
    on_main!(on_prev, |u| u.invoke_prev());
    on_main!(on_like, |u| u.invoke_toggle_like());
    on_main!(on_expand, |u| {
        set_game_mode(&u, false);
        show_from_tray(&u);
        mini_close();
    });
    mini.on_dismiss(|| mini_close());
    mini.window().on_close_requested(|| {
        mini_close();
        slint::CloseRequestResponse::HideWindow
    });
    {
        let main = main.clone();
        mini.on_seek(move |t| {
            if let Some(u) = main.upgrade() {
                u.invoke_seek(t);
            }
        });
    }
    {
        let w = mini.as_weak();
        mini.on_drag(move || {
            if let Some(m) = w.upgrade() {
                use slint::winit_030::WinitWindowAccessor;
                m.window().with_winit_window(|w| {
                    let _ = w.drag_window();
                });
            }
        });
    }
    let sync = {
        let main = main.clone();
        let mw = mini.as_weak();
        move || {
            let (Some(u), Some(m)) = (main.upgrade(), mw.upgrade()) else { return };
            let now = u.get_now();
            m.set_has_now(u.get_has_now());
            m.set_track_title(now.title.clone());
            m.set_track_artist(now.artist.clone());
            m.set_art(now.art.clone());
            m.set_has_art(now.has_art);
            m.set_playing(u.get_playing());
            m.set_liked(u.get_liked());
            m.set_game_mode(u.get_game_mode());
            m.set_position(u.get_position());
            m.set_duration(u.get_duration());
            let (ut, mt) = (u.global::<Theme>(), m.global::<Theme>());
            mt.set_accent(ut.get_accent());
            mt.set_canvas(ut.get_canvas());
            mt.set_floating(ut.get_floating());
        }
    };
    sync();
    let t = Timer::default();
    t.start(TimerMode::Repeated, Duration::from_millis(250), sync);
    let _ = mini.show();
    #[cfg(windows)]
    {
        // Windows 11'de köşe yuvarlaması DWM'e bırakılır
        let mw = mini.as_weak();
        Timer::single_shot(Duration::from_millis(60), move || {
            if let Some(m) = mw.upgrade() {
                use slint::winit_030::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
                use slint::winit_030::WinitWindowAccessor;
                use windows::Win32::Foundation::HWND;
                use windows::Win32::Graphics::Dwm::{
                    DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
                };
                m.window().with_winit_window(|w| {
                    if let Ok(handle) = w.window_handle() {
                        if let RawWindowHandle::Win32(h) = handle.as_raw() {
                            let hwnd = HWND(h.hwnd.get() as *mut std::ffi::c_void);
                            let dark: u32 = 1;
                            let round: i32 = DWMWCP_ROUND.0;
                            unsafe {
                                let _ = DwmSetWindowAttribute(
                                    hwnd,
                                    DWMWA_USE_IMMERSIVE_DARK_MODE,
                                    (&dark as *const u32).cast(),
                                    std::mem::size_of::<u32>() as u32,
                                );
                                let _ = DwmSetWindowAttribute(
                                    hwnd,
                                    DWMWA_WINDOW_CORNER_PREFERENCE,
                                    (&round as *const i32).cast(),
                                    std::mem::size_of::<i32>() as u32,
                                );
                            }
                        }
                    }
                });
            }
        });
    }
    MINI.with(|m| *m.borrow_mut() = Some((mini, t, main)));
}

fn mini_close() {
    // Bırakmayı olay döngüsüne ertele: kapatma düğmesinin kendi geri çağrısı içindeyiz
    let _ = slint::invoke_from_event_loop(|| {
        if let Some((w, _t, main)) = MINI.with(|m| m.borrow_mut().take()) {
            let _ = w.hide();
            // Oyun modundayken mini kapatılırsa ana pencere görünmez kalmasın
            if let Some(u) = main.upgrade() {
                if u.get_game_mode() {
                    set_game_mode(&u, false);
                    show_from_tray(&u);
                }
            }
        }
    });
}

thread_local! {
    /// Oyun modunda bellek göstergesini tazeleyen zamanlayıcı
    static GAME_TICK: RefCell<Option<Timer>> = const { RefCell::new(None) };
}

/// Oyun modu: mini oynatıcıyı aç, belleği boşalt ve ana pencereyi gizle. Çıkınca kapaklar geri gelir.
fn set_game_mode(ui: &AppWindow, on: bool) {
    if on == ui.get_game_mode() {
        return;
    }
    ui.set_game_mode(on);
    if on {
        ui.set_eq_pop(false);
        ui.set_sleep_pop(false);
        ui.set_menu_open(false);
        ui.set_drawer_open(false);
        mini_open(ui);
        release_ram(ui);
        // winit set_visible Wayland'de etkisiz; Slint hide() yüzeyi gerçekten kaldırır.
        // Mini açık olduğu için olay döngüsü sürer; geri dönüş show_from_tray (ui.show()).
        let _ = ui.hide();
        #[cfg(all(target_os = "linux", target_env = "gnu"))]
        {
            // Ayırıcıdaki boş sayfaları işletim sistemine geri ver (Slint dokuları geç bırakabilir)
            unsafe {
                libc::malloc_trim(0);
            }
            Timer::single_shot(Duration::from_millis(500), || {
                unsafe {
                    libc::malloc_trim(0);
                }
            });
        }
        let weak = ui.as_weak();
        let t = Timer::default();
        let tick = move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_game_mem(own_memory_text().into());
            }
        };
        tick();
        t.start(TimerMode::Repeated, Duration::from_secs(5), tick);
        GAME_TICK.with(|g| *g.borrow_mut() = Some(t));
    } else {
        GAME_TICK.with(|g| g.borrow_mut().take());
        ui.set_game_mem("".into());
        ui.set_window_hidden(false);
        ui.global::<Motion>().set_paused(false);
        reload_art(ui);
    }
    update_viz(ui);
}

/// "Vynz şu an ~N MB kullanıyor" (Linux: /proc; diğerlerinde boş)
fn own_memory_text() -> String {
    #[cfg(target_os = "linux")]
    {
        let Ok(s) = std::fs::read_to_string("/proc/self/status") else { return String::new() };
        // RssAnon: uygulamanın kendi belleği (paylaşılan kütüphaneler hariç)
        let kb: u64 = s
            .lines()
            .find(|l| l.starts_with("RssAnon:"))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        if kb == 0 {
            return String::new();
        }
        format!("Vynz şu an yaklaşık {} MB bellek kullanıyor.", kb / 1024)
    }
    #[cfg(not(target_os = "linux"))]
    {
        String::new()
    }
}

thread_local! {
    /// Uyku zamanlayıcısının sesi kısma zamanlayıcısı (çalışırken canlı tutulur)
    static SLEEP_FADE: RefCell<Option<Timer>> = const { RefCell::new(None) };
}

/// Uyku süresi doldu: sesi 8 sn'de kıs, duraklat, ses düzeyini geri ver.
fn sleep_fire(ui: &AppWindow) {
    let v0 = ui.get_volume() as f64;
    let steps = 32u32;
    let n = std::rc::Rc::new(std::cell::Cell::new(0u32));
    let t = Timer::default();
    let weak = ui.as_weak();
    t.start(TimerMode::Repeated, Duration::from_millis(250), move || {
        let Some(ui) = weak.upgrade() else { return };
        let k = n.get() + 1;
        n.set(k);
        // Kullanıcı bu arada duraklattıysa ya da yeni süre kurduysa bırak
        let cancelled = !ui.get_playing() || ui.get_sleep_left() > 0;
        if k >= steps || cancelled {
            with_state(|st| {
                if !cancelled {
                    st.player.send(Cmd::SetPause(true));
                }
                st.player.send(Cmd::Volume(v0));
            });
            SLEEP_FADE.with(|f| f.borrow_mut().take());
            return;
        }
        let f = 1.0 - k as f64 / steps as f64;
        with_state(|st| st.player.send(Cmd::Volume(v0 * f * f)));
    });
    SLEEP_FADE.with(|f| *f.borrow_mut() = Some(t));
}

/// Bekçi: parça 8 sn'de ilerlemezse adres bozuktur (ör. yavaşlatılmış akış); yedek yoldan aç.
fn watchdog_check(ui: &AppWindow) {
    let stuck = with_state(|st| {
        let Some(at) = st.load_at else { return None };
        if st.progressed || at.elapsed() < Duration::from_secs(8) || !ui.get_playing() {
            return None;
        }
        st.load_at = None;
        st.index.and_then(|i| st.queue.get(i)).map(|t| t.key())
    });
    if let Some(key) = stuck {
        dbg(&format!("takıldı {key}: adres unutuldu, yedek yol"));
        with_state(|st| st.resolver.invalidate(&key));
        on_error(ui, "akış ilerlemedi".into());
    }
}

fn on_error(ui: &AppWindow, e: String) {
    let retried = with_state(|st| {
        if st.fallback_tried {
            return false;
        }
        st.fallback_tried = true;
        st.appended = None;
        let Some(t) = st.index.and_then(|i| st.queue.get(i).cloned()) else { return false };
        match page_url(st, &t) {
            Some(page) => {
                st.player.send(Cmd::Load { url: page, title: media_title(&t), start: st.pending_start });
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
            // Atlamadan hemen sonra gelen eski konumları gösterme
            let stale = with_state(|st| match st.seek_guard {
                Some((target, at)) if ((t as f32) - target).abs() > 1.5 && at.elapsed() < Duration::from_secs(2) => true,
                Some(_) => {
                    st.seek_guard = None;
                    false
                }
                None => false,
            });
            if stale {
                return;
            }
            if ui.get_playing() && with_state(|st| st.last_session_save.elapsed() > Duration::from_secs(10)) {
                save_session(ui);
            }
            with_state(|st| {
                if t > 0.4 {
                    st.progressed = true;
                }
                st.mpris.send(mpris::Msg::Position(t));
                if st.seek_pending {
                    st.seek_pending = false;
                    st.mpris.send(mpris::Msg::Seeked(t));
                }
                if let Some((_, played, last)) = st.listen.as_mut() {
                    let d = t as f32 - *last;
                    if d > 0.0 && d <= 1.5 {
                        *played += d; // ileri sarma sayılmaz
                    }
                    *last = t as f32;
                }
            });
            ui.set_position(t as f32);
            if std::env::var_os("VYNZ_LYRDBG").is_some() {
                eprintln!("LYR TIME t={t:.3}");
            }
            with_state(|st| st.last_pos = (t as f32, std::time::Instant::now()));
            update_lyrics(ui, t as f32);
            if t >= 20.0 {
                queue_for_cache();
            }
        }
        Ev::Duration(d) => {
            with_state(|st| st.mpris.send(mpris::Msg::Length(d)));
            ui.set_duration(d as f32)
        }
        Ev::Paused(p) => {
            // Geri yüklenmiş ama henüz yüklenmemiş parça: mpv'nin açılıştaki "pause=no" bildirimi oynatıyor sanılmasın.
            if with_state(|st| !st.loaded && st.resume_pos.is_some()) {
                return;
            }
            if p && ui.get_has_now() {
                save_session(ui);
            }
            if ui.get_has_now() {
                with_state(|st| st.mpris.send(mpris::Msg::Playing(!p)));
            }
            ui.set_playing(!p && ui.get_has_now());
            // Dolum duraklatınca bulunduğu yerde dursun, devam edince akmaya devam etsin.
            update_lyrics(ui, with_state(|st| st.last_pos.0));
            update_viz(ui);
        }
        Ev::Buffering(b) => {
            if !b {
                dbg("ses başladı");
            }
            ui.set_buffering(b)
        }
        Ev::Started(pos) => on_started(ui, pos),
        Ev::Seeked => with_state(|st| st.seek_pending = true),
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
    if cache::exists(&key).is_some() || st.resolver.cached(&key).is_some() || st.resolver.is_pending(&key) {
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

fn is_metered_connection() -> bool {
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("nmcli")
            .args(["-t", "-f", "GENERAL.METERED", "dev", "show"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).lines().any(|l| l.contains(":yes")))
            .unwrap_or(false)
    }
    #[cfg(target_os = "windows")]
    {
        use windows::Networking::Connectivity::{NetworkCostType, NetworkInformation};
        NetworkInformation::GetInternetConnectionProfile()
            .and_then(|p| p.GetConnectionCost())
            .map(|c| {
                let t = c.NetworkCostType().unwrap_or(NetworkCostType::Unknown);
                matches!(t, NetworkCostType::Fixed | NetworkCostType::Variable)
                    || c.Roaming().unwrap_or(false)
                    || c.OverDataLimit().unwrap_or(false)
            })
            .unwrap_or(false)
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        false
    }
}

fn is_on_battery() -> bool {
    let Ok(mgr) = starship_battery::Manager::new() else { return false };
    let Ok(bats) = mgr.batteries() else { return false };
    bats.flatten().any(|b| b.state() == starship_battery::State::Discharging)
}

fn start_env_monitor(metered: Arc<AtomicBool>, on_battery: Arc<AtomicBool>) {
    thread::spawn(move || loop {
        metered.store(is_metered_connection(), Ordering::Relaxed);
        on_battery.store(is_on_battery(), Ordering::Relaxed);
        thread::sleep(Duration::from_secs(60));
    });
}

// ---------- Radyo ----------

fn radio_track(r: resolve::RadioTrack) -> TrackInfo {
    TrackInfo {
        source: Source::Ytm,
        thumb: sources::ytm::big_thumb(&r.thumb),
        id: r.id,
        title: r.title,
        artist: r.artist,
        album: r.album,
        duration: r.duration,
        url: None,
    }
}

/// Bir şarkının radyosu (benzerleri). SoundCloud şarkısıysa önce YouTube Music karşılığı bulunur.
fn fetch_radio(seed: TrackInfo, done: impl FnOnce(Result<Vec<TrackInfo>, String>) + Send + 'static) {
    let (resolver, ytm) = with_state(|st| (st.resolver.clone(), provider(st, Source::Ytm)));
    thread::spawn(move || {
        let id = if seed.source == Source::Ytm {
            Some(seed.id.clone())
        } else {
            ytm.and_then(|p| p.search(&format!("{} {}", seed.title, seed.artist), 1).ok()).and_then(|r| r.into_iter().next()).map(|t| t.id)
        };
        let Some(id) = id else {
            done(Err("YouTube Music'te karşılığı bulunamadı".into()));
            return;
        };
        let (tx, rx) = mpsc::channel();
        resolver.radio(&id, Prio::Now, Box::new(move |r| {
            let _ = tx.send(r);
        }));
        match rx.recv_timeout(Duration::from_secs(20)) {
            Ok(Ok(list)) => done(Ok(list.into_iter().map(radio_track).collect())),
            Ok(Err(e)) => done(Err(e)),
            Err(_) => done(Err("radyo zaman aşımı".into())),
        }
    });
}

/// Seçilen şarkıdan radyo başlat: şarkı + benzerleri yeni kuyruk olur.
fn start_radio(ui: &AppWindow, seed: TrackInfo) {
    let playing_same = ui.get_has_now() && ui.get_now().key == seed.key().as_str();
    with_state(|st| st.radio_busy = true);
    ui.set_radio_busy(true);
    toast(ui, &format!("“{}” radyosu hazırlanıyor…", seed.title));
    let weak = ui.as_weak();
    let seed2 = seed.clone();
    fetch_radio(seed, move |res| {
        let _ = weak.upgrade_in_event_loop(move |ui| {
            with_state(|st| st.radio_busy = false);
            ui.set_radio_busy(false);
            match res {
                Ok(list) => {
                    let mut q = vec![seed2.clone()];
                    let filtered: Vec<TrackInfo> = with_state(|st| {
                        list.into_iter().filter(|t| t.key() != seed2.key() && !st.history.is_disliked(t)).collect()
                    });
                    q.extend(filtered);
                    if playing_same {
                        // Çalan şarkı kesilmeden kuyruğun geri kalanı radyo olur.
                        with_state(|st| {
                            st.queue = q;
                            st.index = Some(0);
                            st.shuffle_orig = None;
                        });
                        queue_changed(&ui);
                    } else {
                        play_list(&ui, q, 0);
                    }
                    toast(&ui, &format!("Radyo: {}", seed2.title));
                }
                Err(e) => toast(&ui, &format!("Radyo alınamadı: {e}")),
            }
        });
    });
}

/// Kuyruk bitmek üzere/bitti: çalan şarkının radyosundan, kuyrukta olmayanları ekle.
fn extend_with_radio(ui: &AppWindow, then_play: bool) {
    let seed = with_state(|st| {
        if st.radio_busy {
            return None;
        }
        let t = st.index.and_then(|i| st.queue.get(i).cloned())?;
        st.radio_busy = true;
        Some(t)
    });
    let Some(seed) = seed else { return };
    dbg(&format!("liste uzatılıyor: {}", seed.key()));
    let weak = ui.as_weak();
    fetch_radio(seed, move |res| {
        let _ = weak.upgrade_in_event_loop(move |ui| {
            let added = with_state(|st| {
                st.radio_busy = false;
                let Ok(list) = res else { return 0 };
                let have: HashSet<String> = st.queue.iter().map(TrackInfo::key).collect();
                let skipped = st.history.recently_skipped(14);
                let new: Vec<TrackInfo> = list
                    .into_iter()
                    .filter(|t| !have.contains(&t.key()) && !skipped.contains(&t.key()) && !st.history.is_disliked(t))
                    .take(25)
                    .collect();
                let n = new.len();
                st.queue.extend(new);
                n
            });
            if added > 0 {
                queue_changed(&ui);
                if then_play {
                    step(&ui, 1);
                }
            } else if then_play {
                ui.set_playing(false);
            }
        });
    });
}

// ---------- Kuyruk ----------

/// Kuyruk değişti: arayüzü tazele, mpv'deki sıradaki artık doğru değilse yeniden hazırla.
fn queue_changed(ui: &AppWindow) {
    mpris_caps();
    let (list, idx) = with_state(|st| {
        if st.appended.is_some() {
            st.player.send(Cmd::ClearNext);
            st.appended = None;
        }
        for t in st.queue.clone().iter().take(40) {
            request_art(st, t);
        }
        (st.queue.clone(), st.index)
    });
    ui.set_queue(model_of(&list));
    ui.set_queue_index(idx.map(|i| i as i32).unwrap_or(-1));
    schedule_prefetch(ui);
}

fn queue_remove(ui: &AppWindow, i: usize) {
    let current = with_state(|st| st.index == Some(i));
    if current {
        // Çalanı çıkarmak = sonrakine geç ve onu listeden sil.
        let t = with_state(|st| {
            st.index = None;
            st.queue.remove(i)
        });
        if with_state(|st| i < st.queue.len()) {
            play_index(ui, i);
        }
        queue_changed(ui);
        toast_undo(ui, "Kuyruktan çıkarıldı", Undo::QueueItem { pos: i, t });
        return;
    }
    let removed = with_state(|st| {
        if i >= st.queue.len() {
            return None;
        }
        let t = st.queue.remove(i);
        if let Some(cur) = st.index {
            if i < cur {
                st.index = Some(cur - 1);
            }
        }
        Some(t)
    });
    queue_changed(ui);
    if let Some(t) = removed {
        toast_undo(ui, "Kuyruktan çıkarıldı", Undo::QueueItem { pos: i, t });
    }
}

/// `from`daki şarkıyı `to` konumuna taşıdıktan sonra çalan şarkının yeni indeksi.
fn moved_index(cur: usize, from: usize, to: usize) -> usize {
    if cur == from {
        to
    } else if from < cur && cur <= to {
        cur - 1
    } else if to <= cur && cur < from {
        cur + 1
    } else {
        cur
    }
}

/// Kuyrukta `i`deki şarkıyı `delta` kadar öteye taşı (sürükle-bırak: araya girer, takas değil).
fn queue_move(ui: &AppWindow, i: usize, delta: i32) {
    let ok = with_state(|st| {
        let j = i as i64 + delta as i64;
        if delta == 0 || j < 0 || j as usize >= st.queue.len() || i >= st.queue.len() {
            return false;
        }
        let j = j as usize;
        let t = st.queue.remove(i);
        st.queue.insert(j, t);
        st.index = st.index.map(|c| moved_index(c, i, j));
        true
    });
    if ok {
        queue_changed(ui);
    }
}

fn queue_clear(ui: &AppWindow) {
    let (old, old_index) = with_state(|st| (st.queue.clone(), st.index));
    with_state(|st| {
        if let Some(t) = st.index.and_then(|i| st.queue.get(i).cloned()) {
            st.queue = vec![t];
            st.index = Some(0);
        } else {
            st.queue.clear();
        }
        st.shuffle_orig = None;
    });
    queue_changed(ui);
    if old.len() > 1 {
        toast_undo(ui, &format!("Kuyruk temizlendi ({} şarkı)", old.len() - 1), Undo::Queue { queue: old, index: old_index });
    }
}

fn play_next(ui: &AppWindow, t: TrackInfo) {
    let empty = with_state(|st| st.index.is_none());
    if empty {
        play_list(ui, vec![t], 0);
        return;
    }
    with_state(|st| {
        let key = t.key();
        let cur = st.index.unwrap_or(0);
        // Kuyrukta (çalandan sonra) zaten varsa kopyalama, öne al.
        if let Some(j) = st.queue.iter().enumerate().skip(cur + 1).find(|(_, x)| x.key() == key).map(|(j, _)| j) {
            st.queue.remove(j);
        }
        let at = (cur + 1).min(st.queue.len());
        st.queue.insert(at, t);
    });
    queue_changed(ui);
}

fn add_to_queue(ui: &AppWindow, t: TrackInfo) {
    let empty = with_state(|st| st.index.is_none());
    if empty {
        play_list(ui, vec![t], 0);
        return;
    }
    with_state(|st| st.queue.push(t));
    queue_changed(ui);
}

fn toggle_shuffle(ui: &AppWindow) {
    let on = with_state(|st| {
        let Some(i) = st.index else { return false };
        if let Some(orig) = st.shuffle_orig.take() {
            // Kapat: özgün sıraya dön, çalan şarkının yerini bul.
            let key = st.queue[i].key();
            st.index = orig.iter().position(|t| t.key() == key).or(Some(0));
            st.queue = orig;
            false
        } else {
            st.shuffle_orig = Some(st.queue.clone());
            // Çalandan sonrasını karıştır (basit Fisher-Yates, zamana dayalı tohum).
            let mut seed = history::now() ^ 0x9E37_79B9_7F4A_7C15;
            let rest = &mut st.queue[i + 1..];
            for k in (1..rest.len()).rev() {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                let j = (seed % (k as u64 + 1)) as usize;
                rest.swap(k, j);
            }
            true
        }
    });
    ui.set_shuffle_on(on);
    with_state(|st| st.mpris.send(mpris::Msg::Shuffle(on)));
    queue_changed(ui);
}

fn cycle_repeat(ui: &AppWindow) {
    let mode = with_state(|st| {
        st.repeat = (st.repeat + 1) % 3;
        st.player.send(Cmd::LoopFile(st.repeat == 2));
        st.repeat
    });
    ui.set_repeat_mode(mode);
    with_state(|st| st.mpris.send(mpris::Msg::Loop(mode)));
}

fn refresh_liked(ui: &AppWindow) {
    let likes = with_state(|st| {
        let list = st.history.likes.clone();
        for t in list.iter().take(40) {
            request_art(st, t);
        }
        list
    });
    ui.set_liked_count(likes.len() as i32);
    ui.set_liked_duration(format_total_duration(&likes).into());
    ui.set_liked_tracks(model_of(&likes));
}

// ---------- İndirilenler ----------

fn refresh_downloads(ui: &AppWindow) {
    let (list, pending) = with_state(|st| {
        let list = st.dl_list.clone();
        for t in list.iter().take(40) {
            request_art(st, t);
        }
        (list, st.dl_pending.clone())
    });
    ui.set_dl_count(list.len() as i32);
    ui.set_dl_tracks(model_of(&list));
    let active: Vec<DlItem> = pending
        .iter()
        .map(|(t, p)| DlItem { title: format!("{} — {}", t.artist, t.title).into(), progress: *p })
        .collect();
    ui.set_dl_active(ModelRc::new(VecModel::from(active)));
    let weak = ui.as_weak();
    thread::spawn(move || {
        let used = downloads::usage();
        let _ = weak.upgrade_in_event_loop(move |ui| ui.set_dl_size(human_bytes(used).into()));
    });
}

/// Şarkıları indir: önce akış adresi çözülür, sonra indirici sırasına girer.
fn download_tracks(ui: &AppWindow, tracks: Vec<TrackInfo>) {
    let mut added = 0;
    let mut sc = 0;
    for t in tracks {
        let key = t.key();
        let skip = with_state(|st| {
            downloads::path(&key).is_some() || st.dl_pending.iter().any(|(x, _)| x.key() == key)
        });
        if skip {
            continue;
        }
        if t.source == Source::SoundCloud {
            sc += 1; // HLS parça listesi; şimdilik indirilemiyor
            continue;
        }
        let Some(page) = with_state(|st| page_url(st, &t)) else { continue };
        added += 1;
        let weak = ui.as_weak();
        with_state(|st| {
            st.dl_pending.push((t.clone(), 0.0));
            let tt = t.clone();
            st.resolver.resolve(
                &key,
                &page,
                Prio::Soon,
                Some(Box::new(move |res| {
                    let _ = weak.upgrade_in_event_loop(move |ui| match res {
                        Ok(r) if !r.stream.contains(".m3u8") => with_state(|st| st.downloader.enqueue(tt, r.stream)),
                        Ok(_) => on_download_event(&ui, downloads::Event::Failed(tt, "akış indirilemez".into())),
                        Err(e) => on_download_event(&ui, downloads::Event::Failed(tt, e)),
                    });
                })),
            );
        });
    }
    if added > 0 {
        toast(ui, &if added == 1 { "İndiriliyor…".to_string() } else { format!("{added} şarkı indiriliyor…") });
    } else if sc > 0 {
        toast(ui, "SoundCloud şarkıları şimdilik indirilemiyor");
    } else {
        toast(ui, "Zaten indirilmiş");
    }
    refresh_downloads(ui);
}

fn on_download_event(ui: &AppWindow, ev: downloads::Event) {
    match ev {
        downloads::Event::Progress(key, p) => {
            with_state(|st| {
                if let Some(x) = st.dl_pending.iter_mut().find(|(t, _)| t.key() == key) {
                    x.1 = p;
                }
            });
            // Yalnız ilerleme çubuklarını tazele (liste aynı)
            let pending = with_state(|st| st.dl_pending.clone());
            let active: Vec<DlItem> = pending.iter().map(|(t, p)| DlItem { title: format!("{} — {}", t.artist, t.title).into(), progress: *p }).collect();
            ui.set_dl_active(ModelRc::new(VecModel::from(active)));
            return;
        }
        downloads::Event::Done(t) => {
            with_state(|st| {
                let key = t.key();
                st.dl_pending.retain(|(x, _)| x.key() != key);
                if !st.dl_list.iter().any(|x| x.key() == key) {
                    st.dl_list.insert(0, t);
                }
                downloads::save(&st.dl_list);
            });
        }
        downloads::Event::Failed(t, e) => {
            eprintln!("indirilemedi {}: {e}", t.key());
            with_state(|st| st.dl_pending.retain(|(x, _)| x.key() != t.key()));
            toast(ui, &format!("İndirilemedi: {}", t.title));
        }
    }
    refresh_downloads(ui);
}

fn remove_download(ui: &AppWindow, key: &str) {
    with_state(|st| {
        st.dl_list.retain(|x| x.key() != key);
        downloads::save(&st.dl_list);
    });
    downloads::remove_files(key);
    toast(ui, "İndirilenlerden kaldırıldı");
    refresh_downloads(ui);
}

fn toggle_like_current(ui: &AppWindow) {
    let liked = with_state(|st| {
        let t = st.index.and_then(|i| st.queue.get(i).cloned())?;
        Some(st.history.toggle_like(&t))
    });
    if let Some(l) = liked {
        ui.set_liked(l);
        refresh_liked(ui);
        toast(ui, if l { "Beğendiklerine eklendi" } else { "Beğendiklerinden çıkarıldı" });
    }
}

// ---------- Sağ tık menüsü ----------

/// Menünün hedef şarkısı: kind 0 raf (a, b), 1 arama sonucu, 2 kayıtlı liste, 3 kuyruk, 4 açık koleksiyon, 5 sanatçı, 6 beğendiklerim.
fn menu_target(st: &State, kind: i32, a: i32, b: i32) -> Option<TrackInfo> {
    let (a, b) = (a as usize, b as usize);
    match kind {
        0 => st.shelves.get(a).and_then(|s| s.items.get(b)).cloned(),
        1 => st.results.get(a).cloned(),
        2 => st.current_pl.and_then(|p| st.playlists.get(p)).and_then(|p| {
            let real = st.pl_visible.get(a).copied().unwrap_or(a);
            p.tracks.get(real)
        }).cloned(),
        3 => st.queue.get(a).cloned(),
        4 => st.coll.as_ref().and_then(|c| c.1.get(a)).cloned(),
        5 => st.artist.as_ref().and_then(|c| c.1.get(a)).cloned(),
        6 => st.history.likes.get(a).cloned(),
        7 => st.index.and_then(|i| st.queue.get(i)).cloned(),
        8 => st.dl_list.get(a).cloned(),
        _ => None,
    }
}

fn menu_action(ui: &AppWindow, action: i32) {
    let Some(t) = with_state(|st| st.menu.take()) else { return };
    match action {
        0 => play_list(ui, vec![t], 0),
        1 => {
            toast(ui, &format!("Sıradaki: {}", t.title));
            play_next(ui, t);
        }
        2 => {
            toast(ui, &format!("Kuyruğa eklendi: {}", t.title));
            add_to_queue(ui, t);
        }
        3 => start_radio(ui, t),
        10 => {
            if downloads::path(&t.key()).is_some() {
                remove_download(ui, &t.key());
            } else {
                download_tracks(ui, vec![t]);
            }
        }
        9 => {
            let on = with_state(|st| st.history.toggle_boost(&t));
            toast(ui, if on { "Benzerleri daha sık önerilecek; ana sayfa yenilenince görünür" } else { "Artık öne çıkarılmıyor" });
        }
        4 => {
            let l = with_state(|st| st.history.toggle_like(&t));
            if ui.get_now().key == t.key().as_str() {
                ui.set_liked(l);
            }
            refresh_liked(ui);
            toast(ui, if l { "Beğendiklerine eklendi" } else { "Beğendiklerinden çıkarıldı" });
        }
        5 => {
            let artist = t.artist.split(',').next().unwrap_or("").trim().to_string();
            if !artist.is_empty() {
                ui.invoke_go("search".into());
                search(ui, artist);
            }
        }
        6 => {
            let removed = with_state(|st| {
                let pi = st.current_pl?;
                let pl = st.playlists.get_mut(pi)?;
                let pos = pl.tracks.iter().position(|x| x.key() == t.key())?;
                let t = pl.tracks.remove(pos);
                let _ = library::save(&st.playlists);
                Some((pi, pos, t))
            });
            if let Some((pi, pos, t)) = removed {
                open_playlist(ui, pi);
                refresh_playlists(ui);
                toast_undo(ui, "Listeden çıkarıldı", Undo::PlaylistTrack { pl: pi, pos, t });
            }
        }
        7 => {
            let (home_data, was_liked) = with_state(|st| {
                let was_liked = st.history.is_liked(&t.key());
                st.history.dislike_track(&t);
                ((!st.home.personal.is_empty() || !st.home.external.is_empty()).then(|| st.home.clone()), was_liked)
            });
            refresh_hidden(ui);
            refresh_liked(ui);
            if let Some(d) = home_data {
                show_home(ui, d);
            }
            if ui.get_has_now() && ui.get_now().key == t.key().as_str() {
                ui.set_liked(false);
                step(ui, 1);
            }
            let msg = format!("Artık önerilmeyecek: {}", t.title);
            toast_undo(ui, &msg, Undo::Dislike { t, liked: was_liked });
        }
        8 => {
            let artist = t.artist.split(',').next().unwrap_or("").trim().to_string();
            if artist.is_empty() {
                return;
            }
            let (home_data, skip_now) = with_state(|st| {
                st.history.dislike_artist(&artist);
                let skip = st.index.and_then(|i| st.queue.get(i)).is_some_and(|cur| st.history.is_disliked(cur));
                let hd = (!st.home.personal.is_empty() || !st.home.external.is_empty()).then(|| st.home.clone());
                (hd, skip)
            });
            refresh_hidden(ui);
            if let Some(d) = home_data {
                show_home(ui, d);
            }
            if skip_now {
                step(ui, 1);
            }
            toast_undo(ui, &format!("Artık önerilmeyecek sanatçı: {artist}"), Undo::DislikeArtist(artist.clone()));
        }
        _ => {}
    }
}

fn refresh_hidden(ui: &AppWindow) {
    let (artists, tracks) = with_state(|st| {
        let a: Vec<slint::SharedString> = st.history.disliked_artists.iter().map(|s| s.as_str().into()).collect();
        let t: Vec<slint::SharedString> = st
            .history
            .disliked_tracks
            .iter()
            .map(|x| {
                if x.artist.is_empty() {
                    x.title.as_str().into()
                } else {
                    format!("{} — {}", x.title, x.artist).into()
                }
            })
            .collect();
        (a, t)
    });
    ui.set_hidden_artists(ModelRc::new(VecModel::from(artists)));
    ui.set_hidden_tracks(ModelRc::new(VecModel::from(tracks)));
}

/// Seçicideki "Yeni liste": menü şarkısıyla birlikte oluştur, sayfayı değiştirme.
fn create_playlist_with_track(ui: &AppWindow, name: String) {
    let name = name.trim().to_string();
    if name.is_empty() {
        return;
    }
    let ok = with_state(|st| {
        let t = st.menu.take()?;
        st.playlists.insert(0, library::Playlist { name: name.clone(), origin: String::new(), tracks: vec![t], image: String::new() });
        // Açık liste indeksi bir kaydı.
        st.current_pl = st.current_pl.map(|i| i + 1);
        let _ = library::save(&st.playlists);
        Some(())
    });
    if ok.is_some() {
        if ui.get_current_playlist() >= 0 {
            ui.set_current_playlist(ui.get_current_playlist() + 1);
        }
        refresh_playlists(ui);
        toast(ui, &format!("'{name}' oluşturuldu ve şarkı eklendi"));
    }
}

fn add_menu_track_to_playlist(ui: &AppWindow, pl_idx: usize) {
    let res = with_state(|st| {
        let t = st.menu.take()?;
        let pl = st.playlists.get_mut(pl_idx)?;
        let name = pl.name.clone();
        if pl.tracks.iter().any(|x| x.key() == t.key()) {
            return Some((name, false));
        }
        pl.tracks.push(t);
        let _ = library::save(&st.playlists);
        Some((name, true))
    });
    if let Some((name, added)) = res {
        refresh_playlists(ui);
        if with_state(|st| st.current_pl) == Some(pl_idx) {
            open_playlist(ui, pl_idx);
        }
        toast(ui, &if added { format!("'{name}' listesine eklendi") } else { format!("Zaten '{name}' listesinde") });
    }
}

// ---------- Veri yükleme ----------

fn home_cache() -> std::path::PathBuf {
    cache::dir().parent().map(|p| p.join("home.json")).unwrap_or_default()
}

fn greeting() -> &'static str {
    let h = std::process::Command::new("date").arg("+%H").output().ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<u32>().ok()).unwrap_or(12);
    match h {
        5..=11 => "Günaydın",
        12..=17 => "İyi günler",
        18..=22 => "İyi akşamlar",
        _ => "İyi geceler",
    }
}

fn track_from_json(v: &serde_json::Value) -> Option<TrackInfo> {
    Some(TrackInfo {
        source: Source::Ytm,
        id: v.get("id")?.as_str()?.to_string(),
        title: v.get("title").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        artist: v.get("artist").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        album: v.get("album").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        duration: v.get("duration").and_then(|x| x.as_f64()).unwrap_or(0.0) as f32,
        thumb: sources::ytm::big_thumb(v.get("thumb").and_then(|x| x.as_str()).unwrap_or("")),
        url: None,
    })
}

fn cards_from_json(v: Option<&serde_json::Value>) -> Vec<Card> {
    v.and_then(|x| x.as_array())
        .map(|a| a.iter().filter_map(|c| serde_json::from_value::<Card>(c.clone()).ok()).map(|mut c| {
            c.thumb = sources::ytm::big_thumb(&c.thumb);
            c
        }).collect())
        .unwrap_or_default()
}

/// Keşfet verisinden raflar: trend şarkılar, ruh hâli, yeni çıkanlar, popüler sanatçılar.
fn explore_sections(d: &serde_json::Value) -> Vec<Section> {
    let mut out = Vec::new();
    let trending: Vec<TrackInfo> = d.get("trending").and_then(|x| x.as_array()).map(|a| a.iter().filter_map(track_from_json).collect()).unwrap_or_default();
    if !trending.is_empty() {
        out.push(Section { title: "Trend şarkılar".into(), items: trending, cards: Vec::new(), style: 0 });
    }
    for (key, title, style) in [("moods", "Ruh hâli ve türler", 4), ("releases", "Yeni albümler", 2), ("artists", "Türkiye'de popüler sanatçılar", 3)] {
        let cards = cards_from_json(d.get(key));
        if !cards.is_empty() {
            out.push(Section { title: title.into(), items: Vec::new(), cards, style });
        }
    }
    out
}

fn artists_summary(tracks: &[TrackInfo]) -> String {
    let mut seen = HashSet::new();
    let mut names = Vec::new();
    for t in tracks {
        let a = t.artist.split(',').next().unwrap_or("").trim();
        if !a.is_empty() && seen.insert(a.to_lowercase()) {
            names.push(a.to_string());
            if names.len() >= 3 {
                break;
            }
        }
    }
    if names.is_empty() {
        format!("{} şarkı", tracks.len())
    } else {
        format!("{} ve daha fazlası", names.join(", "))
    }
}

/// Ana sayfayı diz: tekrar önlemeli, aşağı indikçe değişen görsel şablonlarla ritmik akış.
fn compose(st: &State, h: &HomeData) -> Vec<Section> {
    let p = &h.personal;
    let find_title = |list: &[Section], title: &str| list.iter().find(|s| s.title == title).cloned();
    let mut out = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    let dedup = |items: Vec<TrackInfo>, seen: &mut HashSet<String>, keep_min: usize| -> Vec<TrackInfo> {
        let allowed: Vec<TrackInfo> = items.into_iter().filter(|t| !st.history.is_disliked(t)).collect();
        let mut fresh = Vec::new();
        for t in &allowed {
            if !seen.contains(&t.key()) {
                fresh.push(t.clone());
            }
        }
        let chosen = if fresh.len() >= keep_min { fresh } else { allowed };
        for t in &chosen {
            seen.insert(t.key());
        }
        chosen
    };

    // 1. Hızlı erişim (style 1: kompakt 2×4 kutucuklar)
    let quick = recommend::quick_picks(&st.history, p, &h.external);
    if !quick.is_empty() {
        let q: Vec<TrackInfo> = quick.into_iter().take(8).collect();
        for t in &q {
            seen.insert(t.key());
        }
        out.push(Section { title: "Hızlı erişim".into(), items: q, cards: Vec::new(), style: 1 });
    }

    // 2. Öne çıkan karışımlar (style 5: 2'li büyük vitrin kartları)
    let mix_sec = p.iter().find(|s| s.title.ends_with("için karışım")).cloned();
    let disc_sec = find_title(p, "Yeni keşifler");
    let mut feature_cards = Vec::new();
    if let Some(m) = &mix_sec {
        if !m.items.is_empty() {
            feature_cards.push(Card {
                kind: "mix".into(),
                key: "mix".into(),
                title: m.title.clone(),
                sub: artists_summary(&m.items),
                thumb: m.items[0].thumb.clone(),
            });
        }
    }
    if let Some(d) = &disc_sec {
        if !d.items.is_empty() {
            feature_cards.push(Card {
                kind: "discover".into(),
                key: "discover".into(),
                title: "Yeni keşifler".into(),
                sub: artists_summary(&d.items),
                thumb: d.items[0].thumb.clone(),
            });
        }
    }
    if !feature_cards.is_empty() {
        out.push(Section { title: "Sana özel".into(), items: Vec::new(), cards: feature_cards, style: 5 });
    }

    let ext_tracks: Vec<Section> = h.external.iter().filter(|s| s.style == 0).cloned().collect();
    let similar: Vec<Section> = p.iter().filter(|s| s.title.ends_with(" ve benzerleri")).cloned().collect();
    // Geçmiş, öne çıkarılanlar ya da Zevkler varsa sayfa kişisel raflarla başlar; genel
    // Türkiye vitrini en alta "Keşfet" olarak iner. Hiçbiri yoksa vitrin yedek olarak üstte kalır.
    let personalized = !st.history.is_empty() || !st.settings.taste_tracks.is_empty() || !st.settings.taste_genres.is_empty();

    let spotlight = |out: &mut Vec<Section>, seen: &mut HashSet<String>, src: Option<Section>| {
        if let Some(sp) = src {
            let items = dedup(sp.items, seen, 4);
            if items.len() >= 4 {
                let title = if sp.title.ends_with(" ve benzerleri") {
                    format!("Çünkü {} dinliyorsun", sp.title.trim_end_matches(" ve benzerleri"))
                } else {
                    sp.title
                };
                out.push(Section { title, items, cards: Vec::new(), style: 7 });
            }
        }
    };
    let recent = |out: &mut Vec<Section>, seen: &mut HashSet<String>| {
        if let Some(rec) = find_title(p, "Kaldığın yerden") {
            let items = dedup(rec.items, seen, 4);
            if items.len() >= 3 {
                out.push(Section { title: rec.title, items, cards: Vec::new(), style: 0 });
            }
        }
    };

    if personalized {
        // 3. Çünkü X dinliyorsun (odak bandı)
        spotlight(&mut out, &mut seen, similar.first().cloned().or_else(|| mix_sec.clone()));
        // 4. Zevkine göre çalma listeleri (topluluk listeleri, sanatçılarından ve türlerinden)
        if let Some(pl) = find_title(p, TASTE_PLAYLISTS) {
            out.push(pl);
        }
        // 5. İkinci sevilen sanatçının benzerleri
        if let Some(s) = similar.get(1) {
            let items = dedup(s.items.clone(), &mut seen, 6);
            if items.len() >= 6 {
                out.push(Section { title: s.title.clone(), items, cards: Vec::new(), style: 0 });
            }
        }
        // 6. Kaldığın yerden
        recent(&mut out, &mut seen);
        // 7. Ruh hâli ve türler
        if let Some(moods) = find_title(&h.explore, "Ruh hâli ve türler") {
            out.push(moods);
        }
        // 8. Keşfet: Türkiye trendleri en altta, sevmediklerin ayıklanmış ve kısa.
        if let Some(c) = find_title(&h.explore, "Trend şarkılar") {
            let items: Vec<TrackInfo> = dedup(c.items, &mut seen, 6).into_iter().take(12).collect();
            if !items.is_empty() {
                out.push(Section { title: "Keşfet · Türkiye'de trend".into(), items, cards: Vec::new(), style: 6 });
            }
        }
        if let Some(albums) = find_title(&h.explore, "Yeni albümler") {
            out.push(albums);
        }
        if let Some(artists) = find_title(&h.explore, "Türkiye'de popüler sanatçılar") {
            out.push(artists);
        }
        return out;
    }

    // Kişisel sinyal yok: genel vitrin (ilk açılış ya da sıfırlama sonrası; Zevkler ekranı da açılır).
    let chart_src = find_title(&h.explore, "Trend şarkılar").or_else(|| ext_tracks.first().cloned());
    if let Some(c) = chart_src {
        let items = dedup(c.items, &mut seen, 6);
        if !items.is_empty() {
            out.push(Section { title: c.title, items, cards: Vec::new(), style: 6 });
        }
    }
    if let Some(artists) = find_title(&h.explore, "Türkiye'de popüler sanatçılar") {
        out.push(artists);
    }
    spotlight(&mut out, &mut seen, similar.first().cloned().or_else(|| mix_sec.clone()).or_else(|| ext_tracks.get(1).cloned()));
    if let Some(moods) = find_title(&h.explore, "Ruh hâli ve türler") {
        out.push(moods);
    }
    if let Some(albums) = find_title(&h.explore, "Yeni albümler") {
        out.push(albums);
    }
    recent(&mut out, &mut seen);
    for s in h.external.iter().filter(|s| s.style != 0).take(2) {
        out.push(s.clone());
    }
    out
}

const TASTE_PLAYLISTS: &str = "Zevkine göre çalma listeleri";

/// Ana sayfayı göster.
fn show_home(ui: &AppWindow, data: HomeData) {
    let all: Vec<Section> = with_state(|st| {
        st.mix = data.personal.iter().find(|s| s.title.ends_with("için karışım")).map(|s| s.items.clone()).unwrap_or_default();
        st.discover = data.personal.iter().find(|s| s.title == "Yeni keşifler").map(|s| s.items.clone()).unwrap_or_default();
        let all = compose(st, &data);
        st.home = data;
        for s in &all {
            for t in s.items.iter().take(if s.style == 1 || s.style == 6 { 12 } else { 8 }) {
                request_art(st, t);
            }
            for c in s.cards.iter().take(10) {
                request_card_art(st, c);
            }
        }
        if let Some(first) = all.first() {
            for t in first.items.iter().take(3) {
                speculate(st, t);
            }
        }
        // "Karışımı çal" anında başlasın.
        for t in st.mix.clone().iter().take(2) {
            speculate(st, t);
        }
        st.shelves = all.clone();
        all
    });
    dbg(&format!("raflar: {}", all.iter().map(|s| format!("{} ({})", s.title, s.items.len() + s.cards.len())).collect::<Vec<_>>().join(" | ")));
    let mix_shelf = all.iter().position(|s| s.title.ends_with("için karışım")).map(|i| i as i32).unwrap_or(-1);
    let shelves: Vec<Shelf> = all
        .iter()
        .map(|s| Shelf { title: s.title.as_str().into(), items: model_of(&s.items), cards: cards_of(&s.cards), style: s.style })
        .collect();
    ui.set_mix_shelf(mix_shelf);
    ui.set_shelves(ModelRc::new(VecModel::from(shelves)));
    ui.set_has_mix(with_state(|st| !st.mix.is_empty()));
    ui.set_greeting(greeting().into());
}

fn load_home(ui: &AppWindow) {
    // Önce son hesaplanan ana sayfa: açılış anında dolu bir sayfa.
    if let Some(cached) = std::fs::read(home_cache()).ok().and_then(|r| serde_json::from_slice::<HomeData>(&r).ok()) {
        show_home(ui, cached);
        ui.set_home_loading(false);
    }
    let (providers, resolver, history_snapshot, taste_tracks, taste_genres, blocked_genres, taste_cache) = with_state(|st| {
        (
            st.providers.clone(),
            st.resolver.clone(),
            st.history.snapshot(),
            st.settings.taste_tracks.clone(),
            st.settings.taste_genres.clone(),
            st.settings.blocked_genres.clone(),
            st.taste_cache.clone(),
        )
    });
    let weak = ui.as_weak();
    thread::spawn(move || {
        // Keşfet isteği hemen yola çıksın.
        let (etx, erx) = mpsc::channel();
        resolver.request(serde_json::json!({"op": "explore"}), Prio::Soon, Box::new(move |r| {
            let _ = etx.send(r);
        }));
        let mut external = Vec::new();
        let mut errors = Vec::new();
        for p in &providers {
            match p.home() {
                Ok(mut s) => external.append(&mut s),
                Err(e) => errors.push(format!("{}: {e}", source_name(p.source()))),
            }
        }
        // Kullanıcı tür seçti ama tek tek şarkı işaretlemediyse, seçili türlerden 1'er tohum şarkı al.
        let mut effective_taste_tracks = taste_tracks;
        if effective_taste_tracks.is_empty() && !taste_genres.is_empty() {
            if let Some(ytm) = providers.iter().find(|p| p.source() == Source::Ytm) {
                for gid in taste_genres.iter().take(4) {
                    if let Some(list) = taste_cache.get(gid) {
                        if let Some(t) = list.first() {
                            effective_taste_tracks.push(t.clone());
                            continue;
                        }
                    }
                    if let Some(q) = recommend::genre_query(gid) {
                        if let Ok(found) = ytm.search(q, 3) {
                            if let Some(t) = found.into_iter().next() {
                                effective_taste_tracks.push(t);
                            }
                        }
                    }
                }
            }
        }
        let mut blocked_artists = HashSet::new();
        for gid in &blocked_genres {
            if let Some(list) = taste_cache.get(gid) {
                for t in list {
                    blocked_artists.insert(history::artist_id(t));
                }
            }
        }
        let hist_seeds = recommend::seeds_with_tastes(&history_snapshot, &effective_taste_tracks);
        // Geçmiş ve Zevkler boşsa tohumlar YouTube Music'in genel önerilerinden.
        let seeds = if hist_seeds.is_empty() {
            external.iter().find(|s| s.style == 0).map(|s| s.items.iter().filter(|t| t.source == Source::Ytm).take(3).map(|t| (t.clone(), 0.6)).collect()).unwrap_or_default()
        } else {
            hist_seeds
        };
        let (tx, rx) = mpsc::channel();
        for (seed, _) in &seeds {
            let tx = tx.clone();
            let key = seed.key();
            resolver.radio(&seed.id, Prio::Soon, Box::new(move |r| {
                let _ = tx.send((key, r));
            }));
        }
        drop(tx);
        // Zevkine göre topluluk listeleri: en güçlü 3 tohumun sanatçısı + 2 seçili tür.
        let (ptx, prx) = mpsc::channel();
        let mut queries: Vec<String> = Vec::new();
        if !history_snapshot.is_empty() || !taste_genres.is_empty() || !effective_taste_tracks.is_empty() {
            for (s, _) in seeds.iter().take(3) {
                let a = s.artist.split(',').next().unwrap_or("").trim();
                if !a.is_empty() && !queries.iter().any(|q| q.eq_ignore_ascii_case(a)) {
                    queries.push(a.to_string());
                }
            }
            for gid in taste_genres.iter().take(2) {
                if let Some((_, name, _)) = recommend::GENRES.iter().find(|(g, _, _)| g == gid) {
                    queries.push(format!("{name} karışım"));
                }
            }
        }
        if !queries.is_empty() {
            resolver.request(serde_json::json!({"op": "playlists", "queries": queries, "per": 3}), Prio::Soon, Box::new(move |r| {
                let _ = ptx.send(r);
            }));
        } else {
            drop(ptx);
        }
        let mut radios = HashMap::new();
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while radios.len() < seeds.len() {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match rx.recv_timeout(left) {
                Ok((key, Ok(list))) => {
                    radios.insert(key, list.into_iter().map(radio_track).collect::<Vec<_>>());
                }
                Ok((_, Err(e))) => errors.push(format!("radyo: {e}")),
                Err(_) => break,
            }
        }
        let explore = match erx.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(d)) => explore_sections(&d),
            Ok(Err(e)) => {
                errors.push(format!("keşfet: {e}"));
                Vec::new()
            }
            Err(_) => Vec::new(),
        };
        let mut personal = recommend::build_with_blocked(&history_snapshot, &seeds, &radios, &blocked_artists);
        if let Ok(Ok(d)) = prx.recv_timeout(Duration::from_secs(8)) {
            let cards = cards_from_json(d.get("cards"));
            if cards.len() >= 3 {
                personal.push(Section { title: TASTE_PLAYLISTS.into(), items: Vec::new(), cards, style: 2 });
            }
        }
        let data = HomeData { personal, external, explore };
        if !data.personal.is_empty() || !data.external.is_empty() {
            let _ = std::fs::write(home_cache(), serde_json::to_vec(&data).unwrap_or_default());
        }
        let _ = weak.upgrade_in_event_loop(move |ui| {
            let empty = data.external.is_empty() && data.explore.is_empty();
            show_home(&ui, data);
            ui.set_home_loading(false);
            if let Some(e) = errors.first() {
                if empty {
                    toast(&ui, &format!("Öneriler alınamadı: {e}"));
                }
            }
        });
    });
}

// ---------- Koleksiyonlar (liste, albüm, sanatçı, ruh hâli) ----------

/// Üst sayfayı aç: ruh hâli → liste iki katlı olabilir; aynı türden yeni sayfa üsttekinin yerine geçer.
fn open_sheet(ui: &AppWindow, kind: &str) {
    let closed = ui.get_sheet().is_empty(); // kenar çubuğu go() ile kapatmış olabilir
    with_state(|st| {
        if closed {
            st.sheets.clear();
        }
        if st.sheets.last().map(|x| x.as_str()) == Some(kind) {
            st.sheets.pop();
        }
        st.sheets.retain(|x| x != kind);
        st.sheets.push(kind.to_string());
    });
    ui.set_lyrics_open(false);
    ui.set_sheet(kind.into());
    ui.set_sheet_gen(ui.get_sheet_gen() + 1);
}

/// "A, B & C feat. D" → ["A", "B", "C", "D"] (sırayla, tekrarsız).
fn split_artists(s: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut t = s.to_string();
    for sep in [" & ", " feat. ", " ft. ", " Feat. ", " x ", " X ", " ve "] {
        t = t.replace(sep, ",");
    }
    for p in t.split(',') {
        let p = p.trim();
        if !p.is_empty() && p != "—" && !out.iter().any(|x| x == p) {
            out.push(p.to_string());
        }
    }
    out
}

/// Sanatçı adından sanatçı sayfası (kanal kimliğini çözücü bulur).
fn open_artist_by_name(ui: &AppWindow, name: &str, vid: &str) {
    let c = Card { kind: "artist".into(), key: format!("find:{name}\t{vid}"), title: name.to_string(), sub: String::new(), thumb: String::new() };
    open_artist(ui, c);
}

/// Sanatçı sayfası: afiş, popüler şarkılar, albümler, single'lar, benzer sanatçılar, hakkında.
fn open_artist(ui: &AppWindow, c: Card) {
    ui.set_artist_name(c.title.as_str().into());
    ui.set_artist_sub("".into());
    ui.set_artist_desc("".into());
    ui.set_artist_tracks(ModelRc::new(VecModel::from(Vec::<Track>::new())));
    ui.set_artist_shelves(ModelRc::new(VecModel::from(Vec::<Shelf>::new())));
    ui.set_has_banner(false);
    ui.set_artist_expanded(false);
    ui.set_artist_loading(true);
    open_sheet(ui, "artist");
    let seq = with_state(|st| {
        st.coll_gen += 1;
        st.artist = None;
        st.banner_key = format!("banner-{}", c.art_key());
        st.coll_gen
    });
    let resolver = with_state(|st| st.resolver.clone());
    let weak = ui.as_weak();
    let req = serde_json::json!({"op": "collection", "kind": "artist", "key": c.key});
    resolver.request(req, Prio::Now, Box::new(move |res| {
        let _ = weak.upgrade_in_event_loop(move |ui| {
            if with_state(|st| st.coll_gen) != seq {
                return;
            }
            ui.set_artist_loading(false);
            let d = match res {
                Ok(d) => d,
                Err(e) => {
                    toast(&ui, &format!("Sanatçı açılamadı: {e}"));
                    return;
                }
            };
            let text = |k: &str| d.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
            let tracks: Vec<TrackInfo> = d.get("tracks").and_then(|x| x.as_array()).map(|a| a.iter().filter_map(track_from_json).collect()).unwrap_or_default();
            let shelves: Vec<Section> = d
                .get("shelves")
                .and_then(|x| x.as_array())
                .map(|a| {
                    a.iter()
                        .map(|s| Section {
                            title: s.get("title").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                            items: Vec::new(),
                            cards: cards_from_json(s.get("cards")),
                            style: s.get("style").and_then(|x| x.as_i64()).unwrap_or(2) as i32,
                        })
                        .filter(|s| !s.cards.is_empty())
                        .collect()
                })
                .unwrap_or_default();
            let name = text("title");
            if !name.is_empty() {
                ui.set_artist_name(name.into());
            }
            ui.set_artist_sub(text("sub").into());
            ui.set_artist_desc(text("desc").into());
            let banner = text("banner");
            with_state(|st| {
                for t in tracks.iter().take(12) {
                    request_art_hot(st, t.key(), &t.thumb);
                }
                for t in tracks.iter().take(2) {
                    speculate(st, t);
                }
                for s in &shelves {
                    for c in s.cards.iter().take(8) {
                        request_art_hot(st, c.art_key(), &c.thumb);
                    }
                }
                st.artist = Some((c, tracks.clone(), shelves.clone()));
            });
            // Afiş kapak kuyruğunu beklemesin: kendi iş parçacığında.
            if !banner.is_empty() {
                let key = format!("W|{}", with_state(|st| st.banner_key.clone()));
                let weak = ui.as_weak();
                thread::spawn(move || {
                    if let Ok(a) = art::load_wide(&key[2..], &banner, 1200, 500) {
                        let _ = weak.upgrade_in_event_loop(move |ui| apply_art(&ui, key, a));
                    }
                });
            }
            ui.set_artist_tracks(model_of(&tracks));
            let sh: Vec<Shelf> = shelves.iter().map(|s| Shelf { title: s.title.as_str().into(), items: model_of(&s.items), cards: cards_of(&s.cards), style: s.style }).collect();
            ui.set_artist_shelves(ModelRc::new(VecModel::from(sh)));
            // Test kancası: sanal masaüstünde tekerlek Slint'e ulaşmıyor.
            if let Some(px) = std::env::var("VYNZ_SHEET_SCROLL").ok().and_then(|v| v.parse::<f32>().ok()) {
                ui.set_scroll_request(px);
            }
        });
    }));
}

/// Basit karıştırma (xorshift; ek bağımlılık gerekmez).
fn shuffled(mut v: Vec<TrackInfo>) -> Vec<TrackInfo> {
    let mut x = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(7) | 1;
    for i in (1..v.len()).rev() {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        v.swap(i, (x % (i as u64 + 1)) as usize);
    }
    v
}

fn close_sheets(ui: &AppWindow) {
    with_state(|st| st.sheets.clear());
    ui.set_sheet("".into());
    drop_banner(ui);
}

/// Sanatçı sayfası kapanınca afişi (~2.4 MB RGBA) bellekten at.
fn drop_banner(ui: &AppWindow) {
    ui.set_has_banner(false);
    ui.set_artist_banner(Image::default());
    // Geç gelen afiş iş parçacığı kapanmış sayfaya yazmasın.
    with_state(|st| st.banner_key.clear());
    let keys: Vec<String> = BANNER_ORDER.with(|o| o.borrow_mut().drain(..).collect());
    ARTS.with(|m| {
        let mut map = m.borrow_mut();
        for k in &keys {
            map.remove(k);
        }
    });
    with_state(|st| {
        for k in keys {
            st.requested.remove(&k);
        }
    });
}

fn sheet_back(ui: &AppWindow) {
    let top = with_state(|st| {
        st.sheets.pop();
        st.coll_gen += 1; // bekleyen yanıt kapanan sayfaya yazmasın
        st.sheets.last().cloned()
    });
    ui.set_coll_loading(false);
    ui.set_artist_loading(false);
    if !with_state(|st| st.sheets.iter().any(|s| s == "artist")) {
        drop_banner(ui);
    }
    ui.set_sheet(top.unwrap_or_default().into());
}

fn open_card(ui: &AppWindow, c: Card) {
    if c.kind == "mix" {
        let list = with_state(|st| st.mix.clone());
        if !list.is_empty() {
            play_list(ui, list, 0);
        }
        return;
    }
    if c.kind == "discover" {
        let list = with_state(|st| st.discover.clone());
        if !list.is_empty() {
            play_list(ui, list, 0);
        }
        return;
    }
    if c.kind == "saved" {
        if let Some(i) = c.key.strip_prefix("saved-").and_then(|x| x.parse::<usize>().ok()) {
            open_playlist(ui, i);
            close_sheets(ui);
            ui.set_view("playlist".into());
        }
        return;
    }
    if c.kind == "artist" {
        open_artist(ui, c);
        return;
    }
    let mood = c.kind == "mood";
    ui.set_coll_loading(true);
    if mood {
        ui.set_browse_title(c.title.as_str().into());
        ui.set_browse_cards(ModelRc::new(VecModel::from(Vec::<CardUi>::new())));
    } else {
        ui.set_coll_title(c.title.as_str().into());
        ui.set_coll_sub(c.sub.as_str().into());
        ui.set_coll_kind(card_kind(&c.kind));
        ui.set_coll_tracks(ModelRc::new(VecModel::from(Vec::<Track>::new())));
    }
    open_sheet(ui, if mood { "browse" } else { "collection" });
    let seq = with_state(|st| {
        st.coll_gen += 1;
        st.coll_gen
    });
    let resolver = with_state(|st| st.resolver.clone());
    let weak = ui.as_weak();
    let req = serde_json::json!({"op": "collection", "kind": c.kind, "key": c.key});
    resolver.request(req, Prio::Now, Box::new(move |res| {
        let _ = weak.upgrade_in_event_loop(move |ui| {
            if with_state(|st| st.coll_gen) != seq {
                return; // bu arada başka bir sayfa açıldı
            }
            ui.set_coll_loading(false);
            let d = match res {
                Ok(d) => d,
                Err(e) => {
                    toast(&ui, &format!("Açılamadı: {e}"));
                    return;
                }
            };
            if mood {
                let cards = cards_from_json(d.get("cards"));
                with_state(|st| {
                    for c in cards.iter().take(40) {
                        request_art_hot(st, c.art_key(), &c.thumb);
                    }
                    st.browse = cards.clone();
                });
                ui.set_browse_cards(cards_of(&cards));
            } else {
                let tracks: Vec<TrackInfo> = d.get("tracks").and_then(|x| x.as_array()).map(|a| a.iter().filter_map(track_from_json).collect()).unwrap_or_default();
                if let Some(t) = d.get("title").and_then(|x| x.as_str()).filter(|t| !t.is_empty()) {
                    ui.set_coll_title(t.into());
                }
                with_state(|st| {
                    for t in tracks.iter().take(40) {
                        request_art_hot(st, t.key(), &t.thumb);
                    }
                    for t in tracks.iter().take(2) {
                        speculate(st, t);
                    }
                    st.coll = Some((c, tracks.clone()));
                });
                ui.set_coll_tracks(model_of(&tracks));
            }
        });
    }));
}

fn save_collection(ui: &AppWindow) {
    let saved = with_state(|st| {
        let (c, tracks) = st.coll.clone()?;
        if tracks.is_empty() {
            return None;
        }
        st.playlists.push(library::Playlist { name: ui.get_coll_title().to_string(), origin: format!("ytm:{}", c.key), tracks, image: String::new() });
        let _ = library::save(&st.playlists);
        Some(st.playlists.len())
    });
    if saved.is_some() {
        refresh_playlists(ui);
        toast(ui, &format!("“{}” Listelerim'e kaydedildi", ui.get_coll_title()));
    }
}

fn top_ui(t: &Top) -> TopResult {
    let (kind, title, sub, key) = match t {
        Top::Song(t) => (0, t.title.clone(), format!("Şarkı · {}", t.artist), t.key()),
        Top::Card(c) => (card_kind(&c.kind).min(3), c.title.clone(), c.sub.clone(), c.art_key()),
    };
    let (art, has_art) = ARTS.with(|a| a.borrow().get(&key).map(|(i, _)| (i.clone(), true))).unwrap_or_default();
    TopResult { kind, title: title.into(), sub: sub.into(), art, has_art }
}

fn top_key(t: &Top) -> String {
    match t {
        Top::Song(t) => t.key(),
        Top::Card(c) => c.art_key(),
    }
}

/// Arama: YouTube Music (sıcak yardımcı, ~0.5-1 sn) hemen gösterilir; SoundCloud arkadan eklenir.
fn search(ui: &AppWindow, query: String) {
    let (seq, resolver, sc) = with_state(|st| {
        st.search_seq += 1;
        (st.search_seq, st.resolver.clone(), provider(st, Source::SoundCloud))
    });
    ui.set_searching(true);
    ui.set_sc_searching(sc.is_some());
    ui.set_last_query(query.as_str().into());
    let weak = ui.as_weak();
    let req = serde_json::json!({"op": "search", "q": query});
    resolver.request(req, Prio::Now, Box::new(move |res| {
        let _ = weak.upgrade_in_event_loop(move |ui| {
            if with_state(|st| st.search_seq) != seq {
                return; // daha yeni bir arama var
            }
            ui.set_searching(false);
            let d = match res {
                Ok(d) => d,
                Err(e) => {
                    toast(&ui, &format!("Arama hatası: {e}"));
                    return;
                }
            };
            let songs: Vec<TrackInfo> = d.get("songs").and_then(|x| x.as_array()).map(|a| a.iter().filter_map(track_from_json).collect()).unwrap_or_default();
            let mut shelves = Vec::new();
            for (k, title, style) in [("albums", "Albümler", 2), ("artists", "Sanatçılar", 3), ("playlists", "Listeler", 2)] {
                let cards = cards_from_json(d.get(k));
                if !cards.is_empty() {
                    shelves.push(Section { title: title.into(), items: Vec::new(), cards, style });
                }
            }
            let top = d.get("top").filter(|t| !t.is_null()).and_then(|t| {
                if t.get("kind").and_then(|k| k.as_str()) == Some("song") {
                    t.get("track").and_then(track_from_json).map(Top::Song)
                } else {
                    t.get("card").and_then(|c| serde_json::from_value::<Card>(c.clone()).ok()).map(|mut c| {
                        c.thumb = sources::ytm::big_thumb(&c.thumb);
                        Top::Card(c)
                    })
                }
            });
            with_state(|st| {
                for t in songs.iter().take(20) {
                    request_art(st, t);
                }
                for s in &shelves {
                    for c in s.cards.iter().take(10) {
                        request_card_art(st, c);
                    }
                }
                match &top {
                    Some(Top::Song(t)) => {
                        request_art(st, t);
                        speculate(st, t);
                    }
                    Some(Top::Card(c)) => request_card_art(st, c),
                    None => {}
                }
                for t in songs.iter().take(2) {
                    speculate(st, t);
                }
                st.results = songs.clone();
                st.search_shelves = shelves.clone();
                st.top = top.clone();
            });
            ui.set_results(model_of(&songs));
            ui.set_has_top(top.is_some());
            if let Some(t) = &top {
                ui.set_search_top(top_ui(t));
            }
            let sh: Vec<Shelf> = shelves.iter().map(|s| Shelf { title: s.title.as_str().into(), items: model_of(&s.items), cards: cards_of(&s.cards), style: s.style }).collect();
            ui.set_search_shelves(ModelRc::new(VecModel::from(sh)));
        });
    }));
    // SoundCloud arkadan (yavaş, ~3-5 sn): gelince listeye eklenir.
    if let Some(sc) = sc {
        let weak = ui.as_weak();
        thread::spawn(move || {
            let r = sc.search(&query, 15);
            let _ = weak.upgrade_in_event_loop(move |ui| {
                if with_state(|st| st.search_seq) != seq {
                    return;
                }
                ui.set_sc_searching(false);
                if let Ok(found) = r {
                    let all = with_state(|st| {
                        for t in &found {
                            request_art(st, t);
                        }
                        st.results.extend(found);
                        st.results.clone()
                    });
                    ui.set_results(model_of(&all));
                }
            });
        });
    }
}

/// Yazarken: 380 ms duraklayınca ara.
fn search_edited(ui: &AppWindow, text: String) {
    let q = text.trim().to_string();
    let weak = ui.as_weak();
    with_state(|st| {
        if q.chars().count() < 2 {
            st.search_timer.stop();
            return;
        }
        st.search_timer.start(TimerMode::SingleShot, Duration::from_millis(380), move || {
            let Some(ui) = weak.upgrade() else { return };
            if ui.get_last_query() == q.as_str() {
                return;
            }
            if ui.get_view() != "search" {
                ui.invoke_go("search".into());
            }
            search(&ui, q.clone());
        });
    });
}

// ---------- Liste aktarma ----------

fn import_row_ui(r: &import::Row) -> ImportRow {
    let pick = r.pick();
    let key = pick.map(|t| t.key()).unwrap_or_default();
    let (art, has_art) = ARTS.with(|a| a.borrow().get(&key).map(|(i, _)| (i.clone(), true))).unwrap_or_default();
    ImportRow {
        key: key.into(),
        want_title: r.wanted.title.as_str().into(),
        want_artist: r.wanted.artist.as_str().into(),
        want_duration: r.wanted.duration,
        title: pick.map(|t| t.title.as_str()).unwrap_or("").into(),
        artist: pick.map(|t| t.artist.as_str()).unwrap_or("").into(),
        duration: pick.map(|t| t.duration).unwrap_or(0.0),
        source: pick.map(|t| t.source.index()).unwrap_or(0),
        status: r.status as i32,
        score: (r.score() * 100.0).round() as i32,
        alts: r.candidates.len() as i32,
        art,
        has_art,
    }
}

fn refresh_import(ui: &AppWindow) {
    let (rows, counts, name, note) = with_state(|st| {
        let Some(f) = st.imported.as_ref() else { return (Vec::new(), vec![0; 4], String::new(), String::new()) };
        let mut counts = vec![0; 4];
        for r in &f.rows {
            counts[r.status as usize] += 1;
        }
        let mut note = format!("{} · {} şarkı", f.source_label, f.rows.len());
        if f.truncated {
            note.push_str(" · Spotify ilk 100 şarkıyı verdi");
        }
        let rows: Vec<ImportRow> = f.rows.iter().map(import_row_ui).collect();
        let picks: Vec<TrackInfo> = f.rows.iter().filter_map(|r| r.pick().cloned()).collect();
        let name = f.name.clone();
        for t in picks.iter().take(60) {
            request_art(st, t);
        }
        (rows, counts, name, note)
    });
    ui.set_import_rows(ModelRc::new(VecModel::from(rows)));
    ui.set_import_counts(ModelRc::new(VecModel::from(counts)));
    ui.set_import_name(name.into());
    ui.set_import_note(note.into());
}

fn looks_like_file(s: &str) -> bool {
    let s = s.trim();
    let b = s.as_bytes();
    let win_drive = b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/');
    s.starts_with('/') || s.starts_with("~/") || s.starts_with("~\\") || s.starts_with("\\\\") || s.starts_with("file://") || win_drive
}

/// Sistem dosya seçicisini aç (`rfd`); seçilen yolla aktar.
fn pick_file(ui: &AppWindow) {
    let weak = ui.as_weak();
    thread::spawn(move || {
        let path = rfd::FileDialog::new()
            .set_title("Liste dosyası seç")
            .add_filter("Listeler", &["json", "csv"])
            .pick_file()
            .map(|p| p.to_string_lossy().into_owned());
        let _ = weak.upgrade_in_event_loop(move |ui| {
            if let Some(p) = path {
                start_file_import(&ui, p);
            }
        });
    });
}

fn start_file_import(ui: &AppWindow, path: String) {
    ui.set_import_busy(true);
    ui.set_import_done(0);
    ui.set_import_total(0);
    ui.set_file_lists(ModelRc::new(VecModel::from(Vec::<slint::SharedString>::new())));
    let weak = ui.as_weak();
    thread::spawn(move || {
        let res = import::read_file(&path);
        let _ = weak.upgrade_in_event_loop(move |ui| match res {
            Ok(lists) if lists.len() == 1 => {
                let l = lists.into_iter().next().unwrap();
                with_state(|st| st.import_origin = format!("dosya: {path}"));
                match_file_list(&ui, l);
            }
            Ok(lists) => {
                ui.set_import_busy(false);
                let labels: Vec<slint::SharedString> = lists.iter().map(|l| format!("{} · {} şarkı", l.name, l.tracks.len()).into()).collect();
                with_state(|st| {
                    st.import_origin = format!("dosya: {path}");
                    st.file_lists = lists;
                });
                ui.set_file_lists(ModelRc::new(VecModel::from(labels)));
            }
            Err(e) => {
                ui.set_import_busy(false);
                toast(&ui, &e);
            }
        });
    });
}

/// Dosyadan gelen listeyi eşleştir ve inceleme ekranına koy.
fn match_file_list(ui: &AppWindow, l: import::FileList) {
    let providers = with_state(|st| provider(st, Source::Ytm).zip(provider(st, Source::SoundCloud)));
    let Some((ytm, sc)) = providers else { return };
    ui.set_import_busy(true);
    ui.set_import_filter(-1);
    ui.set_file_lists(ModelRc::new(VecModel::from(Vec::<slint::SharedString>::new())));
    let weak = ui.as_weak();
    thread::spawn(move || {
        let w = Mutex::new(weak.clone());
        let f = import::match_all(l.name, "Dosya", l.tracks, false, ytm, sc, |done, total| {
            let w = w.lock().unwrap().clone();
            let _ = w.upgrade_in_event_loop(move |ui| {
                ui.set_import_done(done as i32);
                ui.set_import_total(total as i32);
            });
        });
        let _ = weak.upgrade_in_event_loop(move |ui| {
            ui.set_import_busy(false);
            with_state(|st| st.imported = Some(f));
            refresh_import(&ui);
        });
    });
}

fn start_import(ui: &AppWindow, url: String) {
    if looks_like_file(&url) {
        start_file_import(ui, url);
        return;
    }
    let providers = with_state(|st| {
        st.import_origin = url.clone();
        let ytm = st.providers.iter().find(|p| p.source() == Source::Ytm).cloned()?;
        let sc = st.providers.iter().find(|p| p.source() == Source::SoundCloud).cloned()?;
        Some((ytm, sc, st.resolver.clone()))
    });
    let Some((ytm, sc, resolver)) = providers else { return };
    ui.set_import_busy(true);
    ui.set_import_done(0);
    ui.set_import_total(0);
    ui.set_import_filter(-1);
    let weak = ui.as_weak();
    thread::spawn(move || {
        let w = Mutex::new(weak.clone());
        let res = import::run(&url, ytm, sc, &resolver, |done, total| {
            let w = w.lock().unwrap().clone();
            let _ = w.upgrade_in_event_loop(move |ui| {
                ui.set_import_done(done as i32);
                ui.set_import_total(total as i32);
            });
        });
        let _ = weak.upgrade_in_event_loop(move |ui| {
            ui.set_import_busy(false);
            match res {
                Ok(f) => {
                    dbg(&format!("aktarma: {} şarkı", f.rows.len()));
                    with_state(|st| st.imported = Some(f));
                    refresh_import(&ui);
                }
                Err(e) => toast(&ui, &e),
            }
        });
    });
}

fn import_change(ui: &AppWindow, i: usize) {
    let row = with_state(|st| {
        let r = st.imported.as_mut()?.rows.get_mut(i)?;
        if r.candidates.len() < 2 {
            return None;
        }
        r.chosen = (r.chosen + 1) % r.candidates.len();
        // Kullanıcı seçtiyse onaylanmış sayılır.
        r.status = import::Status::Matched;
        let ui_row = import_row_ui(r);
        let pick = r.pick().cloned();
        if let Some(t) = pick {
            request_art(st, &t);
        }
        Some(ui_row)
    });
    if let Some(row) = row {
        ui.get_import_rows().set_row_data(i, row);
        // sayaçları güncelle
        let counts = with_state(|st| {
            let mut c = vec![0; 4];
            for r in st.imported.as_ref().map(|f| f.rows.as_slice()).unwrap_or(&[]) {
                c[r.status as usize] += 1;
            }
            c
        });
        ui.set_import_counts(ModelRc::new(VecModel::from(counts)));
    }
}

/// Kaydetmeden önce istenmeyen satırı çıkar.
fn import_remove(ui: &AppWindow, i: usize) {
    let left = with_state(|st| {
        let f = st.imported.as_mut()?;
        let row = (i < f.rows.len()).then(|| f.rows.remove(i))?;
        Some((f.rows.len(), row))
    });
    let Some((left, row)) = left else { return };
    if left == 0 {
        // Son satır: aktarımın tamamı gider, geri alma da tamamını geri getirir.
        with_state(|st| {
            if let Some(f) = st.imported.as_mut() {
                f.rows.push(row);
            }
        });
        import_clear(ui);
        return;
    }
    let title = row.wanted.title.clone();
    toast_undo(ui, &format!("Aktarımdan çıkarıldı: {title}"), Undo::ImportRow { pos: i, row });
    let model = ui.get_import_rows();
    match model.as_any().downcast_ref::<VecModel<ImportRow>>() {
        // Yerinde sil: kaydırma konumu korunur.
        Some(vm) if i < vm.row_count() => {
            vm.remove(i);
            let counts = with_state(|st| {
                let mut c = vec![0; 4];
                for r in st.imported.as_ref().map(|f| f.rows.as_slice()).unwrap_or(&[]) {
                    c[r.status as usize] += 1;
                }
                c
            });
            ui.set_import_counts(ModelRc::new(VecModel::from(counts)));
            let note = with_state(|st| st.imported.as_ref().map(|f| format!("{} · {} şarkı", f.source_label, f.rows.len())).unwrap_or_default());
            ui.set_import_note(note.into());
        }
        _ => refresh_import(ui),
    }
}

/// Yanlış gelen aktarımı tamamen at; ekran boş aktarma formuna döner.
fn import_clear(ui: &AppWindow) {
    let old = with_state(|st| st.imported.take());
    ui.set_import_rows(ModelRc::new(VecModel::from(Vec::<ImportRow>::new())));
    ui.set_import_counts(ModelRc::new(VecModel::from(vec![0; 4])));
    ui.set_import_name("".into());
    ui.set_import_note("".into());
    ui.set_import_filter(-1);
    if let Some(f) = old {
        let msg = format!("“{}” aktarımı temizlendi", f.name);
        toast_undo(ui, &msg, Undo::Import(f));
    }
}

fn import_save(ui: &AppWindow) {
    let saved = with_state(|st| {
        let f = st.imported.take()?;
        let tracks: Vec<TrackInfo> = f
            .rows
            .iter()
            .filter(|r| r.status != import::Status::Missing)
            .filter_map(|r| r.pick().cloned())
            .collect();
        let n = tracks.len();
        st.playlists.push(library::Playlist { name: f.name.clone(), origin: st.import_origin.clone(), tracks, image: String::new() });
        if let Err(e) = library::save(&st.playlists) {
            eprintln!("kütüphane kaydedilemedi: {e}");
        }
        Some((f.name, n, st.playlists.len() - 1))
    });
    let Some((name, n, idx)) = saved else { return };
    ui.set_import_rows(ModelRc::new(VecModel::from(Vec::<ImportRow>::new())));
    refresh_playlists(ui);
    toast(ui, &format!("“{name}” kaydedildi: {n} şarkı"));
    open_playlist(ui, idx);
    ui.set_view("playlist".into());
}

// ---------- Kaydedilen listeler ----------

fn format_total_duration(tracks: &[TrackInfo]) -> String {
    let total = tracks.iter().map(|t| t.duration.max(0.0) as u64).sum::<u64>();
    if total == 0 {
        return String::new();
    }
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{h} sa {m} dk")
    } else if m > 0 {
        format!("{m} dk {s} sn")
    } else {
        format!("{s} sn")
    }
}

fn build_playlist_infos(st: &mut State) -> Vec<PlaylistInfo> {
    let mut out = Vec::with_capacity(st.playlists.len());
    let sample_tracks: Vec<TrackInfo> = st
        .playlists
        .iter()
        .flat_map(|p| p.tracks.iter().take(8).cloned())
        .collect();
    for t in &sample_tracks {
        request_art(st, t);
    }
    for p in &st.playlists {
        // Özel kapak varsa mozaik yerine o.
        if let Some((img, accent)) = playlist_cover(&p.image) {
            out.push(PlaylistInfo {
                name: p.name.as_str().into(),
                count: p.tracks.len() as i32,
                duration_text: format_total_duration(&p.tracks).into(),
                art: img,
                has_art: true,
                mosaic: false,
                custom: true,
                accent,
                ..Default::default()
            });
            continue;
        }
        // İlk 4 farklı kapak anahtarını ARTS (160px) önbelleğinden topla
        let mut tiles: Vec<(Image, Color)> = Vec::with_capacity(4);
        let mut seen_thumbs: HashSet<&str> = HashSet::new();
        for t in &p.tracks {
            if !t.thumb.is_empty() && !seen_thumbs.insert(t.thumb.as_str()) {
                continue;
            }
            if let Some(pair) = ARTS.with(|a| a.borrow().get(&t.key()).cloned()) {
                tiles.push(pair);
                if tiles.len() == 4 {
                    break;
                }
            }
        }
        let accent = tiles.first().map(|(_, c)| *c).unwrap_or(NEUTRAL);
        let mosaic = tiles.len() == 4;
        let (art, has_art) = tiles.first().map(|(i, _)| (i.clone(), true)).unwrap_or_default();
        let (art2, has_art2) = tiles.get(1).map(|(i, _)| (i.clone(), true)).unwrap_or_default();
        let (art3, has_art3) = tiles.get(2).map(|(i, _)| (i.clone(), true)).unwrap_or_default();
        let (art4, has_art4) = tiles.get(3).map(|(i, _)| (i.clone(), true)).unwrap_or_default();
        out.push(PlaylistInfo {
            name: p.name.as_str().into(),
            count: p.tracks.len() as i32,
            duration_text: format_total_duration(&p.tracks).into(),
            art,
            has_art,
            art2,
            has_art2,
            art3,
            has_art3,
            art4,
            has_art4,
            mosaic,
            custom: false,
            accent,
        });
    }
    out
}

/// Özel liste kapağı: bellekte yoksa diskten çöz (320 px PNG, ~5 ms).
fn playlist_cover(name: &str) -> Option<(Image, Color)> {
    if name.is_empty() {
        return None;
    }
    if let Some(c) = PL_COVERS.with(|m| m.borrow().get(name).cloned()) {
        return Some(c);
    }
    let a = art::load_playlist_cover(name).ok()?;
    let img = Image::from_rgba8(SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&a.rgba, a.width, a.height));
    let c = (img, Color::from_rgb_u8(a.accent.0, a.accent.1, a.accent.2));
    PL_COVERS.with(|m| m.borrow_mut().insert(name.to_string(), c.clone()));
    Some(c)
}

/// Liste için resim seç (sistem dosya seçicisi), kırp-küçült, kaydet.
fn pick_playlist_image(ui: &AppWindow, i: usize) {
    let weak = ui.as_weak();
    thread::spawn(move || {
        let Some(path) = rfd::FileDialog::new()
            .set_title("Liste kapağı seç")
            .add_filter("Resimler", &["png", "jpg", "jpeg", "webp"])
            .pick_file()
        else {
            return;
        };
        let res = art::import_playlist_cover(&path);
        let _ = weak.upgrade_in_event_loop(move |ui| match res {
            Ok(name) => {
                let old = with_state(|st| {
                    let p = st.playlists.get_mut(i)?;
                    let old = std::mem::replace(&mut p.image, name);
                    let _ = library::save(&st.playlists);
                    Some(old)
                });
                if let Some(old) = old {
                    PL_COVERS.with(|m| m.borrow_mut().remove(&old));
                    art::remove_playlist_cover(&old);
                    refresh_playlists_ui_only(&ui);
                    toast(&ui, "Liste kapağı güncellendi");
                }
            }
            Err(e) => toast(&ui, &e),
        });
    });
}

/// Özel kapağı kaldır, mozaiğe dön.
fn clear_playlist_image(ui: &AppWindow, i: usize) {
    let old = with_state(|st| {
        let p = st.playlists.get_mut(i)?;
        let old = std::mem::take(&mut p.image);
        let _ = library::save(&st.playlists);
        Some(old)
    });
    if let Some(old) = old.filter(|o| !o.is_empty()) {
        PL_COVERS.with(|m| m.borrow_mut().remove(&old));
        art::remove_playlist_cover(&old);
        refresh_playlists_ui_only(ui);
        toast(ui, "Özel kapak kaldırıldı");
    }
}

fn refresh_playlists_ui_only(ui: &AppWindow) {
    let infos = with_state(build_playlist_infos);
    ui.set_playlists(ModelRc::new(VecModel::from(infos)));
}

fn refresh_playlists(ui: &AppWindow) {
    let data = with_state(|st| (!st.home.personal.is_empty() || !st.home.external.is_empty()).then(|| st.home.clone()));
    if let Some(d) = data {
        show_home(ui, d);
    }
    refresh_playlists_ui_only(ui);
}

fn create_new_playlist(ui: &AppWindow, name: String) {
    let name = name.trim().to_string();
    if name.is_empty() {
        return;
    }
    with_state(|st| {
        st.playlists.insert(0, library::Playlist { name: name.clone(), origin: String::new(), tracks: Vec::new(), image: String::new() });
        let _ = library::save(&st.playlists);
    });
    refresh_playlists(ui);
    open_playlist(ui, 0);
    ui.invoke_go("playlist".into());
    toast(ui, &format!("'{name}' oluşturuldu"));
}

// ---------- Zevkler (ilk açılış & Ayarlar) ----------

fn refresh_taste_summary(ui: &AppWindow) {
    let summary = with_state(|st| {
        let names: Vec<&str> = recommend::GENRES
            .iter()
            .filter(|(id, _, _)| st.settings.taste_genres.iter().any(|g| g == id))
            .map(|(_, label, _)| *label)
            .collect();
        let mut parts = Vec::new();
        if !names.is_empty() {
            parts.push(names.join(", "));
        }
        if !st.settings.taste_tracks.is_empty() {
            parts.push(format!("{} örnek şarkı", st.settings.taste_tracks.len()));
        }
        if !st.settings.blocked_genres.is_empty() {
            parts.push(format!("{} engelli tür", st.settings.blocked_genres.len()));
        }
        if parts.is_empty() { "Henüz seçilmedi".into() } else { parts.join(" · ") }
    });
    ui.set_taste_summary(summary.into());
}

fn sync_taste_genres_ui(ui: &AppWindow) {
    let rows: Vec<TasteGenre> = with_state(|st| {
        recommend::GENRES
            .iter()
            .map(|(id, label, _)| TasteGenre {
                id: (*id).into(),
                label: (*label).into(),
                selected: st.taste_draft_genres.iter().any(|g| g == id),
                blocked: st.taste_draft_blocked.iter().any(|g| g == id),
            })
            .collect()
    });
    ui.set_taste_genres(ModelRc::new(VecModel::from(rows)));
}

fn sync_taste_samples_ui(ui: &AppWindow) {
    let (samples, picked) = with_state(|st| {
        let samples = st.taste_samples.clone();
        for t in &samples {
            request_art(st, t);
        }
        let picked: Vec<bool> = samples
            .iter()
            .map(|t| st.taste_draft_tracks.iter().any(|x| x.key() == t.key()))
            .collect();
        (samples, picked)
    });
    ui.set_taste_samples(model_of(&samples));
    ui.set_taste_picked(ModelRc::new(VecModel::from(picked)));
}

fn load_genre_samples(ui: &AppWindow, gid: String, show_in_ui: bool) {
    let label = recommend::GENRES.iter().find(|(id, _, _)| *id == gid).map(|(_, l, _)| *l).unwrap_or("");
    let cached = with_state(|st| st.taste_cache.get(&gid).cloned());
    if let Some(list) = cached {
        if show_in_ui {
            with_state(|st| st.taste_samples = list);
            ui.set_taste_active_genre(label.into());
            ui.set_taste_samples_loading(false);
            sync_taste_samples_ui(ui);
        }
        return;
    }
    let Some(q) = recommend::genre_query(&gid) else { return };
    let ytm = with_state(|st| provider(st, Source::Ytm));
    let Some(ytm) = ytm else { return };
    if show_in_ui {
        ui.set_taste_active_genre(label.into());
        ui.set_taste_samples_loading(true);
    }
    let weak = ui.as_weak();
    let label_s = label.to_string();
    thread::spawn(move || {
        let list = ytm.search(q, 8).unwrap_or_default();
        let _ = weak.upgrade_in_event_loop(move |ui| {
            with_state(|st| {
                st.taste_cache.insert(gid.clone(), list.clone());
                if show_in_ui && ui.get_taste_active_genre() == label_s.as_str() {
                    st.taste_samples = list;
                }
            });
            if show_in_ui && ui.get_taste_active_genre() == label_s.as_str() {
                ui.set_taste_samples_loading(false);
                sync_taste_samples_ui(&ui);
            }
        });
    });
}

fn open_tastes(ui: &AppWindow) {
    let first_selected = with_state(|st| {
        st.taste_draft_genres = st.settings.taste_genres.clone();
        st.taste_draft_blocked = st.settings.blocked_genres.clone();
        st.taste_draft_tracks = st.settings.taste_tracks.clone();
        st.taste_samples.clear();
        st.taste_draft_genres.first().cloned()
    });
    sync_taste_genres_ui(ui);
    ui.set_taste_active_genre("".into());
    ui.set_taste_samples(ModelRc::new(VecModel::from(Vec::<Track>::new())));
    ui.set_taste_picked(ModelRc::new(VecModel::from(Vec::<bool>::new())));
    ui.set_taste_samples_loading(false);
    ui.set_tastes_step(0);
    ui.set_tastes_open(true);
    if let Some(gid) = first_selected {
        load_genre_samples(ui, gid, true);
    }
}

fn taste_toggle_genre(ui: &AppWindow, gid: String) {
    let selected_now = with_state(|st| {
        if let Some(pos) = st.taste_draft_genres.iter().position(|g| g == &gid) {
            st.taste_draft_genres.remove(pos);
            false
        } else {
            st.taste_draft_blocked.retain(|g| g != &gid);
            st.taste_draft_genres.push(gid.clone());
            true
        }
    });
    sync_taste_genres_ui(ui);
    if selected_now {
        load_genre_samples(ui, gid, true);
    }
}

fn taste_block_genre(ui: &AppWindow, gid: String) {
    let blocked_now = with_state(|st| {
        if let Some(pos) = st.taste_draft_blocked.iter().position(|g| g == &gid) {
            st.taste_draft_blocked.remove(pos);
            false
        } else {
            st.taste_draft_genres.retain(|g| g != &gid);
            st.taste_draft_blocked.push(gid.clone());
            true
        }
    });
    sync_taste_genres_ui(ui);
    if blocked_now {
        // Engellenen türün örnek sanatçılarını arka planda önbellekle (ana sayfa filtresi için)
        load_genre_samples(ui, gid, false);
    }
}

fn taste_toggle_sample(ui: &AppWindow, idx: usize) {
    with_state(|st| {
        let Some(t) = st.taste_samples.get(idx).cloned() else { return };
        if let Some(pos) = st.taste_draft_tracks.iter().position(|x| x.key() == t.key()) {
            st.taste_draft_tracks.remove(pos);
        } else {
            st.taste_draft_tracks.push(t);
        }
    });
    sync_taste_samples_ui(ui);
}

fn taste_preview_sample(ui: &AppWindow, idx: usize) {
    if let Some(t) = with_state(|st| st.taste_samples.get(idx).cloned()) {
        play_list(ui, vec![t], 0);
    }
}

fn taste_save(ui: &AppWindow) {
    with_state(|st| {
        st.settings.taste_genres = st.taste_draft_genres.clone();
        st.settings.blocked_genres = st.taste_draft_blocked.clone();
        st.settings.taste_tracks = st.taste_draft_tracks.clone();
        st.settings.tastes_done = true;
        let _ = st.settings.save();
    });
    ui.set_tastes_step(1);
    refresh_taste_summary(ui);
    toast(ui, "Zevkler kaydedildi; ana sayfa yenileniyor…");
    load_home(ui);
}

/// Algoritmayı sıfırla. Beğeniler kütüphane sayıldığı için her iki durumda da korunur.
fn reset_recs(ui: &AppWindow, with_history: bool) {
    let undo = with_state(|st| Undo::Reset {
        boosts: st.history.boosted_tracks.clone(),
        disliked_tracks: st.history.disliked_tracks.clone(),
        disliked_artists: st.history.disliked_artists.clone(),
        listens: with_history.then(|| st.history.listens.clone()),
        taste_genres: st.settings.taste_genres.clone(),
        blocked_genres: st.settings.blocked_genres.clone(),
        taste_tracks: st.settings.taste_tracks.clone(),
        tastes_done: st.settings.tastes_done,
    });
    with_state(|st| {
        st.history.reset_profile();
        if with_history {
            st.history.clear_listens();
        }
        st.settings.taste_genres.clear();
        st.settings.blocked_genres.clear();
        st.settings.taste_tracks.clear();
        st.settings.tastes_done = false;
        let _ = st.settings.save();
    });
    let _ = std::fs::remove_file(home_cache());
    refresh_hidden(ui);
    refresh_taste_summary(ui);
    if with_history {
        refresh_profile(ui);
    }
    toast_undo(ui, if with_history { "Geçmiş ve öneri profili sıfırlandı" } else { "Öneri profili sıfırlandı" }, undo);
    load_home(ui);
    // Boş kalmasın: yeni zevkleri seçmeye yönlendir (vitrin yedek olarak arkada dolar).
    open_tastes(ui);
}

fn taste_skip(ui: &AppWindow) {
    with_state(|st| {
        if !st.settings.tastes_done {
            st.settings.tastes_done = true;
            let _ = st.settings.save();
        }
    });
    ui.set_tastes_step(1);
}

fn sync_playlist_view(ui: &AppWindow) {
    let tracks = with_state(|st| {
        let Some(pi) = st.current_pl else { return Vec::new() };
        let Some(pl) = st.playlists.get(pi) else { return Vec::new() };
        let q = st.pl_filter.trim().to_lowercase();
        let mut vis = Vec::new();
        let mut out = Vec::new();
        for (idx, t) in pl.tracks.iter().enumerate() {
            if q.is_empty()
                || t.title.to_lowercase().contains(&q)
                || t.artist.to_lowercase().contains(&q)
                || t.album.to_lowercase().contains(&q)
            {
                vis.push(idx);
                out.push(t.clone());
            }
        }
        st.pl_visible = vis;
        out
    });
    ui.set_playlist_tracks(model_of(&tracks));
}

fn open_playlist(ui: &AppWindow, i: usize) {
    let ok = with_state(|st| {
        let p = st.playlists.get(i)?.tracks.clone();
        st.current_pl = Some(i);
        st.pl_filter.clear();
        for t in p.iter().take(40) {
            request_art(st, t);
        }
        for t in p.iter().take(2) {
            speculate(st, t);
        }
        Some(())
    });
    if ok.is_none() {
        return;
    }
    ui.set_current_playlist(i as i32);
    ui.set_pl_filter("".into());
    ui.set_pl_rename_open(false);
    sync_playlist_view(ui);
}

fn rename_playlist(ui: &AppWindow, i: usize, name: String) {
    let name = name.trim().to_string();
    if name.is_empty() {
        return;
    }
    let ok = with_state(|st| {
        let pl = st.playlists.get_mut(i)?;
        pl.name = name.clone();
        let _ = library::save(&st.playlists);
        Some(())
    });
    if ok.is_some() {
        refresh_playlists(ui);
        toast(ui, &format!("Liste adı güncellendi: {name}"));
    }
}

fn move_playlist_track(ui: &AppWindow, vis_i: usize, delta: i32) {
    let ok = with_state(|st| {
        if !st.pl_filter.trim().is_empty() {
            return false;
        }
        let Some(pi) = st.current_pl else { return false };
        let Some(pl) = st.playlists.get_mut(pi) else { return false };
        let j = vis_i as i64 + delta as i64;
        if j < 0 || j as usize >= pl.tracks.len() || vis_i >= pl.tracks.len() {
            return false;
        }
        pl.tracks.swap(vis_i, j as usize);
        let _ = library::save(&st.playlists);
        true
    });
    if ok {
        sync_playlist_view(ui);
        refresh_playlists_ui_only(ui);
    }
}

fn remove_from_playlist(ui: &AppWindow, vis_i: usize) {
    let ok = with_state(|st| {
        let pi = st.current_pl?;
        let real_i = st.pl_visible.get(vis_i).copied().unwrap_or(vis_i);
        let pl = st.playlists.get_mut(pi)?;
        if real_i >= pl.tracks.len() {
            return None;
        }
        let t = pl.tracks.remove(real_i);
        let _ = library::save(&st.playlists);
        Some(Undo::PlaylistTrack { pl: pi, pos: real_i, t })
    });
    if let Some(u) = ok {
        sync_playlist_view(ui);
        refresh_playlists_ui_only(ui);
        toast_undo(ui, "Listeden çıkarıldı", u);
    }
}

fn filter_playlist(ui: &AppWindow, q: String) {
    with_state(|st| st.pl_filter = q);
    sync_playlist_view(ui);
}

fn shuffle_playlist(ui: &AppWindow) {
    let list = with_state(|st| st.current_pl.and_then(|i| st.playlists.get(i)).map(|p| p.tracks.clone())).unwrap_or_default();
    if !list.is_empty() {
        play_list(ui, shuffled(list), 0);
    }
}

fn delete_playlist(ui: &AppWindow, i: usize) {
    let removed = with_state(|st| {
        if i >= st.playlists.len() {
            return None;
        }
        let pl = st.playlists.remove(i);
        // Açık olan başka bir listeyse indeksini kaydır, silinen açıksa kapat.
        st.current_pl = match st.current_pl {
            Some(c) if c == i => None,
            Some(c) if c > i => Some(c - 1),
            c => c,
        };
        if let Err(e) = library::save(&st.playlists) {
            eprintln!("kütüphane kaydedilemedi: {e}");
        }
        Some(pl)
    });
    ui.set_current_playlist(with_state(|st| st.current_pl).map(|c| c as i32).unwrap_or(-1));
    refresh_playlists(ui);
    if let Some(pl) = removed {
        let msg = format!("“{}” silindi", pl.name);
        toast_undo(ui, &msg, Undo::Playlist { idx: i, pl });
    }
}

// ---------- Profil ve canlı spektrum ----------

fn refresh_profile(ui: &AppWindow) {
    let (s, tracks) = with_state(|st| {
        let s = stats::compute(&st.history);
        let tracks: Vec<TrackInfo> = s.top_tracks.iter().map(|t| t.0.clone()).collect();
        for t in &tracks {
            request_art(st, t);
        }
        st.prof_tracks = tracks.clone();
        (s, tracks)
    });
    ui.set_prof_total(stats::human(s.total_secs).into());
    ui.set_prof_week(stats::human(s.week_secs).into());
    ui.set_prof_tracks_count(s.tracks as i32);
    ui.set_prof_artists_count(s.artists as i32);
    ui.set_prof_likes(s.likes as i32);
    let max_a = s.top_artists.first().map(|a| a.1).unwrap_or(1.0).max(1.0);
    let artists: Vec<ArtistStat> = s
        .top_artists
        .iter()
        .map(|(n, secs)| ArtistStat { name: n.as_str().into(), label: stats::human(*secs).into(), frac: secs / max_a })
        .collect();
    ui.set_prof_top_artists(ModelRc::new(VecModel::from(artists)));
    ui.set_prof_top_tracks(model_of(&tracks));
    let max_h = s.hours.iter().cloned().fold(0.0f32, f32::max).max(1.0);
    ui.set_prof_hours(ModelRc::new(VecModel::from(s.hours.iter().map(|h| h / max_h).collect::<Vec<f32>>())));
    let peak = s.hours.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map(|(h, _)| h).unwrap_or(0);
    ui.set_prof_peak(if s.total_secs > 0.0 {
        format!("En çok {:02}:00 civarında dinliyorsun.", peak).into()
    } else {
        "Dinledikçe burası dolacak.".into()
    });
}

/// Spektrumu yalnız gerekince çalıştır: profil ekranı açıkken ya da "müziğe tepki" açık ve
/// zemin görünürken (ana sayfa / söz ekranı; Sade stilde kapalı), her ikisinde de yalnız çalarken.
fn update_viz(ui: &AppWindow) {
    let zemin = ui.get_bg_reactive()
        && ui.get_bg_style() != 2
        && !ui.get_window_hidden()
        && (ui.get_lyrics_open() || ui.get_view() == "home");
    let want = ui.get_playing() && (ui.get_view() == "profile" || zemin);
    let running = with_state(|st| st.viz.is_some());
    if want == running {
        return;
    }
    if !want {
        with_state(|st| st.viz = None); // Drop cava'yı kapatır
        ui.set_viz_on(false);
        ui.set_beat(0.0);
        ui.set_spectrum(ModelRc::new(VecModel::from(vec![0.0f32; viz::BARS])));
        return;
    }
    ui.set_spectrum(ModelRc::new(VecModel::from(vec![0.0f32; viz::BARS])));
    let weak = ui.as_weak();
    struct BeatState {
        m: f32,
        v: f32,
        beat: f32,
    }
    let beat_st = Mutex::new(BeatState { m: 0.0, v: 0.0, beat: 0.0 });
    match viz::Viz::start(move |frame| {
        // Uyarlamalı vuruş: 0..3 bas (1.0) + 4..7 vuruş/trampet (0.6), ~2 sn kayan ortalama/sapma
        let mut sum = 0.0f32;
        let mut wsum = 0.0f32;
        for (i, &x) in frame.iter().take(8).enumerate() {
            let w = if i < 4 { 1.0 } else { 0.6 };
            sum += x * w;
            wsum += w;
        }
        let e = if wsum > 0.0 { sum / wsum } else { 0.0 };
        let beat = {
            let mut st = beat_st.lock().unwrap();
            const ALPHA: f32 = 0.03;
            let diff = e - st.m;
            let onset = (diff / (2.0 * st.v.sqrt() + 0.02)).clamp(0.0, 1.0);
            st.m += ALPHA * diff;
            st.v = ((1.0 - ALPHA) * st.v + ALPHA * diff * diff).max(0.0);
            st.beat = if onset > st.beat { onset } else { st.beat * 0.85 };
            st.beat
        };
        let _ = weak.upgrade_in_event_loop(move |ui| {
            if (beat - ui.get_beat()).abs() > 0.01 {
                ui.set_beat(beat);
            }
            if ui.get_view() == "profile" {
                let m = ui.get_spectrum();
                for (i, v) in frame.into_iter().enumerate() {
                    m.set_row_data(i, v);
                }
            }
        });
    }) {
        Ok(v) => {
            with_state(|st| st.viz = Some(v));
            ui.set_viz_on(true);
        }
        Err(e) => eprintln!("spektrum: {e}"),
    }
}

/// Ortam zemini zamanlayıcısı (33 ms): yalnız gerekince çalışır.
/// - İlk açılış/Ayarlar "Görünümünü seç" adımı görünürken (önizlemeler canlı)
/// - Veya Sade (2) dışındaki bir stilde, hareket açıkken, çalarken ve ana sayfa/söz ekranı görünürken
/// Pencere gizliyse ya da oyun modundaysa hiç çalışmaz.
fn update_amb_timer(ui: &AppWindow) {
    let preview_step = ui.get_tastes_open() && ui.get_tastes_step() == 1;
    let lyrics_motion_ok = !ui.get_lyrics_open() || ui.get_lyrics_motion();
    let bg_active = ui.get_bg_style() != 2
        && ui.get_bg_motion()
        && lyrics_motion_ok
        && ui.get_playing()
        && (ui.get_lyrics_open() || ui.get_view() == "home");
    let want = !ui.get_window_hidden() && !ui.get_game_mode() && (preview_step || bg_active);
    let weak = ui.as_weak();
    with_state(|st| {
        if want == st.amb_running {
            return;
        }
        st.amb_running = want;
        if !want {
            st.amb_timer.stop();
            return;
        }
        st.amb_timer.start(TimerMode::Repeated, Duration::from_millis(33), move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_amb_t(ui.get_amb_t() + 0.033);
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
    ui.set_normalize_on(s.normalize);
    ui.set_discord_on(s.discord_rpc);
    ui.set_close_to_tray(s.close_to_tray);
    ui.set_readahead_index(s.readahead_index());
    ui.set_cookies_index(s.cookies_index());
    ui.set_download_index(s.download_index());
    ui.set_autoradio_on(s.autoradio);
    ui.set_lyrics_style(s.lyrics_style);
    ui.set_lyrics_size(s.lyrics_size);
    ui.set_lyrics_align(s.lyrics_align);
    ui.set_lyrics_spacing(s.lyrics_spacing.clamp(0, 2));
    ui.set_lyrics_motion(s.lyrics_motion);
    ui.set_lyrics_translate(s.lyrics_translate);
    ui.set_keep_index(s.keep_index());
    ui.set_crossfade_index(s.crossfade_index());
    ui.set_eq_on(s.eq_on);
    ui.set_eq_preset(s.eq_preset);
    ui.set_eq_gains(ModelRc::new(VecModel::from(s.eq_gains.to_vec())));
    ui.set_theme_accent(s.theme_accent);
    ui.set_theme_backdrop(s.theme_backdrop);
    ui.set_auto_update(s.auto_update);
    ui.set_bg_style(s.bg_style.clamp(0, 4));
    ui.set_bg_level(s.bg_level.clamp(0, 2));
    ui.set_bg_motion(s.bg_motion);
    ui.set_bg_reactive(s.bg_reactive && cfg!(target_os = "linux"));
    ui.set_bg_grain(s.bg_grain);
    apply_theme(ui, s);
}

/// Ekolayzır değişti: arayüz, çalar ve ayar dosyası.
fn eq_changed(ui: &AppWindow, gains: [f32; 10]) {
    let (on, preset) = with_state(|st| (st.settings.eq_on, st.settings.eq_preset));
    ui.set_eq_on(on);
    ui.set_eq_preset(preset);
    ui.set_eq_gains(ModelRc::new(VecModel::from(gains.to_vec())));
    apply_settings(ui);
}

fn rgb(c: u32) -> Color {
    Color::from_rgb_u8((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

thread_local! {
    /// Kapaktan türetilen son vurgu (sabit vurgudan otomatiğe dönünce kullanılır)
    static COVER_ACCENT: std::cell::Cell<Option<Color>> = const { std::cell::Cell::new(None) };
}

/// Tema: sabit vurgu (0 = kapaktan) ve zemin renkleri.
fn apply_theme(ui: &AppWindow, s: &Settings) {
    let th = ui.global::<Theme>();
    let bd = settings::BACKDROPS[s.theme_backdrop.clamp(0, settings::BACKDROPS.len() as i32 - 1) as usize];
    th.set_canvas(rgb(bd[0]));
    th.set_outer(rgb(bd[1]));
    th.set_floating(rgb(bd[2]));
    th.set_drawer(rgb(bd[3]));
    let a = settings::ACCENTS[s.theme_accent.clamp(0, settings::ACCENTS.len() as i32 - 1) as usize];
    if a != 0 {
        th.set_accent(rgb(a));
    } else if let Some(c) = COVER_ACCENT.with(|x| x.get()) {
        th.set_accent(c);
    }
    refresh_amb_discs(ui);
}

/// Kapaktan gelen vurgu: sabit vurgu seçiliyse yalnız saklanır.
fn set_cover_accent(ui: &AppWindow, c: Color) {
    COVER_ACCENT.with(|x| x.set(Some(c)));
    if ui.get_theme_accent() == 0 {
        ui.global::<Theme>().set_accent(c);
    }
    refresh_amb_discs(ui);
}

fn apply_settings(ui: &AppWindow) {
    let prev = with_state(|st| st.settings.clone());
    let s = Settings {
        cache_gb: settings::CACHE_GB[ui.get_cache_index().clamp(0, 4) as usize],
        prefetch: ui.get_prefetch_on(),
        normalize: ui.get_normalize_on(),
        discord_rpc: ui.get_discord_on(),
        close_to_tray: ui.get_close_to_tray(),
        readahead_secs: settings::READAHEAD_SECS[ui.get_readahead_index().clamp(0, 3) as usize],
        cookies_browser: settings::COOKIE_BROWSERS[ui.get_cookies_index().clamp(0, 1) as usize].map(str::to_string),
        download_kbps: settings::DOWNLOAD_KBPS[ui.get_download_index().clamp(0, 3) as usize],
        autoradio: ui.get_autoradio_on(),
        lyrics_style: ui.get_lyrics_style(),
        lyrics_size: ui.get_lyrics_size(),
        lyrics_align: ui.get_lyrics_align(),
        lyrics_spacing: ui.get_lyrics_spacing(),
        lyrics_motion: ui.get_lyrics_motion(),
        lyrics_translate: ui.get_lyrics_translate(),
        keep_days: settings::KEEP_DAYS[ui.get_keep_index().clamp(0, 3) as usize],
        volume: ui.get_volume(),
        crossfade_secs: settings::CROSSFADE_SECS[ui.get_crossfade_index().clamp(0, 4) as usize],
        eq_on: ui.get_eq_on(),
        theme_accent: ui.get_theme_accent(),
        theme_backdrop: ui.get_theme_backdrop(),
        auto_update: ui.get_auto_update(),
        bg_style: ui.get_bg_style().clamp(0, 4),
        bg_level: ui.get_bg_level(),
        bg_motion: ui.get_bg_motion(),
        bg_reactive: ui.get_bg_reactive(),
        bg_grain: ui.get_bg_grain(),
        ..prev
    };
    with_state(|st| {
        st.player.send(Cmd::Readahead(s.readahead_secs));
        st.player.send(Cmd::Normalize(s.normalize));
        st.player.send(Cmd::Eq(s.eq_on.then_some(s.eq_gains)));
        st.player.send(Cmd::Crossfade(s.crossfade_secs as f64));
        discord::set_enabled(s.discord_rpc);
        st.resolver.set_cookies(s.cookies_browser.clone());
        if !s.prefetch && st.appended.is_some() {
            st.player.send(Cmd::ClearNext);
            st.appended = None;
        }
        let limit = s.cache_bytes();
        let keep = s.keep_days;
        st.fetcher.set_limits(s.download_kbps as u64 * 1024, limit);
        thread::spawn(move || {
            cache::expire(keep);
            cache::enforce(limit);
        });
        if let Err(e) = s.save() {
            eprintln!("ayarlar kaydedilemedi: {e}");
        }
        st.settings = s;
    });
    let s = with_state(|st| st.settings.clone());
    apply_theme(ui, &s);
    update_viz(ui);
    update_amb_timer(ui);
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

/// Konuma atla: arayüz hedefi hemen gösterir, mpv'den gelen eski konumlar kısa süre yok sayılır.
fn seek_to(ui: &AppWindow, secs: f64) {
    let secs = secs.max(0.0);
    ui.set_position(secs as f32);
    with_state(|st| {
        st.seek_guard = Some((secs as f32, std::time::Instant::now()));
        st.player.send(Cmd::Seek(secs));
    });
    update_lyrics(ui, secs as f32);
    save_session(ui);
}

fn session_path() -> std::path::PathBuf {
    paths::data_dir().join("session.json")
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Session {
    queue: Vec<TrackInfo>,
    index: usize,
    position: f64,
}

/// Kuyruk, çalan parça ve konumu kaydet (kapanışta ve çalarken arada bir).
fn save_session(ui: &AppWindow) {
    let s = with_state(|st| {
        st.last_session_save = std::time::Instant::now();
        let i = st.index?;
        if st.queue.is_empty() {
            return None;
        }
        // Dinlenmeye başlanmadan kapatıldıysa geri yüklenen konumu koru
        let pos = st.resume_pos.unwrap_or(ui.get_position() as f64);
        Some(Session { queue: st.queue.clone(), index: i, position: pos })
    });
    let Some(s) = s else { return };
    let p = session_path();
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let tmp = p.with_extension("json.tmp");
    if std::fs::write(&tmp, serde_json::to_vec(&s).unwrap_or_default()).is_ok() {
        let _ = std::fs::rename(tmp, p);
    }
}

/// Açılışta son oturumu geri getir: parça duraklatılmış hazır bekler.
fn restore_session(ui: &AppWindow) {
    let Some(s) = std::fs::read(session_path()).ok().and_then(|r| serde_json::from_slice::<Session>(&r).ok()) else { return };
    let Some(t) = s.queue.get(s.index).cloned() else { return };
    let model = model_of(&s.queue);
    with_state(|st| {
        for x in s.queue.iter().take(30) {
            request_art(st, x);
        }
        st.queue = s.queue.clone();
        st.index = Some(s.index);
        st.loaded = false;
        st.resume_pos = Some(s.position);
        st.pending_start = s.position;
    });
    ui.set_queue(model);
    set_now_ui(ui, &t, s.index);
    with_state(|st| {
        st.listen = None; // geri yükleme dinleme sayılmasın
        st.load_at = None; // bekçi tetiklenmesin
        st.mpris.send(mpris::Msg::Playing(false));
        st.mpris.send(mpris::Msg::Position(s.position));
    });
    ui.set_playing(false);
    ui.set_position(s.position as f32);
    dbg(&format!("oturum geri yüklendi: {} @ {:.0} sn", t.key(), s.position));
}

/// MPRIS (medya tuşları, çubuk) komutlarını uygula.
// ---------- Güncelleme ----------

/// Son sürümü denetle; yenisi varsa (atlanmamışsa) hemen indirip kurmaya başla.
/// `manual`: Ayarlar'dan istendi (sonuç ne olursa bildir, atlanan sürümü de göster).
fn check_update(ui: &AppWindow, manual: bool) {
    if updater::is_dev_build() {
        if manual {
            toast(ui, "Geliştirici derlemesi: kendini güncellemez (cargo build ile güncelle)");
        }
        return;
    }
    if manual {
        ui.set_update_checking(true);
    }
    let skip = with_state(|st| st.settings.skip_version.clone());
    let weak = ui.as_weak();
    thread::spawn(move || {
        let res = updater::check();
        let _ = weak.upgrade_in_event_loop(move |ui| {
            ui.set_update_checking(false);
            match res {
                Ok(Some(u)) if manual || u.version != skip => start_update(&ui, u),
                Ok(_) => {
                    if manual {
                        toast(&ui, &format!("Vynz güncel ({})", updater::current()));
                    }
                }
                Err(e) => {
                    eprintln!("güncelleme denetlenemedi: {e}");
                    if manual {
                        toast(&ui, "Güncelleme denetlenemedi (bağlantı?)");
                    }
                }
            }
        });
    });
}

fn start_update(ui: &AppWindow, u: updater::Update) {
    if updater::BUSY.swap(true, Ordering::SeqCst) {
        return; // önceki kurulum (iptal edilmiş olsa da) henüz bitmedi
    }
    ui.set_update_version(u.version.as_str().into());
    ui.set_update_notes(u.notes.as_str().into());
    ui.set_update_progress(0.0);
    ui.set_update_error("".into());
    ui.set_update_state(1);
    updater::CANCEL.store(false, Ordering::Relaxed);
    with_state(|st| st.pending_update = Some(u.clone()));
    let weak = ui.as_weak();
    thread::spawn(move || {
        let w = weak.clone();
        let res = updater::install(&u, &updater::CANCEL, move |p| {
            let _ = w.upgrade_in_event_loop(move |ui| ui.set_update_progress(p));
        });
        updater::BUSY.store(false, Ordering::SeqCst);
        let _ = weak.upgrade_in_event_loop(move |ui| match res {
            // Kurulum bitti ama kullanıcı bu arada vazgeçtiyse yeniden başlatma; yeni sürüm sonraki açılışta.
            Ok(()) if updater::CANCEL.load(Ordering::Relaxed) => {
                ui.set_update_state(0);
                toast(&ui, "Güncelleme kuruldu; bir sonraki açılışta geçerli olur");
            }
            Ok(()) => {
                ui.set_update_state(2);
                // Kısa bir an "yeniden başlatılıyor" görünsün, sonra kapan ve yenisini aç.
                let weak = ui.as_weak();
                Timer::single_shot(Duration::from_millis(1200), move || {
                    if weak.upgrade().is_some() {
                        updater::RESTART.store(true, Ordering::Relaxed);
                        let _ = slint::quit_event_loop();
                    }
                });
            }
            Err(e) if updater::CANCEL.load(Ordering::Relaxed) => {
                let _ = e;
                ui.set_update_state(0);
            }
            Err(e) => {
                eprintln!("güncelleme kurulamadı: {e}");
                ui.set_update_error(e.into());
                ui.set_update_state(3);
            }
        });
    });
}

fn on_mpris(ui: &AppWindow, a: mpris::Act) {
    use mpris::Act;
    match a {
        Act::PlayPause => ui.invoke_toggle_play(),
        Act::Play | Act::Pause => {
            let want_play = matches!(a, Act::Play);
            if ui.get_has_now() && want_play != ui.get_playing() {
                ui.invoke_toggle_play();
            }
        }
        Act::Next => ui.invoke_next(),
        Act::Prev => ui.invoke_prev(),
        Act::Seek(off) => seek_to(ui, ui.get_position() as f64 + off),
        Act::SetPosition(p) => seek_to(ui, p),
        Act::Volume(v) => {
            let v = v.clamp(0.0, 1.0);
            ui.set_volume(v as f32);
            with_state(|st| st.player.send(Cmd::Volume(v)));
        }
        Act::Shuffle(on) => {
            if on != ui.get_shuffle_on() {
                toggle_shuffle(ui);
            }
        }
        Act::Loop(mode) => {
            for _ in 0..3 {
                if ui.get_repeat_mode() == mode {
                    break;
                }
                cycle_repeat(ui);
            }
        }
    }
}

/// Pencereyi sistem tepsisine gizle ve düşük RAM (oyun) moduna geç.
#[allow(dead_code)]
fn hide_to_tray(ui: &AppWindow) {
    release_ram(ui);
    use slint::winit_030::WinitWindowAccessor;
    ui.window().with_winit_window(|w| w.set_visible(false));
}

/// Düşük kaynak: animasyonları durdur, çalan dışındaki kapakları bellekten at, spektrumu kapat.
/// Tepsiye inerken ve oyun modunda kullanılır.
fn release_ram(ui: &AppWindow) {
    ui.set_window_hidden(true);
    ui.global::<Motion>().set_paused(true);
    let now_key = if ui.get_has_now() { Some(ui.get_now().key.to_string()) } else { None };
    ARTS.with(|m| m.borrow_mut().retain(|k, _| now_key.as_deref() == Some(k.as_str())));
    ART_ORDER.with(|q| q.borrow_mut().retain(|k| now_key.as_deref() == Some(k.as_str())));
    BANNER_ORDER.with(|q| q.borrow_mut().clear());
    BLURS.with(|m| m.borrow_mut().clear());
    BLUR2S.with(|m| m.borrow_mut().clear());
    PALETTES.with(|m| m.borrow_mut().clear());
    BLUR_ORDER.with(|q| q.borrow_mut().clear());
    BLUR2_ORDER.with(|q| q.borrow_mut().clear());
    BIGS.with(|m| m.borrow_mut().clear());
    BIG_ORDER.with(|q| q.borrow_mut().clear());
    with_state(|st| {
        st.requested.clear();
        if let Some(k) = &now_key {
            st.requested.insert(k.clone());
        }
        st.viz = None;
    });
    ui.set_viz_on(false);
    #[cfg(target_os = "windows")]
    unsafe {
        use windows::Win32::System::Threading::{GetCurrentProcess, SetProcessWorkingSetSize};
        let _ = SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
    }
}

/// Tepsiden geri dön: pencereyi göster ve kapakları disk önbelleğinden hızla doldur.
#[allow(dead_code)]
fn show_from_tray(ui: &AppWindow) {
    if ui.get_game_mode() {
        // oyun modundayken tepsiden açılırsa oyun sayfası görünür kalsın
        use slint::winit_030::WinitWindowAccessor;
        let _ = ui.show();
        ui.window().with_winit_window(|w| {
            w.set_visible(true);
            w.set_minimized(false);
            w.focus_window();
        });
        return;
    }
    ui.set_window_hidden(false);
    ui.global::<Motion>().set_paused(false);
    use slint::winit_030::WinitWindowAccessor;
    let _ = ui.show();
    ui.window().with_winit_window(|w| {
        w.set_visible(true);
        w.set_minimized(false);
        w.focus_window();
    });
    reload_art(ui);
}

/// Bellekten atılan kapakları disk önbelleğinden yeniden iste (ana sayfa rafları + çalan).
fn reload_art(ui: &AppWindow) {
    if ui.get_has_now() {
        let key = ui.get_now().key.to_string();
        with_state(|st| {
            if let Some(t) = st.index.and_then(|i| st.queue.get(i)).filter(|t| t.key() == key).cloned() {
                // küçük kapak bulanık zemin + paleti getirir; büyük kapak geri gelince küçüğün yerini alır
                st.requested.remove(&key);
                request_art(st, &t);
                request_big(st, &t);
            }
        });
    }
    with_state(|st| {
        let shelves = st.shelves.clone();
        for s in &shelves {
            for t in s.items.iter().take(if s.style == 1 || s.style == 6 { 12 } else { 8 }) {
                request_art(st, t);
            }
            for c in s.cards.iter().take(10) {
                request_card_art(st, c);
            }
        }
    });
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Güncellemeden sonra yeniden açıldıysak eski süreç kapanana kadar bekle (MPRIS adı vb.).
    updater::wait_for_parent();
    updater::cleanup_old();
    let ui = AppWindow::new()?;
    // Wayland app_id. Needs the platform (so after AppWindow::new) but must come before
    // the native window is created on first show. Without it the window has an empty
    // class and docks cannot match it to vynz.desktop.
    let _ = slint::set_xdg_app_id("vynz");
    ui.set_app_version(updater::current().into());
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
    ui.set_volume(settings.volume.clamp(0.0, 1.0));
    player.send(Cmd::Volume(ui.get_volume() as f64));

    let metered = Arc::new(AtomicBool::new(false));
    let on_battery = Arc::new(AtomicBool::new(false));
    start_env_monitor(metered.clone(), on_battery.clone());
    let resolver = Resolver::new(settings.cookies_browser.clone());
    runtime::maybe_update_background();
    discord::start(settings.discord_rpc);
    let mpris_server = {
        let weak = weak.clone();
        mpris::Mpris::start(move |a| {
            let _ = weak.upgrade_in_event_loop(move |ui| on_mpris(&ui, a));
        })
    };
    mpris_server.send(mpris::Msg::Volume(ui.get_volume() as f64));
    let fetcher = fetcher::Fetcher::new(settings.download_kbps as u64 * 1024, settings.cache_bytes(), metered.clone());
    {
        // Açılışta ve sonra günde bir: süresi dolan şarkılar, eski kapaklar.
        let limit = settings.cache_bytes();
        let keep = settings.keep_days;
        thread::spawn(move || loop {
            cache::expire(keep);
            cache::enforce(limit);
            art::prune(30, 150 * 1024 * 1024);
            thread::sleep(Duration::from_secs(24 * 3600));
        });
    }

    let downloader = {
        let w = weak.clone();
        downloads::Downloader::new(move |ev| {
            let _ = w.upgrade_in_event_loop(move |ui| on_download_event(&ui, ev));
        })
    };
    let pool = start_art_pool(weak.clone());
    STATE.with(|s| {
        *s.borrow_mut() = Some(State {
            providers: vec![Arc::new(Ytm::new(resolver.clone())), Arc::new(SoundCloud::new(resolver.clone()))],
            player,
            resolver: resolver.clone(),
            settings,
            art_tx: pool.0,
            art_hot: pool.1,
            shelves: Vec::new(),
            results: Vec::new(),
            queue: Vec::new(),
            index: None,
            appended: None,
            fetcher: fetcher.clone(),
            dl_list: downloads::load(),
            dl_pending: Vec::new(),
            downloader: downloader.clone(),
            cache_queued: 0,
            play_seq: 0,
            loaded: false,
            fallback_tried: false,
            search_seq: 0,
            requested: HashSet::new(),
            toast_timer: Timer::default(),
            undo: None,
            hover_timer: Timer::default(),
            metered,
            on_battery,
            imported: None,
            import_origin: String::new(),
            playlists: library::load(),
            current_pl: None,
            pl_visible: Vec::new(),
            pl_filter: String::new(),
            lyrics: HashMap::new(),
            lyr_pending: HashSet::new(),
            lyr_line: None,
            lyr_spoken: 0,
            lyr_seg: None,
            lyr_hi: 0.0,
            lyr_force: false,
            lyr_offsets: load_offsets(),
            history: history::History::load(),
            listen: None,
            shuffle_orig: None,
            repeat: 0,
            radio_busy: false,
            menu: None,
            artist_pick: (Vec::new(), String::new()),
            pending_update: None,

            mix: Vec::new(),
            discover: Vec::new(),
            coll: None,
            artist: None,
            banner_key: String::new(),
            browse: Vec::new(),
            home: HomeData::default(),
            viz: None,
            prof_tracks: Vec::new(),
            file_lists: Vec::new(),
            mpris: mpris_server.clone(),
            seek_pending: false,
            last_pos: (0.0, std::time::Instant::now()),
            nav: vec!["home".into()],
            sheets: Vec::new(),
            coll_gen: 0,
            nav_skip: false,
            top: None,
            search_shelves: Vec::new(),
            search_timer: Timer::default(),
            translations: HashMap::new(),
            tr_pending: HashSet::new(),
            load_at: None,
            progressed: false,
            watchdog: Timer::default(),
            seek_guard: None,
            pending_start: 0.0,
            resume_pos: None,
            last_session_save: std::time::Instant::now(),
            taste_cache: HashMap::new(),
            taste_samples: Vec::new(),
            taste_draft_genres: Vec::new(),
            taste_draft_blocked: Vec::new(),
            taste_draft_tracks: Vec::new(),
            amb_timer: Timer::default(),
            amb_running: false,
        })
    });
    with_state(|st| {
        st.player.send(Cmd::Normalize(st.settings.normalize));
        st.player.send(Cmd::Eq(st.settings.eq_on.then_some(st.settings.eq_gains)));
        st.player.send(Cmd::Crossfade(st.settings.crossfade_secs as f64));
    });
    refresh_playlists(&ui);
    refresh_liked(&ui);
    refresh_downloads(&ui);
    refresh_hidden(&ui);
    refresh_taste_summary(&ui);
    ui.set_runtime_status(runtime::status_text().into());
    if !runtime::is_ready() {
        ui.set_setup_banner("Çözücü ortamı arka planda hazırlanıyor…".into());
        let weak = weak.clone();
        thread::spawn(move || {
            let _ = runtime::ensure_ready();
            let status = runtime::status_text();
            let _ = weak.upgrade_in_event_loop(move |ui| {
                ui.set_setup_banner("".into());
                ui.set_runtime_status(status.into());
            });
        });
    }

    ui.on_play_shelf({
        let weak = weak.clone();
        move |si, i| {
            let Some(ui) = weak.upgrade() else { return };
            let list = with_state(|st| st.shelves.get(si as usize).map(|s| s.items.clone()));
            let Some(list) = list else { return };
            if i < 0 {
                // Odak bandı "Tümünü çal": tüm listeyi sırayla çal
                if !list.is_empty() {
                    play_list(&ui, list, 0);
                }
            } else if let Some(t) = list.get(i as usize).cloned() {
                // Tekil şarkı tıklaması: hemen çal ve o şarkının radyosunu başlat
                play_list(&ui, vec![t.clone()], 0);
                start_radio(&ui, t);
            }
        }
    });
    ui.on_play_result({
        let weak = weak.clone();
        move |i| {
            let Some(ui) = weak.upgrade() else { return };
            if let Some(t) = with_state(|st| st.results.get(i as usize).cloned()) {
                play_list(&ui, vec![t.clone()], 0);
                start_radio(&ui, t);
            }
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
            // Geri yüklenen oturum: parça henüz mpv'de değil, kaldığı yerden başlat
            let resume = with_state(|st| (!st.loaded && st.resume_pos.is_some()).then_some(st.index).flatten());
            if let Some(i) = resume {
                play_index(&ui, i);
                return;
            }
            let playing = ui.get_playing();
            with_state(|st| st.player.send(Cmd::SetPause(playing)));
        }
    });
    ui.on_seek({
        let weak = weak.clone();
        move |secs| {
            if let Some(ui) = weak.upgrade() {
                seek_to(&ui, secs as f64);
            }
        }
    });
    ui.on_set_volume(|v| {
        with_state(|st| {
            st.player.send(Cmd::Volume(v as f64));
            st.mpris.send(mpris::Msg::Volume(v as f64));
        })
    });
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

    ui.on_view_changed({
        let weak = weak.clone();
        move |v| {
            let Some(ui) = weak.upgrade() else { return };
            let can = with_state(|st| {
                if st.nav_skip {
                    st.nav_skip = false;
                } else if st.nav.last().map(|x| x.as_str()) != Some(v.as_str()) {
                    st.nav.push(v.to_string());
                    if st.nav.len() > 30 {
                        st.nav.remove(0);
                    }
                }
                st.nav.len() > 1
            });
            ui.set_can_back(can);
            if v == "profile" {
                refresh_profile(&ui);
            }
            update_viz(&ui);
            update_amb_timer(&ui);
        }
    });
    ui.on_play_profile_track({
        let weak = weak.clone();
        move |i| {
            let Some(ui) = weak.upgrade() else { return };
            let list = with_state(|st| st.prof_tracks.clone());
            if !list.is_empty() {
                play_list(&ui, list, i as usize);
            }
        }
    });
    ui.on_lyric_relayout({
        let weak = weak.clone();
        move || {
            let Some(ui) = weak.upgrade() else { return };
            with_state(|st| st.lyr_force = true);
            let pos = ui.get_position();
            update_lyrics(&ui, pos);
        }
    });
    ui.on_open_lyric_link({
        let weak = weak.clone();
        move |which| {
            let Some(ui) = weak.upgrade() else { return };
            let key = ui.get_now().key.to_string();
            let url = with_state(|st| {
                let c = st.lyrics.get(&key)?.as_ref()?.credit.clone()?;
                Some(if which == 0 { c.song_url } else { c.user_url })
            });
            if let Some(u) = url.filter(|u| !u.is_empty()) {
                let _ = open::that_detached(u);
            }
        }
    });
    ui.on_sheet_back({
        let weak = weak.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                sheet_back(&ui);
            }
        }
    });
    ui.on_go_back({
        let weak = weak.clone();
        move || {
            let Some(ui) = weak.upgrade() else { return };
            let prev = with_state(|st| {
                if st.nav.len() < 2 {
                    return None;
                }
                st.nav.pop();
                st.nav_skip = true;
                st.nav.last().cloned()
            });
            if let Some(p) = prev {
                ui.set_lyrics_open(false);
                ui.set_view(p.into());
                ui.set_can_back(with_state(|st| st.nav.len() > 1));
            }
        }
    });
    ui.on_search_edited({
        let weak = weak.clone();
        move |t| {
            if let Some(ui) = weak.upgrade() {
                search_edited(&ui, t.to_string());
            }
        }
    });
    ui.on_play_top({
        let weak = weak.clone();
        move || {
            let Some(ui) = weak.upgrade() else { return };
            match with_state(|st| st.top.clone()) {
                Some(Top::Song(t)) => {
                    play_list(&ui, vec![t.clone()], 0);
                    start_radio(&ui, t);
                }
                Some(Top::Card(c)) => open_card(&ui, c),
                None => {}
            }
        }
    });
    ui.on_open_search_card({
        let weak = weak.clone();
        move |si, i| {
            let Some(ui) = weak.upgrade() else { return };
            let c = with_state(|st| st.search_shelves.get(si as usize).and_then(|s| s.cards.get(i as usize)).cloned());
            if let Some(c) = c {
                open_card(&ui, c);
            }
        }
    });
    ui.on_open_card({
        let weak = weak.clone();
        move |si, i| {
            let Some(ui) = weak.upgrade() else { return };
            let c = with_state(|st| st.shelves.get(si as usize).and_then(|s| s.cards.get(i as usize)).cloned());
            if let Some(c) = c {
                open_card(&ui, c);
            }
        }
    });
    ui.on_open_browse_card({
        let weak = weak.clone();
        move |i| {
            let Some(ui) = weak.upgrade() else { return };
            if let Some(c) = with_state(|st| st.browse.get(i as usize).cloned()) {
                open_card(&ui, c);
            }
        }
    });
    ui.on_play_collection({
        let weak = weak.clone();
        move |i| {
            let Some(ui) = weak.upgrade() else { return };
            if let Some(list) = with_state(|st| st.coll.as_ref().map(|c| c.1.clone())) {
                if !list.is_empty() {
                    play_list(&ui, list, i.max(0) as usize);
                }
            }
        }
    });
    ui.on_play_artist({
        let weak = weak.clone();
        move |i| {
            let Some(ui) = weak.upgrade() else { return };
            let list = with_state(|st| st.artist.as_ref().map(|a| a.1.clone())).unwrap_or_default();
            if !list.is_empty() {
                play_list(&ui, list, i.max(0) as usize);
            }
        }
    });
    ui.on_shuffle_artist({
        let weak = weak.clone();
        move || {
            let Some(ui) = weak.upgrade() else { return };
            let list = with_state(|st| st.artist.as_ref().map(|a| a.1.clone())).unwrap_or_default();
            if !list.is_empty() {
                play_list(&ui, shuffled(list), 0);
            }
        }
    });
    ui.on_radio_artist({
        let weak = weak.clone();
        move || {
            let Some(ui) = weak.upgrade() else { return };
            if let Some(t) = with_state(|st| st.artist.as_ref().and_then(|a| a.1.first().cloned())) {
                start_radio(&ui, t);
            }
        }
    });
    ui.on_open_artist_card({
        let weak = weak.clone();
        move |si, i| {
            let Some(ui) = weak.upgrade() else { return };
            let c = with_state(|st| st.artist.as_ref().and_then(|a| a.2.get(si as usize)).and_then(|s| s.cards.get(i as usize)).cloned());
            if let Some(c) = c {
                open_card(&ui, c);
            }
        }
    });
    ui.on_save_collection({
        let weak = weak.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                save_collection(&ui);
            }
        }
    });
    ui.on_toggle_like({
        let weak = weak.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                toggle_like_current(&ui);
            }
        }
    });
    ui.on_toggle_shuffle({
        let weak = weak.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                toggle_shuffle(&ui);
            }
        }
    });
    ui.on_cycle_repeat({
        let weak = weak.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                cycle_repeat(&ui);
            }
        }
    });
    ui.on_play_mix({
        let weak = weak.clone();
        move || {
            let Some(ui) = weak.upgrade() else { return };
            let mix = with_state(|st| st.mix.clone());
            if !mix.is_empty() {
                play_list(&ui, mix, 0);
            }
        }
    });
    ui.on_refresh_home({
        let weak = weak.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                toast(&ui, "Öneriler yenileniyor…");
                load_home(&ui);
            }
        }
    });
    ui.on_radio_from_current({
        let weak = weak.clone();
        move || {
            let Some(ui) = weak.upgrade() else { return };
            if let Some(t) = with_state(|st| st.index.and_then(|i| st.queue.get(i).cloned())) {
                start_radio(&ui, t);
            }
        }
    });
    ui.on_queue_remove({
        let weak = weak.clone();
        move |i| {
            if let Some(ui) = weak.upgrade() {
                queue_remove(&ui, i as usize);
            }
        }
    });
    ui.on_queue_move({
        let weak = weak.clone();
        move |i, d| {
            if let Some(ui) = weak.upgrade() {
                queue_move(&ui, i as usize, d);
            }
        }
    });
    ui.on_queue_clear({
        let weak = weak.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                queue_clear(&ui);
            }
        }
    });
    ui.on_update_later({
        let weak = weak.clone();
        move || {
            let Some(ui) = weak.upgrade() else { return };
            updater::CANCEL.store(true, Ordering::Relaxed);
            ui.set_update_state(0);
        }
    });
    ui.on_update_skip({
        let weak = weak.clone();
        move || {
            let Some(ui) = weak.upgrade() else { return };
            updater::CANCEL.store(true, Ordering::Relaxed);
            ui.set_update_state(0);
            with_state(|st| {
                if let Some(u) = &st.pending_update {
                    st.settings.skip_version = u.version.clone();
                    let _ = st.settings.save();
                }
            });
            toast(&ui, "Bu sürüm atlanacak; Ayarlar'dan yine kurabilirsin");
        }
    });
    ui.on_update_retry({
        let weak = weak.clone();
        move || {
            let Some(ui) = weak.upgrade() else { return };
            if let Some(u) = with_state(|st| st.pending_update.clone()) {
                start_update(&ui, u);
            }
        }
    });
    ui.on_update_open_page(|| {
        let _ = open::that_detached(updater::RELEASES_PAGE);
    });
    ui.on_check_updates({
        let weak = weak.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                check_update(&ui, true);
            }
        }
    });
    ui.on_eq_set({
        let weak = weak.clone();
        move |band, gain| {
            let Some(ui) = weak.upgrade() else { return };
            let gains = with_state(|st| {
                if let Some(g) = st.settings.eq_gains.get_mut(band as usize) {
                    *g = gain.clamp(-12.0, 12.0);
                }
                st.settings.eq_preset = settings::EQ_PRESETS.iter().position(|(_, p)| *p == st.settings.eq_gains).map(|i| i as i32).unwrap_or(-1);
                st.settings.eq_on = true;
                st.settings.eq_gains
            });
            eq_changed(&ui, gains);
        }
    });
    ui.on_eq_pick({
        let weak = weak.clone();
        move |i| {
            let Some(ui) = weak.upgrade() else { return };
            let Some((_, g)) = settings::EQ_PRESETS.get(i as usize) else { return };
            with_state(|st| {
                st.settings.eq_gains = *g;
                st.settings.eq_preset = i;
                st.settings.eq_on = i != 0 || st.settings.eq_on;
            });
            eq_changed(&ui, *g);
        }
    });
    ui.global::<Nav>().on_artist({
        let weak = weak.clone();
        move |artists, key, x, y| {
            let Some(ui) = weak.upgrade() else { return };
            let names = split_artists(&artists);
            // Yalnız YouTube Music anahtarında videoId ipucu var ("0-<id>")
            let vid = key.strip_prefix("0-").unwrap_or("").to_string();
            match names.len() {
                0 => {}
                1 => open_artist_by_name(&ui, &names[0], &vid),
                _ => {
                    ui.set_artist_pick_names(ModelRc::new(VecModel::from(names.iter().map(|n| slint::SharedString::from(n.as_str())).collect::<Vec<_>>())));
                    ui.set_artist_pick_x(x);
                    ui.set_artist_pick_y(y);
                    ui.set_menu_open(false);
                    ui.set_artist_pick_open(true);
                    with_state(|st| st.artist_pick = (names, vid));
                }
            }
        }
    });
    ui.on_artist_pick({
        let weak = weak.clone();
        move |i| {
            let Some(ui) = weak.upgrade() else { return };
            let pick = with_state(|st| st.artist_pick.0.get(i as usize).cloned().map(|n| (n, st.artist_pick.1.clone())));
            if let Some((name, vid)) = pick {
                open_artist_by_name(&ui, &name, &vid);
            }
        }
    });
    ui.on_menu_for({
        let weak = weak.clone();
        move |kind, a, b| {
            let Some(ui) = weak.upgrade() else { return };
            let t = with_state(|st| {
                let t = menu_target(st, kind, a, b);
                st.menu = t.clone();
                t.map(|t| (t.title.clone(), st.history.is_liked(&t.key()), st.history.is_boosted(&t.key())))
            });
            match t {
                Some((title, liked, boosted)) => {
                    let downloaded = with_state(|st| st.menu.as_ref().is_some_and(|t| downloads::path(&t.key()).is_some()));
                    ui.set_menu_downloaded(downloaded);
                    ui.set_menu_title(title.into());
                    ui.set_menu_liked(liked);
                    ui.set_menu_boosted(boosted);
                }
                None => ui.set_menu_open(false),
            }
        }
    });
    ui.on_menu_action({
        let weak = weak.clone();
        move |a| {
            if let Some(ui) = weak.upgrade() {
                ui.set_menu_open(false);
                menu_action(&ui, a);
            }
        }
    });
    ui.on_lyrics_web_search({
        let weak = weak.clone();
        move || {
            let Some(ui) = weak.upgrade() else { return };
            let now = ui.get_now();
            let q = format!("{} {} şarkı sözleri", now.artist, now.title);
            let enc: String = q
                .bytes()
                .map(|b| if b.is_ascii_alphanumeric() { (b as char).to_string() } else if b == b' ' { "+".into() } else { format!("%{b:02X}") })
                .collect();
            let _ = open::that_detached(format!("https://www.google.com/search?q={enc}"));
        }
    });
    ui.on_lyric_tick({
        let weak = weak.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                lyric_tick(&ui);
            }
        }
    });
    ui.on_lyric_seek({
        let weak = weak.clone();
        move |t| {
            let Some(ui) = weak.upgrade() else { return };
            let off = with_state(|st| st.lyr_offsets.get(&ui.get_now().key.to_string()).copied().unwrap_or(0.0));
            seek_to(&ui, (t - off).max(0.0) as f64);
        }
    });
    ui.on_lyrics_style_changed({
        let weak = weak.clone();
        move || {
            let Some(ui) = weak.upgrade() else { return };
            apply_settings(&ui);
            // Boyut ya da çeviri değişince satırlar yeniden kurulsun
            render_lyrics(&ui);
        }
    });
    ui.on_lyric_shift({
        let weak = weak.clone();
        move |d| {
            let Some(ui) = weak.upgrade() else { return };
            let key = ui.get_now().key.to_string();
            let off = with_state(|st| {
                let o = st.lyr_offsets.entry(key).or_insert(0.0);
                *o = ((*o + d) * 10.0).round() / 10.0;
                let v = *o;
                st.lyr_line = None;
                st.lyr_spoken = usize::MAX;
                // Şarkıya özel ayar kalıcı olsun
                st.lyr_offsets.retain(|_, x| x.abs() > 0.01);
                if let Some(dir) = offsets_path().parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let _ = std::fs::write(offsets_path(), serde_json::to_vec(&st.lyr_offsets).unwrap_or_default());
                v
            });
            ui.set_lyric_offset(off);
            update_lyrics(&ui, ui.get_position());
        }
    });
    ui.on_start_import({
        let weak = weak.clone();
        move |url| {
            if let Some(ui) = weak.upgrade() {
                start_import(&ui, url.trim().to_string());
            }
        }
    });
    ui.on_pick_file({
        let weak = weak.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                pick_file(&ui);
            }
        }
    });
    ui.on_import_file_list({
        let weak = weak.clone();
        move |i| {
            let Some(ui) = weak.upgrade() else { return };
            if let Some(l) = with_state(|st| st.file_lists.get(i as usize).cloned()) {
                match_file_list(&ui, l);
            }
        }
    });
    ui.on_import_change({
        let weak = weak.clone();
        move |i| {
            if let Some(ui) = weak.upgrade() {
                import_change(&ui, i as usize);
            }
        }
    });
    ui.on_import_save({
        let weak = weak.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                import_save(&ui);
            }
        }
    });
    ui.on_open_playlist({
        let weak = weak.clone();
        move |i| {
            if let Some(ui) = weak.upgrade() {
                open_playlist(&ui, i as usize);
            }
        }
    });
    ui.on_play_playlist({
        let weak = weak.clone();
        move |j| {
            let Some(ui) = weak.upgrade() else { return };
            let (list, real_i) = with_state(|st| {
                let real = st.pl_visible.get(j as usize).copied().unwrap_or(j as usize);
                let list = st.current_pl.and_then(|i| st.playlists.get(i)).map(|p| p.tracks.clone());
                (list, real)
            });
            if let Some(list) = list {
                if !list.is_empty() {
                    let idx = real_i.min(list.len() - 1);
                    play_list(&ui, list, idx);
                }
            }
        }
    });
    ui.on_rename_playlist({
        let weak = weak.clone();
        move |i, name| {
            if let Some(ui) = weak.upgrade() {
                rename_playlist(&ui, i as usize, name.to_string());
            }
        }
    });
    ui.on_shuffle_playlist({
        let weak = weak.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                shuffle_playlist(&ui);
            }
        }
    });
    ui.on_move_playlist_track({
        let weak = weak.clone();
        move |i, d| {
            if let Some(ui) = weak.upgrade() {
                move_playlist_track(&ui, i as usize, d);
            }
        }
    });
    ui.on_remove_from_playlist({
        let weak = weak.clone();
        move |i| {
            if let Some(ui) = weak.upgrade() {
                remove_from_playlist(&ui, i as usize);
            }
        }
    });
    ui.on_filter_playlist({
        let weak = weak.clone();
        move |q| {
            if let Some(ui) = weak.upgrade() {
                filter_playlist(&ui, q.to_string());
            }
        }
    });
    ui.on_import_remove({
        let weak = weak.clone();
        move |i| {
            if let Some(ui) = weak.upgrade() {
                import_remove(&ui, i as usize);
            }
        }
    });
    ui.on_pick_playlist_image({
        let weak = weak.clone();
        move |i| {
            if let Some(ui) = weak.upgrade() {
                pick_playlist_image(&ui, i as usize);
            }
        }
    });
    ui.on_clear_playlist_image({
        let weak = weak.clone();
        move |i| {
            if let Some(ui) = weak.upgrade() {
                clear_playlist_image(&ui, i as usize);
            }
        }
    });
    ui.on_undo({
        let weak = weak.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                apply_undo(&ui);
            }
        }
    });
    ui.on_import_clear({
        let weak = weak.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                import_clear(&ui);
            }
        }
    });
    ui.on_delete_playlist({
        let weak = weak.clone();
        move |i| {
            if let Some(ui) = weak.upgrade() {
                delete_playlist(&ui, i as usize);
            }
        }
    });
    ui.on_create_playlist({
        let weak = weak.clone();
        move |name| {
            if let Some(ui) = weak.upgrade() {
                create_new_playlist(&ui, name.to_string());
            }
        }
    });
    ui.on_create_playlist_with_track({
        let weak = weak.clone();
        move |name| {
            if let Some(ui) = weak.upgrade() {
                create_playlist_with_track(&ui, name.to_string());
            }
        }
    });
    ui.on_add_to_playlist({
        let weak = weak.clone();
        move |pl_idx| {
            if let Some(ui) = weak.upgrade() {
                add_menu_track_to_playlist(&ui, pl_idx as usize);
            }
        }
    });
    ui.on_play_downloads({
        let weak = weak.clone();
        move |i, shuffle| {
            let Some(ui) = weak.upgrade() else { return };
            let mut list = with_state(|st| st.dl_list.clone());
            if list.is_empty() {
                return;
            }
            let mut start = i.max(0) as usize;
            if shuffle {
                let mut seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1);
                for k in (1..list.len()).rev() {
                    seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                    list.swap(k, (seed as usize) % (k + 1));
                }
                start = 0;
            }
            play_list(&ui, list, start);
        }
    });
    ui.on_download_view({
        let weak = weak.clone();
        move || {
            let Some(ui) = weak.upgrade() else { return };
            let view = ui.get_view().to_string();
            let tracks = with_state(|st| match view.as_str() {
                "liked" => st.history.likes.clone(),
                "playlist" => st.current_pl.and_then(|p| st.playlists.get(p)).map(|p| p.tracks.clone()).unwrap_or_default(),
                _ => Vec::new(),
            });
            if !tracks.is_empty() {
                download_tracks(&ui, tracks);
            }
        }
    });
    ui.on_play_liked({
        let weak = weak.clone();
        move |i| {
            let Some(ui) = weak.upgrade() else { return };
            let list = with_state(|st| st.history.likes.clone());
            if !list.is_empty() {
                play_list(&ui, list, i as usize);
            }
        }
    });
    ui.on_shuffle_liked({
        let weak = weak.clone();
        move || {
            let Some(ui) = weak.upgrade() else { return };
            let mut list = with_state(|st| st.history.likes.clone());
            if list.is_empty() {
                return;
            }
            let n = list.len();
            let mut seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1);
            for i in (1..n).rev() {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                let j = (seed as usize) % (i + 1);
                list.swap(i, j);
            }
            play_list(&ui, list, 0);
        }
    });
    ui.on_update_runtime({
        let weak = weak.clone();
        move || {
            let Some(ui) = weak.upgrade() else { return };
            ui.set_runtime_updating(true);
            let w = weak.clone();
            runtime::update_now(move |res| {
                let status = runtime::status_text();
                let _ = w.upgrade_in_event_loop(move |ui| {
                    ui.set_runtime_updating(false);
                    ui.set_runtime_status(status.into());
                    match res {
                        Ok(msg) => toast(&ui, &msg),
                        Err(e) => toast(&ui, &format!("Güncelleme hatası: {e}")),
                    }
                });
            });
        }
    });
    ui.on_open_tastes({
        let weak = weak.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                open_tastes(&ui);
            }
        }
    });
    ui.on_reset_recs({
        let weak = weak.clone();
        move |with_history| {
            if let Some(ui) = weak.upgrade() {
                reset_recs(&ui, with_history);
            }
        }
    });
    ui.on_taste_toggle_genre({
        let weak = weak.clone();
        move |gid| {
            if let Some(ui) = weak.upgrade() {
                taste_toggle_genre(&ui, gid.to_string());
            }
        }
    });
    ui.on_taste_block_genre({
        let weak = weak.clone();
        move |gid| {
            if let Some(ui) = weak.upgrade() {
                taste_block_genre(&ui, gid.to_string());
            }
        }
    });
    ui.on_taste_toggle_sample({
        let weak = weak.clone();
        move |i| {
            if let Some(ui) = weak.upgrade() {
                taste_toggle_sample(&ui, i as usize);
            }
        }
    });
    ui.on_taste_preview_sample({
        let weak = weak.clone();
        move |i| {
            if let Some(ui) = weak.upgrade() {
                taste_preview_sample(&ui, i as usize);
            }
        }
    });
    ui.on_taste_save({
        let weak = weak.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                taste_save(&ui);
            }
        }
    });
    ui.on_taste_skip({
        let weak = weak.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                taste_skip(&ui);
            }
        }
    });
    ui.on_unhide_artist({
        let weak = weak.clone();
        move |i| {
            let Some(ui) = weak.upgrade() else { return };
            let home_data = with_state(|st| {
                if let Some(a) = st.history.disliked_artists.get(i as usize).cloned() {
                    st.history.undislike_artist(&a);
                }
                (!st.home.personal.is_empty() || !st.home.external.is_empty()).then(|| st.home.clone())
            });
            refresh_hidden(&ui);
            if let Some(d) = home_data {
                show_home(&ui, d);
            }
            toast(&ui, "Sanatçı engeli kaldırıldı");
        }
    });
    ui.on_unhide_track({
        let weak = weak.clone();
        move |i| {
            let Some(ui) = weak.upgrade() else { return };
            let home_data = with_state(|st| {
                if let Some(t) = st.history.disliked_tracks.get(i as usize).cloned() {
                    st.history.undislike_track(&t.key());
                }
                (!st.home.personal.is_empty() || !st.home.external.is_empty()).then(|| st.home.clone())
            });
            refresh_hidden(&ui);
            if let Some(d) = home_data {
                show_home(&ui, d);
            }
            toast(&ui, "Şarkı engeli kaldırıldı");
        }
    });

    // Test: VYNZ_SCROLL=piksel ile açılıştan birkaç saniye sonra sayfayı kaydır.
    let scroll_timer = Timer::default();
    if let Some(px) = std::env::var("VYNZ_SCROLL").ok().and_then(|v| v.parse::<f32>().ok()) {
        let weak = weak.clone();
        scroll_timer.start(TimerMode::SingleShot, Duration::from_secs(6), move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_scroll_request(px);
            }
        });
    }

    with_state(|st| {
        let weak = weak.clone();
        st.watchdog.start(TimerMode::Repeated, Duration::from_secs(1), move || {
            if let Some(ui) = weak.upgrade() {
                watchdog_check(&ui);
            }
        });
    });

    let force_quit = Arc::new(AtomicBool::new(false));
    // VYNZ_FAKE_WINDOWS=1: Windows başlık çubuğu düzenini Linux'ta görmek için (yalnız test)
    ui.set_is_windows(cfg!(target_os = "windows") || std::env::var_os("VYNZ_FAKE_WINDOWS").is_some());

    #[cfg(target_os = "windows")]
    let _tray_guard = {
        let weak = weak.clone();
        let fq = force_quit.clone();
        tray::Tray::start(move |act| {
            let weak = weak.clone();
            let fq = fq.clone();
            let _ = slint::invoke_from_event_loop(move || {
                let Some(ui) = weak.upgrade() else { return };
                match act {
                    tray::TrayAct::Show => show_from_tray(&ui),
                    tray::TrayAct::PlayPause => on_mpris(&ui, mpris::Act::PlayPause),
                    tray::TrayAct::Next => on_mpris(&ui, mpris::Act::Next),
                    tray::TrayAct::Quit => {
                        fq.store(true, Ordering::Relaxed);
                        let _ = slint::quit_event_loop();
                    }
                }
            });
        })
    };

    ui.on_win_drag({
        let weak = weak.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                use slint::winit_030::WinitWindowAccessor;
                ui.window().with_winit_window(|w| {
                    let _ = w.drag_window();
                });
            }
        }
    });
    ui.on_win_minimize({
        let weak = weak.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                use slint::winit_030::WinitWindowAccessor;
                ui.window().with_winit_window(|w| w.set_minimized(true));
            }
        }
    });
    // Ortam zemini görselleri (bir kez)
    ui.set_fx_disc(Image::from_rgba8(SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(
        &ambience::disc_rgba(),
        ambience::DISC,
        ambience::DISC,
    )));
    ui.set_fx_grain(Image::from_rgba8(SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(
        &ambience::grain_rgba(),
        ambience::GRAIN,
        ambience::GRAIN,
    )));
    ui.set_can_react(cfg!(target_os = "linux"));
    if std::env::var_os("VYNZ_SLEEP_TEST").is_some() {
        ui.set_sleep_unit(1);
    }
    ui.on_sleep_fire({
        let weak = weak.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                sleep_fire(&ui);
            }
        }
    });
    ui.on_viz_check({
        let weak = weak.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                update_viz(&ui);
            }
        }
    });
    ui.on_amb_check({
        let weak = weak.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                update_amb_timer(&ui);
            }
        }
    });
    ui.on_mini_open({
        let weak = weak.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                mini_open(&ui);
            }
        }
    });
    ui.on_game_mode_set({
        let weak = weak.clone();
        move |on| {
            if let Some(ui) = weak.upgrade() {
                set_game_mode(&ui, on);
            }
        }
    });
    ui.on_toggle_fullscreen({
        let weak = weak.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                use slint::winit_030::winit::window::Fullscreen;
                use slint::winit_030::WinitWindowAccessor;
                let mut now = false;
                ui.window().with_winit_window(|w| {
                    now = w.fullscreen().is_none();
                    w.set_fullscreen(if now { Some(Fullscreen::Borderless(None)) } else { None });
                });
                ui.set_fullscreen(now);
            }
        }
    });
    ui.on_win_maximize({
        let weak = weak.clone();
        move || {
            if let Some(ui) = weak.upgrade() {
                use slint::winit_030::WinitWindowAccessor;
                ui.window().with_winit_window(|w| w.set_maximized(!w.is_maximized()));
            }
        }
    });
    ui.on_win_close({
        let weak = weak.clone();
        let fq = force_quit.clone();
        move || {
            let Some(ui) = weak.upgrade() else { return };
            if cfg!(target_os = "windows") && ui.get_close_to_tray() {
                hide_to_tray(&ui);
            } else {
                fq.store(true, Ordering::Relaxed);
                let _ = slint::quit_event_loop();
            }
        }
    });
    ui.window().on_close_requested({
        let weak = weak.clone();
        let fq = force_quit.clone();
        move || {
            if cfg!(target_os = "windows") && !fq.load(Ordering::Relaxed) {
                if let Some(ui) = weak.upgrade() {
                    if ui.get_close_to_tray() {
                        hide_to_tray(&ui);
                        return slint::CloseRequestResponse::KeepWindowShown;
                    }
                }
            }
            slint::CloseRequestResponse::HideWindow
        }
    });

    #[cfg(target_os = "windows")]
    let _win_frame_timer = {
        let t = Timer::default();
        let weak = weak.clone();
        t.start(TimerMode::SingleShot, Duration::from_millis(120), move || {
            if let Some(ui) = weak.upgrade() {
                use slint::winit_030::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
                use slint::winit_030::WinitWindowAccessor;
                use windows::Win32::Foundation::HWND;
                use windows::Win32::Graphics::Dwm::{
                    DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
                };
                ui.window().with_winit_window(|w| {
                    w.set_decorations(false);
                    if let Ok(handle) = w.window_handle() {
                        if let RawWindowHandle::Win32(h) = handle.as_raw() {
                            let hwnd = HWND(h.hwnd.get() as *mut std::ffi::c_void);
                            let dark: u32 = 1;
                            let round: i32 = DWMWCP_ROUND.0;
                            unsafe {
                                let _ = DwmSetWindowAttribute(
                                    hwnd,
                                    DWMWA_USE_IMMERSIVE_DARK_MODE,
                                    (&dark as *const u32).cast(),
                                    std::mem::size_of::<u32>() as u32,
                                );
                                let _ = DwmSetWindowAttribute(
                                    hwnd,
                                    DWMWA_WINDOW_CORNER_PREFERENCE,
                                    (&round as *const i32).cast(),
                                    std::mem::size_of::<i32>() as u32,
                                );
                            }
                        }
                    }
                });
            }
        });
        t
    };

    restore_session(&ui);
    // SIGTERM/SIGINT (kapanış, oturum sonu, kill): döngüden düzgün çık ki oturum kaydedilsin.
    let _ = ctrlc::set_handler(|| {
        let _ = slint::invoke_from_event_loop(|| {
            let _ = slint::quit_event_loop();
        });
    });
    refresh_usage(&ui);
    load_home(&ui);
    if std::env::var_os("VYNZ_UPDATED").is_some() {
        toast(&ui, &format!("Vynz {} sürümüne güncellendi", updater::current()));
    }
    if ui.get_auto_update() && !updater::is_dev_build() {
        // Açılışı yavaşlatmasın: biraz sonra, arka planda.
        let weak = ui.as_weak();
        Timer::single_shot(Duration::from_secs(3), move || {
            if let Some(ui) = weak.upgrade() {
                check_update(&ui, false);
            }
        });
    }
    if with_state(|st| !st.settings.tastes_done && st.history.is_empty()) {
        open_tastes(&ui);
    }
    update_amb_timer(&ui);
    ui.run()?;
    save_session(&ui);
    // Ses düzeyi bir dahaki açılışta aynı gelsin.
    with_state(|st| {
        st.settings.volume = ui.get_volume();
        if let Err(e) = st.settings.save() {
            eprintln!("ayarlar kaydedilemedi: {e}");
        }
    });
    with_state(|st| st.viz = None);
    start_listen(None); // son dinlemeyi kaydet
    resolver.shutdown();
    if updater::RESTART.load(Ordering::Relaxed) {
        updater::relaunch();
    }
    Ok(())
}

#[cfg(test)]
mod queue_tests {
    use super::moved_index;

    #[test]
    fn moving_keeps_current_track() {
        // kuyruk: a b C d e (çalan 2)
        assert_eq!(moved_index(2, 2, 4), 4); // çalanı sona taşı
        assert_eq!(moved_index(2, 0, 3), 1); // a, C'nin arkasına
        assert_eq!(moved_index(2, 4, 0), 3); // e başa
        assert_eq!(moved_index(2, 3, 4), 2); // çalandan sonrası kendi arasında
        assert_eq!(moved_index(2, 0, 1), 2);
        assert_eq!(moved_index(2, 1, 2), 1);
    }
}
