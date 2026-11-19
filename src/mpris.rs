// Sistem medya kontrolleri: Linux'ta MPRIS (mpris-server), Windows'ta SMTC (souvlaki).
// Arayüz güncellemeleri kanalla gelir, medya tuşu komutları `on_act` ile arayüze gider.

#[allow(dead_code)]
pub enum Msg {
    Track { key: String, title: String, artist: String, album: String, art: String, length: f64 },
    Length(f64),
    Playing(bool),
    Position(f64),
    Seeked(f64),
    Caps { next: bool, prev: bool },
    Volume(f64),
    Shuffle(bool),
    Loop(i32),
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub enum Act {
    PlayPause,
    Play,
    Pause,
    Next,
    Prev,
    /// Göreli ileri/geri (sn)
    Seek(f64),
    /// Mutlak konum (sn)
    SetPosition(f64),
    Volume(f64),
    Shuffle(bool),
    /// 0 kapalı, 1 liste, 2 tek şarkı
    Loop(i32),
}

#[cfg(target_os = "linux")]
mod imp {
    use std::thread;

    use mpris_server::{LoopStatus, Metadata, PlaybackStatus, Player, Time, TrackId};

    use super::{Act, Msg};

    #[derive(Clone)]
    pub struct Mpris {
        tx: async_channel::Sender<Msg>,
    }

    fn track_id(key: &str) -> TrackId {
        let safe: String = key.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
        TrackId::try_from(format!("/org/vynz/track/t{safe}").as_str()).unwrap_or(TrackId::NO_TRACK)
    }

    fn secs(t: f64) -> Time {
        Time::from_micros((t.max(0.0) * 1_000_000.0) as i64)
    }

    impl Mpris {
        pub fn start(on_act: impl Fn(Act) + Clone + Send + 'static) -> Mpris {
            let (tx, rx) = async_channel::unbounded::<Msg>();
            thread::Builder::new()
                .name("vynz-mpris".into())
                .spawn(move || {
                    async_io::block_on(async move {
                        let build = |name: String| {
                            Player::builder(&name)
                                .identity("Vynz")
                                .desktop_entry("vynz")
                                .can_play(true)
                                .can_pause(true)
                                .can_seek(true)
                                .can_control(true)
                                .can_go_next(false)
                                .can_go_previous(false)
                                .playback_status(PlaybackStatus::Stopped)
                                .build()
                        };
                        // Ad doluysa (ikinci kopya) işleme özel ad al.
                        let player = match build("vynz".into()).await {
                            Ok(p) => p,
                            Err(_) => match build(format!("vynz.instance{}", std::process::id())).await {
                                Ok(p) => p,
                                Err(e) => {
                                    eprintln!("MPRIS başlatılamadı: {e}");
                                    return;
                                }
                            },
                        };
                        let a = on_act.clone();
                        player.connect_play_pause(move |_| a(Act::PlayPause));
                        let a = on_act.clone();
                        player.connect_play(move |_| a(Act::Play));
                        let a = on_act.clone();
                        player.connect_pause(move |_| a(Act::Pause));
                        let a = on_act.clone();
                        player.connect_stop(move |_| a(Act::Pause));
                        let a = on_act.clone();
                        player.connect_next(move |_| a(Act::Next));
                        let a = on_act.clone();
                        player.connect_previous(move |_| a(Act::Prev));
                        let a = on_act.clone();
                        player.connect_seek(move |_, off| a(Act::Seek(off.as_micros() as f64 / 1e6)));
                        let a = on_act.clone();
                        player.connect_set_position(move |_, _, pos| a(Act::SetPosition(pos.as_micros() as f64 / 1e6)));
                        let a = on_act.clone();
                        player.connect_set_volume(move |_, v| a(Act::Volume(v)));
                        let a = on_act.clone();
                        player.connect_set_shuffle(move |_, s| a(Act::Shuffle(s)));
                        let a = on_act.clone();
                        player.connect_set_loop_status(move |_, l| {
                            a(Act::Loop(match l {
                                LoopStatus::None => 0,
                                LoopStatus::Playlist => 1,
                                LoopStatus::Track => 2,
                            }))
                        });

                        let apply = async {
                            let mut meta = Metadata::new();
                            while let Ok(m) = rx.recv().await {
                                let r = match m {
                                    Msg::Track { key, title, artist, album, art, length } => {
                                        let mut b = Metadata::builder().trackid(track_id(&key)).title(title).artist([artist]).album(album);
                                        if !art.is_empty() {
                                            b = b.art_url(art);
                                        }
                                        if length > 0.0 {
                                            b = b.length(secs(length));
                                        }
                                        meta = b.build();
                                        player.set_position(Time::ZERO);
                                        player.set_metadata(meta.clone()).await
                                    }
                                    Msg::Length(l) => {
                                        if l > 0.0 && meta.length() != Some(secs(l)) {
                                            meta.set_length(Some(secs(l)));
                                            player.set_metadata(meta.clone()).await
                                        } else {
                                            Ok(())
                                        }
                                    }
                                    Msg::Playing(p) => player.set_playback_status(if p { PlaybackStatus::Playing } else { PlaybackStatus::Paused }).await,
                                    Msg::Position(p) => {
                                        player.set_position(secs(p));
                                        Ok(())
                                    }
                                    Msg::Seeked(p) => {
                                        player.set_position(secs(p));
                                        player.seeked(secs(p)).await
                                    }
                                    Msg::Caps { next, prev } => {
                                        let a = player.set_can_go_next(next).await;
                                        a.and(player.set_can_go_previous(prev).await)
                                    }
                                    Msg::Volume(v) => player.set_volume(v).await,
                                    Msg::Shuffle(s) => player.set_shuffle(s).await,
                                    Msg::Loop(l) => {
                                        player
                                            .set_loop_status(match l {
                                                1 => LoopStatus::Playlist,
                                                2 => LoopStatus::Track,
                                                _ => LoopStatus::None,
                                            })
                                            .await
                                    }
                                };
                                if let Err(e) = r {
                                    eprintln!("MPRIS: {e}");
                                }
                            }
                        };
                        futures_lite::future::or(player.run(), apply).await;
                    });
                })
                .ok();
            Mpris { tx }
        }

        pub fn send(&self, m: Msg) {
            let _ = self.tx.try_send(m);
        }
    }
}

#[cfg(target_os = "windows")]
mod imp {
    use std::ffi::c_void;
    use std::sync::mpsc::{self, Sender, TryRecvError};
    use std::thread;
    use std::time::Duration;

    use souvlaki::{MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig, SeekDirection};
    use windows::core::w;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE, WINDOW_EX_STYLE, WS_OVERLAPPED,
    };

    use super::{Act, Msg};

    #[derive(Clone)]
    pub struct Mpris {
        tx: Sender<Msg>,
    }

    struct TrackState {
        title: String,
        artist: String,
        album: String,
        art: String,
        length: f64,
        playing: bool,
        pos: f64,
    }

    impl TrackState {
        fn new() -> Self {
            Self { title: String::new(), artist: String::new(), album: String::new(), art: String::new(), length: 0.0, playing: false, pos: 0.0 }
        }

        fn apply_meta(&self, controls: &mut MediaControls) {
            let _ = controls.set_metadata(MediaMetadata {
                title: (!self.title.is_empty()).then_some(self.title.as_str()),
                artist: (!self.artist.is_empty()).then_some(self.artist.as_str()),
                album: (!self.album.is_empty()).then_some(self.album.as_str()),
                cover_url: (!self.art.is_empty()).then_some(self.art.as_str()),
                duration: (self.length > 0.0).then(|| Duration::from_secs_f64(self.length)),
            });
        }

        fn apply_playback(&self, controls: &mut MediaControls) {
            let progress = Some(MediaPosition(Duration::from_secs_f64(self.pos.max(0.0))));
            let pb = if self.playing { MediaPlayback::Playing { progress } } else { MediaPlayback::Paused { progress } };
            let _ = controls.set_playback(pb);
        }
    }

    fn create_hidden_window() -> Option<HWND> {
        unsafe {
            let hinst = GetModuleHandleW(None).ok()?;
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!("Vynz"),
                WS_OVERLAPPED,
                0,
                0,
                0,
                0,
                None,
                None,
                hinst,
                None,
            )
            .ok()?;
            Some(hwnd)
        }
    }

    impl Mpris {
        pub fn start(on_act: impl Fn(Act) + Clone + Send + 'static) -> Mpris {
            let (tx, rx) = mpsc::channel::<Msg>();
            thread::Builder::new()
                .name("vynz-smtc".into())
                .spawn(move || {
                    let Some(hwnd) = create_hidden_window() else {
                        return;
                    };
                    let cfg = PlatformConfig {
                        dbus_name: "vynz",
                        display_name: "Vynz",
                        hwnd: Some(hwnd.0 as *mut c_void),
                    };
                    let Ok(mut controls) = MediaControls::new(cfg) else {
                        unsafe {
                            let _ = DestroyWindow(hwnd);
                        }
                        return;
                    };
                    let a = on_act.clone();
                    let _ = controls.attach(move |ev| match ev {
                        MediaControlEvent::Play => a(Act::Play),
                        MediaControlEvent::Pause | MediaControlEvent::Stop => a(Act::Pause),
                        MediaControlEvent::Toggle => a(Act::PlayPause),
                        MediaControlEvent::Next => a(Act::Next),
                        MediaControlEvent::Previous => a(Act::Prev),
                        MediaControlEvent::SetPosition(MediaPosition(d)) => a(Act::SetPosition(d.as_secs_f64())),
                        MediaControlEvent::SeekBy(dir, d) => {
                            let s = d.as_secs_f64();
                            a(Act::Seek(if dir == SeekDirection::Forward { s } else { -s }));
                        }
                        MediaControlEvent::SetVolume(v) => a(Act::Volume(v)),
                        _ => {}
                    });

                    let mut st = TrackState::new();
                    let mut wmsg = MSG::default();
                    loop {
                        loop {
                            match rx.try_recv() {
                                Ok(m) => match m {
                                    Msg::Track { title, artist, album, art, length, .. } => {
                                        st.title = title;
                                        st.artist = artist;
                                        st.album = album;
                                        st.art = art;
                                        st.length = length;
                                        st.pos = 0.0;
                                        st.apply_meta(&mut controls);
                                        st.apply_playback(&mut controls);
                                    }
                                    Msg::Length(l) => {
                                        if l > 0.0 && (st.length - l).abs() > 0.5 {
                                            st.length = l;
                                            st.apply_meta(&mut controls);
                                        }
                                    }
                                    Msg::Playing(p) => {
                                        st.playing = p;
                                        st.apply_playback(&mut controls);
                                    }
                                    Msg::Position(p) | Msg::Seeked(p) => {
                                        st.pos = p;
                                        st.apply_playback(&mut controls);
                                    }
                                    Msg::Caps { .. } | Msg::Volume(_) | Msg::Shuffle(_) | Msg::Loop(_) => {}
                                },
                                Err(TryRecvError::Empty) => break,
                                Err(TryRecvError::Disconnected) => {
                                    unsafe {
                                        let _ = DestroyWindow(hwnd);
                                    }
                                    return;
                                }
                            }
                        }
                        unsafe {
                            while PeekMessageW(&mut wmsg, hwnd, 0, 0, PM_REMOVE).as_bool() {
                                let _ = TranslateMessage(&wmsg);
                                DispatchMessageW(&wmsg);
                            }
                        }
                        thread::sleep(Duration::from_millis(50));
                    }
                })
                .ok();
            Mpris { tx }
        }

        pub fn send(&self, m: Msg) {
            let _ = self.tx.send(m);
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
mod imp {
    use super::{Act, Msg};

    #[derive(Clone)]
    pub struct Mpris;

    impl Mpris {
        pub fn start(_on_act: impl Fn(Act) + Clone + Send + 'static) -> Mpris {
            Mpris
        }
        pub fn send(&self, _m: Msg) {}
    }
}

pub use imp::Mpris;
