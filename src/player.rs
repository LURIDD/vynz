// Ses: süreç içi libmpv. Kendi mpv örneği; kullanıcının ymus/mpv'sine dokunmaz.
//
// Hız ve bant genişliği:
// - Çözülmüş doğrudan adresler çalınır (yt-dlp kancası yalnız yedek).
// - Sıradaki parça mpv listesine eklenir; prefetch-playlist, çalan parçanın verisi
//   bittiğinde (önce çalan gelir) sıradakini açmaya başlar: geçiş anında olur.
// - cache-secs ileri okumayı sınırlar: ilk tampondan sonra indirme çalma hızına iner.
// - stream-record ile çalınan veri diske de yazılır (ek indirme yok).

use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use libmpv2::{events::*, mpv_end_file_reason, Format, Mpv};

pub enum Cmd {
    /// Hemen çal (mpv listesindeki sıradakini de temizler).
    Load { url: String, record: Option<String> },
    /// Sıradaki olarak ekle (ön-tampon).
    Append { url: String, record: Option<String> },
    /// Eklenmiş sıradakini kaldır.
    ClearNext,
    SetPause(bool),
    Seek(f64),
    Volume(f64),
    /// İleri okuma sınırı (saniye; 0 = sınırsız).
    Readahead(u32),
}

#[derive(Debug)]
pub enum Ev {
    Time(f64),
    Duration(f64),
    Paused(bool),
    Buffering(bool),
    /// Yeni dosya başladı.
    Started,
    /// Kullanıcı ya da program konum değiştirdi (kayıt artık eksiksiz değil).
    Seeked,
    /// Dosya sonuna ulaşıldı.
    Ended,
    Error(String),
}

#[derive(Clone)]
pub struct Player {
    tx: Sender<Cmd>,
}

impl Player {
    pub fn spawn(readahead: u32, on_event: impl Fn(Ev) + Send + 'static) -> Result<Player, String> {
        let (tx, rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();
        thread::Builder::new()
            .name("vynz-mpv".into())
            .spawn(move || run(rx, readahead, on_event, ready_tx))
            .map_err(|e| e.to_string())?;
        ready_rx.recv().map_err(|e| e.to_string())??;
        Ok(Player { tx })
    }

    pub fn send(&self, cmd: Cmd) {
        let _ = self.tx.send(cmd);
    }
}

fn readahead_value(secs: u32) -> String {
    if secs == 0 { "86400".into() } else { secs.to_string() }
}

fn loadfile(mpv: &Mpv, url: &str, mode: &str, record: &Option<String>) -> libmpv2::Result<()> {
    match record {
        // Dosyaya özel seçenek: yalnız bu parça kaydedilir.
        Some(path) => mpv.command("loadfile", &[url, mode, "-1", &format!("stream-record={path}")]),
        None => mpv.command("loadfile", &[url, mode]),
    }
}

fn run(rx: Receiver<Cmd>, readahead: u32, on_event: impl Fn(Ev), ready: Sender<Result<(), String>>) {
    let ao = std::env::var("VYNZ_AO").ok();
    let mpv = match Mpv::with_initializer(|i| {
        i.set_property("vo", "null")?;
        i.set_property("video", "no")?;
        i.set_property("ytdl", "yes")?;
        i.set_property("ytdl-format", "bestaudio/best")?;
        // Yedek yolda kullanıcının yt-dlp ayar dosyası (indirme ayarları) okunmasın.
        i.set_property("ytdl-raw-options", "ignore-config=")?;
        i.set_property("audio-client-name", "vynz")?;
        i.set_property("cache", "yes")?;
        i.set_property("cache-secs", readahead_value(readahead))?;
        i.set_property("demuxer-max-bytes", "64MiB")?;
        i.set_property("prefetch-playlist", "yes")?;
        if let Some(ao) = &ao {
            i.set_property("ao", ao.as_str())?;
        }
        Ok(())
    }) {
        Ok(m) => m,
        Err(e) => {
            let _ = ready.send(Err(format!("mpv başlatılamadı: {e}")));
            return;
        }
    };
    let events = match mpv.create_client(Some("vynz-events")) {
        Ok(c) => c,
        Err(e) => {
            let _ = ready.send(Err(e.to_string()));
            return;
        }
    };
    let _ = events.disable_deprecated_events();
    let _ = events.observe_property("time-pos", Format::Double, 0);
    let _ = events.observe_property("duration", Format::Double, 1);
    let _ = events.observe_property("pause", Format::Flag, 2);
    let _ = events.observe_property("paused-for-cache", Format::Flag, 3);
    let _ = ready.send(Ok(()));

    let mut last_time = -1.0f64;
    // Kendi verdiğimiz "seek" komutu da Seek olayı üretir; kullanıcı atlamasıyla aynı sayılır.
    loop {
        loop {
            match rx.try_recv() {
                Ok(Cmd::Load { url, record }) => {
                    last_time = -1.0;
                    let _ = mpv.command("playlist-clear", &[]);
                    let _ = mpv.set_property("pause", false);
                    if let Err(e) = loadfile(&mpv, &url, "replace", &record) {
                        on_event(Ev::Error(e.to_string()));
                    } else {
                        on_event(Ev::Buffering(true));
                    }
                }
                Ok(Cmd::Append { url, record }) => {
                    let _ = mpv.command("playlist-clear", &[]);
                    if let Err(e) = loadfile(&mpv, &url, "append", &record) {
                        eprintln!("sıradaki eklenemedi: {e}");
                    }
                }
                Ok(Cmd::ClearNext) => {
                    let _ = mpv.command("playlist-clear", &[]);
                }
                Ok(Cmd::SetPause(p)) => {
                    let _ = mpv.set_property("pause", p);
                }
                Ok(Cmd::Seek(s)) => {
                    let _ = mpv.command("seek", &[&format!("{s:.2}"), "absolute"]);
                }
                Ok(Cmd::Volume(v)) => {
                    let _ = mpv.set_property("volume", (v * 100.0).clamp(0.0, 100.0));
                }
                Ok(Cmd::Readahead(secs)) => {
                    let _ = mpv.set_property("cache-secs", readahead_value(secs));
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => return,
            }
        }
        match events.wait_event(0.05) {
            Some(Ok(Event::PropertyChange { name, change, .. })) => match (name, change) {
                ("time-pos", PropertyData::Double(t)) => {
                    if (t - last_time).abs() >= 0.2 {
                        last_time = t;
                        on_event(Ev::Time(t));
                    }
                }
                ("duration", PropertyData::Double(d)) => on_event(Ev::Duration(d)),
                ("pause", PropertyData::Flag(p)) => on_event(Ev::Paused(p)),
                ("paused-for-cache", PropertyData::Flag(b)) => on_event(Ev::Buffering(b)),
                _ => {}
            },
            Some(Ok(Event::StartFile)) => {
                last_time = -1.0;
                on_event(Ev::Started);
            }
            Some(Ok(Event::Seek)) => on_event(Ev::Seeked),
            Some(Ok(Event::PlaybackRestart)) => on_event(Ev::Buffering(false)),
            Some(Ok(Event::EndFile(r))) => {
                if r == mpv_end_file_reason::Eof {
                    on_event(Ev::Ended);
                } else if r == mpv_end_file_reason::Error {
                    on_event(Ev::Error("parça açılamadı".into()));
                }
            }
            Some(Ok(Event::Shutdown)) => return,
            _ => {}
        }
    }
}
