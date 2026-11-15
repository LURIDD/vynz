// vynz: çok kaynaklı müzik çalar.
// Oynatma yolu (hızlıdan yavaşa):
//   1. Disk önbelleği (daha önce sonuna kadar dinlenmiş parça)   ~0.3 sn
//   2. Önceden çözülmüş akış adresi                               ~1.2 sn
//   3. Çözücü yardımcısı ("şimdi" önceliği)                       ~3-5 sn
//   4. Yedek: mpv'nin yt-dlp kancası                              ~8-10 sn

mod art;
mod cache;
mod fetcher;
mod history;
mod import;
mod library;
mod lyrics;
mod model;
mod mpris;
mod player;
mod recommend;
mod resolve;
mod settings;
mod sources;
mod stats;
mod viz;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
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
    imported: Option<import::Fetched>,
    import_origin: String,
    playlists: Vec<library::Playlist>,
    current_pl: Option<usize>,
    /// Söz önbelleği: None = bulunamadı.
    lyrics: HashMap<String, Option<lyrics::Lyrics>>,
    lyr_pending: HashSet<String>,
    lyr_line: Option<usize>,
    lyr_spoken: usize,
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
    /// Kişisel karışım (ana sayfadaki "Karışımı çal")
    mix: Vec<TrackInfo>,
    /// Açık koleksiyon (liste/albüm/sanatçı): başlık, kart, şarkılar
    coll: Option<(Card, Vec<TrackInfo>)>,
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
    /// Söz ekranı zemini: önceden bulanıklaştırılmış küçük kapaklar.
    static BLURS: RefCell<HashMap<String, Image>> = RefCell::new(HashMap::new());
    /// Yalnız çalan şarkı için büyük kapaklar (kahraman, söz çekmecesi).
    static BIGS: RefCell<HashMap<String, Image>> = RefCell::new(HashMap::new());
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
            // "B|" önekli istek: çalan şarkının büyük kapağı (aynı önbellek dosyası).
            let (file_key, size) = match key.strip_prefix("B|") {
                Some(k) => (k.to_string(), art::BIG),
                None => (key.clone(), art::SMALL),
            };
            match art::load(&file_key, &url, size) {
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
    if let Some(k) = key.strip_prefix("B|") {
        let img = Image::from_rgba8(SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(&a.rgba, a.width, a.height));
        BIGS.with(|m| {
            let mut m = m.borrow_mut();
            if m.len() > 8 {
                m.clear(); // yalnız son birkaç çalan şarkı tutulur
            }
            m.insert(k.to_string(), img.clone());
        });
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
    ARTS.with(|m| m.borrow_mut().insert(key.clone(), (img.clone(), color)));

    let shelves = ui.get_shelves();
    for i in 0..shelves.row_count() {
        if let Some(s) = shelves.row_data(i) {
            update_rows(&s.items, &key, &img);
            update_cards(&s.cards, &key, &img);
        }
    }
    update_cards(&ui.get_browse_cards(), &key, &img);
    update_rows(&ui.get_coll_tracks(), &key, &img);
    update_rows(&ui.get_results(), &key, &img);
    update_rows(&ui.get_queue(), &key, &img);
    update_rows(&ui.get_playlist_tracks(), &key, &img);
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
    BLURS.with(|m| m.borrow_mut().insert(key.clone(), blur.clone()));

    let mut now = ui.get_now();
    if ui.get_has_now() && now.key == key {
        // Büyük kapak hazırsa onu koru.
        if !BIGS.with(|b| b.borrow().contains_key(&key)) {
            now.art = img;
            now.has_art = true;
            ui.set_now(now);
        }
        ui.global::<Theme>().set_accent(color);
        ui.set_now_blur(blur);
        ui.set_has_blur(true);
    }
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
    ui.set_position(0.0);
    ui.set_duration(t.duration);
    ui.set_playing(true);
    let accent = ARTS.with(|a| a.borrow().get(&t.key()).map(|(_, c)| *c)).unwrap_or(NEUTRAL);
    ui.global::<Theme>().set_accent(accent);
    match BLURS.with(|b| b.borrow().get(&t.key()).cloned()) {
        Some(b) => {
            ui.set_now_blur(b);
            ui.set_has_blur(true);
        }
        None => ui.set_has_blur(false),
    }
    request_lyrics(ui, t);
    render_lyrics(ui);
}

// ---------- Sözler ----------

fn request_lyrics(ui: &AppWindow, t: &TrackInfo) {
    let key = t.key();
    let need = with_state(|st| !st.lyrics.contains_key(&key) && st.lyr_pending.insert(key.clone()));
    if !need {
        return;
    }
    let t = t.clone();
    let weak = ui.as_weak();
    thread::spawn(move || {
        let res = lyrics::fetch(&t).ok();
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

/// Çalan şarkının söz durumunu baştan kur.
fn render_lyrics(ui: &AppWindow) {
    let key = ui.get_now().key.to_string();
    let (state, source, plain, offset, lines) = with_state(|st| {
        st.lyr_line = None;
        st.lyr_spoken = usize::MAX;
        let offset = st.lyr_offsets.get(&key).copied().unwrap_or(0.0);
        match st.lyrics.get(&key) {
            None => (if st.lyr_pending.contains(&key) { 1 } else { 0 }, String::new(), Vec::new(), offset, Vec::new()),
            Some(None) => (4, String::new(), Vec::new(), offset, Vec::new()),
            Some(Some(l)) if l.synced => {
                let lines = l.lines.iter().map(|x| LyricLine { text: x.text.as_str().into(), t: x.t, state: 1, dist: 9 }).collect();
                (2, l.source.clone(), Vec::new(), offset, lines)
            }
            Some(Some(l)) => (3, l.source.clone(), l.lines.iter().map(|x| x.text.clone()).collect(), offset, Vec::new()),
        }
    });
    ui.set_lyric_state(state);
    ui.set_lyric_source(source.into());
    ui.set_lyric_offset(offset);
    ui.set_lyric_plain(ModelRc::new(VecModel::from(plain.into_iter().map(Into::into).collect::<Vec<slint::SharedString>>())));
    ui.set_lyric_lines(ModelRc::new(VecModel::from(lines)));
    ui.set_lyric_active(-1);
    ui.set_lyric_gap(-1.0);
    ui.set_lyric_row0(ModelRc::new(VecModel::from(Vec::<LyricWord>::new())));
    ui.set_lyric_row1(ModelRc::new(VecModel::from(Vec::<LyricWord>::new())));
    update_lyrics(ui, ui.get_position());
}

/// Uzun satırı kelime sınırından iki dengeli satıra böl (Slint'te kelime kaydırma için).
fn word_rows(words: &[&str], limit: usize) -> Vec<i32> {
    let total: usize = words.iter().map(|w| w.chars().count() + 1).sum();
    if total <= limit || words.len() < 2 {
        return vec![0; words.len()];
    }
    let mut best = (usize::MAX, 1);
    let mut acc = 0;
    for (k, w) in words.iter().enumerate().take(words.len() - 1) {
        acc += w.chars().count() + 1;
        let d = (acc as i64 - (total / 2) as i64).unsigned_abs() as usize;
        if d < best.0 {
            best = (d, k + 1);
        }
    }
    (0..words.len()).map(|i| if i < best.1 { 0 } else { 1 }).collect()
}

fn word_state(k: usize, spoken: usize, fill: f32) -> (i32, f32) {
    if k < spoken {
        (0, 1.0)
    } else if k == spoken {
        (1, fill)
    } else {
        (2, 0.0)
    }
}

/// Konuma göre etkin satırı, kelime dolumunu ve müzik arasını güncelle (yalnız değişenler).
fn update_lyrics(ui: &AppWindow, pos: f32) {
    if ui.get_lyric_state() != 2 {
        return;
    }
    let key = ui.get_now().key.to_string();
    // Satır başına sığan harf: yazı boyutuna göre (Slint'te kendiliğinden kaydırma yok)
    let limit = match ui.get_lyrics_size() { 0 => 34, 2 => 22, _ => 28 };
    enum Change {
        None,
        Line { idx: Option<usize>, words: Vec<LyricWord> },
        Words { spoken: usize, fill: f32 },
    }
    let (change, gap) = with_state(|st| {
        let offset = st.lyr_offsets.get(&key).copied().unwrap_or(0.0);
        let Some(Some(l)) = st.lyrics.get(&key) else { return (Change::None, None) };
        let p = pos + offset;
        let gap = lyrics::gap(l, p);
        let (idx, spoken, fill) = match lyrics::locate(l, p) {
            Some((i, k, f)) => (Some(i), k, f),
            None => (None, 0, 0.0),
        };
        if idx != st.lyr_line || st.lyr_spoken == usize::MAX {
            st.lyr_line = idx;
            st.lyr_spoken = spoken;
            let words = match idx {
                Some(i) => {
                    let ws: Vec<&str> = l.lines[i].text.split_whitespace().collect();
                    let rows = word_rows(&ws, limit);
                    ws.iter()
                        .enumerate()
                        .map(|(k, w)| {
                            let (state, f) = word_state(k, spoken, fill);
                            LyricWord { text: (*w).into(), state, row: rows[k], fill: f }
                        })
                        .collect()
                }
                None => Vec::new(),
            };
            return (Change::Line { idx, words }, gap);
        }
        st.lyr_spoken = spoken;
        (Change::Words { spoken, fill }, gap)
    });
    ui.set_lyric_gap(gap.unwrap_or(-1.0));
    match change {
        Change::None => {}
        Change::Line { idx, words } => {
            let (r0, r1): (Vec<LyricWord>, Vec<LyricWord>) = words.into_iter().partition(|w| w.row == 0);
            ui.set_lyric_row0(ModelRc::new(VecModel::from(r0)));
            ui.set_lyric_row1(ModelRc::new(VecModel::from(r1)));
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
        Change::Words { spoken, fill } => {
            // Kelime sırası iki satır boyunca sürer; yalnız değişen kelimeler güncellenir.
            let mut k = 0;
            for m in [ui.get_lyric_row0(), ui.get_lyric_row1()] {
                for j in 0..m.row_count() {
                    if let Some(mut w) = m.row_data(j) {
                        let (state, f) = word_state(k, spoken, fill);
                        if w.state != state || (w.fill - f).abs() > 0.01 {
                            w.state = state;
                            w.fill = f;
                            m.set_row_data(j, w);
                        }
                    }
                    k += 1;
                }
            }
        }
    }
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
            st.player.send(Cmd::Load { url: path.to_string_lossy().into_owned(), title: media_title(&t) });
            st.loaded = true;
        } else if let Some(r) = st.resolver.cached(&key) {
            dbg(&format!("çal {key}: önceden çözülmüş"));
            st.player.send(Cmd::Load { url: r.stream, title: media_title(&t) });
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
                st.player.send(Cmd::Load { url: r.stream, title: media_title(&t) });
                st.loaded = true;
            }
            Err(e) => {
                eprintln!("çözülemedi ({key}): {e}; yedek yol");
                st.fallback_tried = true;
                if let Some(page) = page_url(st, &t) {
                    st.player.send(Cmd::Load { url: page, title: media_title(&t) });
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
    }
}

/// Parça bitti. Sıradaki eklenmişse mpv ona kendisi geçer (on_started); değilse biz geçeriz.
fn on_ended(ui: &AppWindow) {
    let has_next = with_state(|st| st.index.is_some_and(|i| st.appended == Some(i + 1)));
    if !has_next {
        step(ui, 1);
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
                st.player.send(Cmd::Load { url: page, title: media_title(&t) });
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
            with_state(|st| {
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
            if ui.get_has_now() {
                with_state(|st| st.mpris.send(mpris::Msg::Playing(!p)));
            }
            ui.set_playing(!p && ui.get_has_now());
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
                    q.extend(list.into_iter().filter(|t| t.key() != seed2.key()));
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
                let new: Vec<TrackInfo> = list.into_iter().filter(|t| !have.contains(&t.key()) && !skipped.contains(&t.key())).take(25).collect();
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
        with_state(|st| {
            st.queue.remove(i);
            st.index = None;
        });
        if with_state(|st| i < st.queue.len()) {
            play_index(ui, i);
            queue_changed(ui);
        } else {
            queue_changed(ui);
        }
        return;
    }
    with_state(|st| {
        if i < st.queue.len() {
            st.queue.remove(i);
            if let Some(cur) = st.index {
                if i < cur {
                    st.index = Some(cur - 1);
                }
            }
        }
    });
    queue_changed(ui);
}

fn queue_move(ui: &AppWindow, i: usize, delta: i32) {
    let ok = with_state(|st| {
        let j = i as i64 + delta as i64;
        if j < 0 || j as usize >= st.queue.len() || i >= st.queue.len() {
            return false;
        }
        let j = j as usize;
        st.queue.swap(i, j);
        st.index = st.index.map(|c| if c == i { j } else if c == j { i } else { c });
        true
    });
    if ok {
        queue_changed(ui);
    }
}

fn queue_clear(ui: &AppWindow) {
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

fn toggle_like_current(ui: &AppWindow) {
    let liked = with_state(|st| {
        let t = st.index.and_then(|i| st.queue.get(i).cloned())?;
        Some(st.history.toggle_like(&t))
    });
    if let Some(l) = liked {
        ui.set_liked(l);
        toast(ui, if l { "Beğendiklerine eklendi" } else { "Beğendiklerinden çıkarıldı" });
    }
}

// ---------- Sağ tık menüsü ----------

/// Menünün hedef şarkısı: kind 0 raf (a, b), 1 arama sonucu, 2 kayıtlı liste, 3 kuyruk, 4 açık koleksiyon.
fn menu_target(st: &State, kind: i32, a: i32, b: i32) -> Option<TrackInfo> {
    let (a, b) = (a as usize, b as usize);
    match kind {
        0 => st.shelves.get(a).and_then(|s| s.items.get(b)).cloned(),
        1 => st.results.get(a).cloned(),
        2 => st.current_pl.and_then(|p| st.playlists.get(p)).and_then(|p| p.tracks.get(a)).cloned(),
        3 => st.queue.get(a).cloned(),
        4 => st.coll.as_ref().and_then(|c| c.1.get(a)).cloned(),
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
        4 => {
            let l = with_state(|st| st.history.toggle_like(&t));
            if ui.get_now().key == t.key().as_str() {
                ui.set_liked(l);
            }
            toast(ui, if l { "Beğendiklerine eklendi" } else { "Beğendiklerinden çıkarıldı" });
        }
        _ => {}
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

/// Ana sayfayı diz: kişisel, kaynak ve keşfet raflarını iç içe geçirerek.
fn compose(st: &State, h: &HomeData) -> Vec<Section> {
    let take = |list: &[Section], pred: &dyn Fn(&Section) -> bool| -> Vec<Section> { list.iter().filter(|s| pred(s)).cloned().collect() };
    let p = &h.personal;
    let is = |t: &'static str| move |s: &Section| s.title == t;
    let mut out = Vec::new();
    let quick = recommend::quick_picks(&st.history, p, &h.external);
    if !quick.is_empty() {
        out.push(Section { title: "Hızlı erişim".into(), items: quick, cards: Vec::new(), style: 1 });
    }
    out.extend(take(p, &|s| s.title.ends_with("için karışım")));
    out.extend(take(p, &is("Kaldığın yerden")));
    if !st.playlists.is_empty() {
        let cards = st.playlists.iter().enumerate().map(|(i, pl)| Card {
            kind: "saved".into(),
            key: format!("saved-{i}"),
            title: pl.name.clone(),
            sub: format!("{} şarkı", pl.tracks.len()),
            thumb: pl.tracks.first().map(|t| t.thumb.clone()).unwrap_or_default(),
        }).collect();
        out.push(Section { title: "Listelerim".into(), items: Vec::new(), cards, style: 2 });
    }
    let ext_tracks = take(&h.external, &|s| s.style == 0);
    let ext_cards = take(&h.external, &|s| s.style != 0);
    out.extend(ext_tracks);
    out.extend(take(&h.explore, &is("Ruh hâli ve türler")));
    out.extend(take(p, &|s| s.title.ends_with(" ve benzerleri")));
    out.extend(take(&h.explore, &is("Trend şarkılar")));
    out.extend(ext_cards.iter().take(2).cloned());
    out.extend(take(p, &is("Yeni keşifler")));
    out.extend(take(&h.explore, &is("Yeni albümler")));
    out.extend(ext_cards.iter().skip(2).take(2).cloned());
    out.extend(take(&h.explore, &is("Türkiye'de popüler sanatçılar")));
    out.extend(take(p, &is("En çok dinlediklerin")));
    out.extend(take(p, &is("Beğendiklerin")));
    out.extend(ext_cards.iter().skip(4).cloned());
    out
}

/// Ana sayfayı göster.
fn show_home(ui: &AppWindow, data: HomeData) {
    let all: Vec<Section> = with_state(|st| {
        let all = compose(st, &data);
        st.home = data;
        st.mix = all.iter().find(|s| s.title.ends_with("için karışım")).map(|s| s.items.clone()).unwrap_or_default();
        for s in &all {
            for t in s.items.iter().take(if s.style == 1 { 16 } else { 8 }) {
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
    let (providers, resolver, hist_seeds, history_snapshot) = with_state(|st| {
        (st.providers.clone(), st.resolver.clone(), recommend::seeds(&st.history), st.history.snapshot())
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
        // Geçmiş yoksa tohumlar YouTube Music'in sana özel önerilerinden.
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
        let personal = recommend::build(&history_snapshot, &seeds, &radios);
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

fn open_card(ui: &AppWindow, c: Card) {
    if c.kind == "saved" {
        if let Some(i) = c.key.strip_prefix("saved-").and_then(|x| x.parse::<usize>().ok()) {
            open_playlist(ui, i);
            ui.set_view("playlist".into());
        }
        return;
    }
    let mood = c.kind == "mood";
    ui.set_coll_loading(true);
    if mood {
        ui.set_browse_title(c.title.as_str().into());
        ui.set_browse_cards(ModelRc::new(VecModel::from(Vec::<CardUi>::new())));
        ui.set_view("browse".into());
    } else {
        ui.set_coll_title(c.title.as_str().into());
        ui.set_coll_sub(c.sub.as_str().into());
        ui.set_coll_kind(card_kind(&c.kind));
        ui.set_coll_tracks(ModelRc::new(VecModel::from(Vec::<Track>::new())));
        ui.set_view("collection".into());
    }
    let resolver = with_state(|st| st.resolver.clone());
    let weak = ui.as_weak();
    let req = serde_json::json!({"op": "collection", "kind": c.kind, "key": c.key});
    resolver.request(req, Prio::Now, Box::new(move |res| {
        let _ = weak.upgrade_in_event_loop(move |ui| {
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
                        request_card_art(st, c);
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
                        request_art(st, t);
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
        st.playlists.push(library::Playlist { name: ui.get_coll_title().to_string(), origin: format!("ytm:{}", c.key), tracks });
        let _ = library::save(&st.playlists);
        Some(st.playlists.len())
    });
    if saved.is_some() {
        refresh_playlists(ui);
        toast(ui, &format!("“{}” Listelerim'e kaydedildi", ui.get_coll_title()));
    }
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
    s.starts_with('/') || s.starts_with("~/") || s.starts_with("file://")
}

/// Sistem dosya seçicisini aç (zenity, yoksa kdialog); seçilen yolla aktar.
fn pick_file(ui: &AppWindow) {
    let weak = ui.as_weak();
    thread::spawn(move || {
        let tries: [(&str, &[&str]); 2] = [
            ("zenity", &["--file-selection", "--title=Liste dosyası seç", "--file-filter=Listeler | *.json *.csv", "--file-filter=Tümü | *"]),
            ("kdialog", &["--getopenfilename", ".", "*.json *.csv"]),
        ];
        let mut path = None;
        let mut found = false;
        for (cmd, args) in tries {
            match std::process::Command::new(cmd).args(args).output() {
                Ok(o) => {
                    found = true;
                    let p = String::from_utf8_lossy(&o.stdout).trim().to_string();
                    if o.status.success() && !p.is_empty() {
                        path = Some(p);
                    }
                    break;
                }
                Err(_) => continue,
            }
        }
        let _ = weak.upgrade_in_event_loop(move |ui| match path {
            Some(p) => start_file_import(&ui, p),
            None if !found => toast(&ui, "Dosya seçici yok; dosya yolunu kutuya yazıp Enter'a bas."),
            None => {}
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
        let ytm = st.providers.iter().find(|p| p.source() == Source::Ytm).cloned();
        let sc = st.providers.iter().find(|p| p.source() == Source::SoundCloud).cloned();
        ytm.zip(sc)
    });
    let Some((ytm, sc)) = providers else { return };
    ui.set_import_busy(true);
    ui.set_import_done(0);
    ui.set_import_total(0);
    ui.set_import_filter(-1);
    let weak = ui.as_weak();
    thread::spawn(move || {
        let w = Mutex::new(weak.clone());
        let res = import::run(&url, ytm, sc, |done, total| {
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
        st.playlists.push(library::Playlist { name: f.name.clone(), origin: st.import_origin.clone(), tracks });
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

fn refresh_playlists(ui: &AppWindow) {
    let data = with_state(|st| (!st.home.personal.is_empty() || !st.home.external.is_empty()).then(|| st.home.clone()));
    if let Some(d) = data {
        show_home(ui, d);
    }
    let infos: Vec<PlaylistInfo> = with_state(|st| {
        st.playlists.iter().map(|p| PlaylistInfo { name: p.name.as_str().into(), count: p.tracks.len() as i32 }).collect()
    });
    ui.set_playlists(ModelRc::new(VecModel::from(infos)));
}

fn open_playlist(ui: &AppWindow, i: usize) {
    let tracks = with_state(|st| {
        let p = st.playlists.get(i)?.tracks.clone();
        st.current_pl = Some(i);
        for t in p.iter().take(40) {
            request_art(st, t);
        }
        for t in p.iter().take(2) {
            speculate(st, t);
        }
        Some(p)
    });
    let Some(tracks) = tracks else { return };
    ui.set_current_playlist(i as i32);
    ui.set_playlist_tracks(model_of(&tracks));
}

fn delete_playlist(ui: &AppWindow, i: usize) {
    with_state(|st| {
        if i < st.playlists.len() {
            st.playlists.remove(i);
            st.current_pl = None;
            if let Err(e) = library::save(&st.playlists) {
                eprintln!("kütüphane kaydedilemedi: {e}");
            }
        }
    });
    ui.set_current_playlist(-1);
    refresh_playlists(ui);
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

/// Spektrumu yalnız profil ekranı açıkken ve çalarken çalıştır.
fn update_viz(ui: &AppWindow) {
    let want = ui.get_view() == "profile" && ui.get_playing();
    let running = with_state(|st| st.viz.is_some());
    if want == running {
        return;
    }
    if !want {
        with_state(|st| st.viz = None); // Drop cava'yı kapatır
        ui.set_viz_on(false);
        ui.set_spectrum(ModelRc::new(VecModel::from(vec![0.0f32; viz::BARS])));
        return;
    }
    ui.set_spectrum(ModelRc::new(VecModel::from(vec![0.0f32; viz::BARS])));
    let weak = ui.as_weak();
    match viz::Viz::start(move |frame| {
        let _ = weak.upgrade_in_event_loop(move |ui| {
            let m = ui.get_spectrum();
            for (i, v) in frame.into_iter().enumerate() {
                m.set_row_data(i, v);
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
    ui.set_autoradio_on(s.autoradio);
    ui.set_lyrics_style(s.lyrics_style);
    ui.set_lyrics_size(s.lyrics_size);
    ui.set_lyrics_align(s.lyrics_align);
    ui.set_lyrics_motion(s.lyrics_motion);
    ui.set_keep_index(s.keep_index());
}

fn apply_settings(ui: &AppWindow) {
    let s = Settings {
        cache_gb: settings::CACHE_GB[ui.get_cache_index().clamp(0, 4) as usize],
        prefetch: ui.get_prefetch_on(),
        readahead_secs: settings::READAHEAD_SECS[ui.get_readahead_index().clamp(0, 3) as usize],
        cookies_browser: settings::COOKIE_BROWSERS[ui.get_cookies_index().clamp(0, 1) as usize].map(str::to_string),
        download_kbps: settings::DOWNLOAD_KBPS[ui.get_download_index().clamp(0, 3) as usize],
        autoradio: ui.get_autoradio_on(),
        lyrics_style: ui.get_lyrics_style(),
        lyrics_size: ui.get_lyrics_size(),
        lyrics_align: ui.get_lyrics_align(),
        lyrics_motion: ui.get_lyrics_motion(),
        keep_days: settings::KEEP_DAYS[ui.get_keep_index().clamp(0, 3) as usize],
    };
    with_state(|st| {
        st.player.send(Cmd::Readahead(s.readahead_secs));
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

/// MPRIS (medya tuşları, çubuk) komutlarını uygula.
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
        Act::Seek(off) => {
            let p = (ui.get_position() as f64 + off).max(0.0);
            with_state(|st| st.player.send(Cmd::Seek(p)));
        }
        Act::SetPosition(p) => with_state(|st| st.player.send(Cmd::Seek(p.max(0.0)))),
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
            imported: None,
            import_origin: String::new(),
            playlists: library::load(),
            current_pl: None,
            lyrics: HashMap::new(),
            lyr_pending: HashSet::new(),
            lyr_line: None,
            lyr_spoken: 0,
            lyr_offsets: HashMap::new(),
            history: history::History::load(),
            listen: None,
            shuffle_orig: None,
            repeat: 0,
            radio_busy: false,
            menu: None,
            mix: Vec::new(),
            coll: None,
            browse: Vec::new(),
            home: HomeData::default(),
            viz: None,
            prof_tracks: Vec::new(),
            file_lists: Vec::new(),
            mpris: mpris_server.clone(),
            seek_pending: false,
        })
    });
    refresh_playlists(&ui);

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
            if v == "profile" {
                refresh_profile(&ui);
            }
            update_viz(&ui);
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
    ui.on_menu_for({
        let weak = weak.clone();
        move |kind, a, b| {
            let Some(ui) = weak.upgrade() else { return };
            let t = with_state(|st| {
                let t = menu_target(st, kind, a, b);
                st.menu = t.clone();
                t.map(|t| (t.title.clone(), st.history.is_liked(&t.key())))
            });
            match t {
                Some((title, liked)) => {
                    ui.set_menu_title(title.into());
                    ui.set_menu_liked(liked);
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
    ui.on_lyric_seek({
        let weak = weak.clone();
        move |t| {
            let Some(ui) = weak.upgrade() else { return };
            let off = with_state(|st| st.lyr_offsets.get(&ui.get_now().key.to_string()).copied().unwrap_or(0.0));
            with_state(|st| st.player.send(Cmd::Seek(((t - off).max(0.0)) as f64)));
        }
    });
    ui.on_lyrics_style_changed({
        let weak = weak.clone();
        move || {
            let Some(ui) = weak.upgrade() else { return };
            apply_settings(&ui);
            // Boyut değişince satır bölme yeniden hesaplansın
            with_state(|st| st.lyr_spoken = usize::MAX);
            update_lyrics(&ui, ui.get_position());
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
                st.lyr_line = None;
                st.lyr_spoken = usize::MAX;
                *o
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
            let list = with_state(|st| st.current_pl.and_then(|i| st.playlists.get(i)).map(|p| p.tracks.clone()));
            if let Some(list) = list {
                play_list(&ui, list, j as usize);
            }
        }
    });
    ui.on_delete_playlist({
        let weak = weak.clone();
        move |i| {
            if let Some(ui) = weak.upgrade() {
                delete_playlist(&ui, i as usize);
                toast(&ui, "Liste silindi");
            }
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

    refresh_usage(&ui);
    load_home(&ui);
    ui.run()?;
    with_state(|st| st.viz = None);
    start_listen(None); // son dinlemeyi kaydet
    resolver.shutdown();
    Ok(())
}
