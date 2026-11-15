// Kendi MPRIS sunucumuz: Quickshell çubuğu, medya tuşları ve playerctl için.
// mpv-mpris yerine: başlık/sanatçı/albüm/kapak/süre ayrı ayrı, önceki/sonraki Vynz kuyruğuna bağlı.
//
// mpris-server'ın Player'ı tek iş parçacıklıdır (Rc); kendi iş parçacığında yerel async
// döngüsünde çalışır. Arayüz güncellemeleri kanalla gelir, komutlar `on_act` ile arayüze gider.

use std::thread;

use mpris_server::{LoopStatus, Metadata, PlaybackStatus, Player, Time, TrackId};

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
