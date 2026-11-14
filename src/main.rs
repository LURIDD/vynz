// vynz: çok kaynaklı müzik çalar. 2. aşama: kaynak yapısı, YouTube Music, libmpv ile gerçek ses.

mod art;
mod model;
mod player;
mod sources;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

use slint::{Color, ComponentHandle, Image, Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, Timer, VecModel, Weak};

use model::{Section, Source, TrackInfo};
use player::{Cmd, Ev, Player};
use sources::{soundcloud::SoundCloud, ytm::Ytm, Provider};

slint::include_modules!();

const NEUTRAL: Color = Color::from_rgb_u8(0xE4, 0xE6, 0xEB);

struct State {
    providers: Vec<Arc<dyn Provider>>,
    player: Player,
    art_tx: mpsc::Sender<(String, String)>,
    shelves: Vec<Section>,
    results: Vec<TrackInfo>,
    queue: Vec<TrackInfo>,
    index: Option<usize>,
    search_seq: u64,
    requested: HashSet<String>,
    toast_timer: Timer,
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
        st.toast_timer.start(slint::TimerMode::SingleShot, Duration::from_secs(4), move || {
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

fn play_index(ui: &AppWindow, i: usize) {
    let res = with_state(|st| {
        let t = st.queue.get(i)?.clone();
        st.index = Some(i);
        let p = provider(st, t.source)?;
        request_art(st, &t);
        Some((t.clone(), p.stream_url(&t), st.player.clone()))
    });
    let Some((t, url, player)) = res else { return };
    match url {
        Ok(url) => player.send(Cmd::Load(url)),
        Err(e) => {
            toast(ui, &format!("Açılamadı: {e}"));
            return;
        }
    }
    ui.set_has_now(true);
    ui.set_now(ui_track(&t));
    ui.set_queue_index(i as i32);
    ui.set_position(0.0);
    ui.set_duration(t.duration);
    ui.set_playing(true);
    ui.set_buffering(true);
    let accent = ARTS.with(|a| a.borrow().get(&t.key()).map(|(_, c)| *c)).unwrap_or(NEUTRAL);
    ui.global::<Theme>().set_accent(accent);
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

fn on_player_event(ui: &AppWindow, ev: Ev) {
    match ev {
        Ev::Time(t) => ui.set_position(t as f32),
        Ev::Duration(d) => ui.set_duration(d as f32),
        Ev::Paused(p) => ui.set_playing(!p && ui.get_has_now()),
        Ev::Buffering(b) => ui.set_buffering(b),
        Ev::Ended => step(ui, 1),
        Ev::Error(e) => {
            toast(ui, &format!("Çalınamadı: {e}"));
            step(ui, 1);
        }
    }
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
                Err(e) => errors.push(e),
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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let ui = AppWindow::new()?;
    if let Ok(t) = std::env::var("VYNZ_TITLE") {
        ui.set_window_title(t.into()); // testte pencereyi ayırt etmek için
    }
    let weak = ui.as_weak();

    let player = {
        let weak = weak.clone();
        Player::spawn(move |ev| {
            let _ = weak.upgrade_in_event_loop(move |ui| on_player_event(&ui, ev));
        })?
    };
    player.send(Cmd::Volume(ui.get_volume() as f64));

    STATE.with(|s| {
        *s.borrow_mut() = Some(State {
            providers: vec![Arc::new(Ytm), Arc::new(SoundCloud)],
            player,
            art_tx: start_art_pool(weak.clone()),
            shelves: Vec::new(),
            results: Vec::new(),
            queue: Vec::new(),
            index: None,
            search_seq: 0,
            requested: HashSet::new(),
            toast_timer: Timer::default(),
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

    load_home(&ui);
    ui.run()?;
    Ok(())
}
