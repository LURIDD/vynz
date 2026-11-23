// Ses: süreç içi libmpv. Kendi mpv örnekleri; kullanıcının mpv'sine dokunmaz.
//
// Hız ve bant genişliği:
// - Çözülmüş doğrudan adresler çalınır (yt-dlp kancası yalnız yedek).
// - Sıradaki parça mpv listesine eklenir; prefetch-playlist, çalan parçanın verisi
//   bittiğinde (önce çalan gelir) sıradakini açmaya başlar: geçiş anında olur.
// - cache-secs ileri okumayı sınırlar: ilk tampondan sonra indirme çalma hızına iner.
//
// Çapraz geçiş (crossfade): mpv iki dosyayı üst üste çalamaz. Bu yüzden iki mpv örneği var
// (A/B). Etkin örnekte parça bitmeye `fade + 4` sn kala sıradaki öbür örnekte duraklatılmış
// açılır (tamponlansın), `fade` sn kala çalmaya başlar ve sesler `fade` boyunca karşılıklı
// kayar. Geçiş başında dışarıya `Started(1)` verilir (sanki mpv listesinde sıradakine
// geçilmiş gibi); eski örneğin olayları o andan sonra yok sayılır. Geçiş kapalıyken
// yalnız A kullanılır ve davranış öncekiyle aynıdır (boşluksuz ön-tampon).

use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Instant;

use libmpv2::{events::*, mpv_end_file_reason, Format, Mpv};

/// Ekolayzır bantları (Hz).
pub const EQ_BANDS: [u32; 10] = [32, 64, 125, 250, 500, 1000, 2000, 4000, 8000, 16000];

pub enum Cmd {
    /// Hemen çal (mpv listesindeki sıradakini de temizler).
    /// `title`: MPRIS'te (çubuk, medya tuşları) görünecek ad.
    Load { url: String, title: String, start: f64 },
    /// Sıradaki olarak ekle (ön-tampon).
    Append { url: String, title: String },
    /// Eklenmiş sıradakini kaldır.
    ClearNext,
    SetPause(bool),
    Seek(f64),
    Volume(f64),
    /// İleri okuma sınırı (saniye; 0 = sınırsız).
    Readahead(u32),
    /// Tek şarkıyı tekrarla.
    LoopFile(bool),
    /// Ses seviyesi dengeleme (dynaudnorm).
    Normalize(bool),
    /// Ekolayzır kazançları (dB, EQ_BANDS sırasıyla); None = kapalı.
    Eq(Option<[f32; 10]>),
    /// Çapraz geçiş süresi (sn); 0 = kapalı.
    Crossfade(f64),
}

#[derive(Debug)]
pub enum Ev {
    Time(f64),
    Duration(f64),
    Paused(bool),
    Buffering(bool),
    /// Yeni dosya başladı; mpv listesindeki sırası (0 = yüklediğimiz, 1 = eklenen sıradaki).
    Started(i64),
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

fn loadfile(mpv: &Mpv, url: &str, mode: &str, title: &str, start: f64) -> libmpv2::Result<()> {
    // Dosyaya özel seçenek; virgül ve eşittir seçenek ayırıcısı olduğundan temizlenir.
    let clean: String = title.chars().map(|c| if c == ',' || c == '=' { ' ' } else { c }).collect();
    let mut opts = format!("force-media-title=%{}%{clean}", clean.len());
    if start > 0.5 {
        opts.push_str(&format!(",start={start:.2}")); // kaldığı yerden
    }
    mpv.command("loadfile", &[url, mode, "-1", &opts])
}

/// mpv `af` dizgesi: dengeleme + ekolayzır (+ kırpılmasın diye ön kazanç düşürme).
pub fn filter_chain(normalize: bool, eq: Option<&[f32; 10]>) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(g) = eq.filter(|g| g.iter().any(|x| x.abs() >= 0.05)) {
        let peak = g.iter().cloned().fold(0.0f32, f32::max);
        if peak > 0.0 {
            parts.push(format!("volume={:.1}dB", -peak * 0.7));
        }
        for (f, gain) in EQ_BANDS.iter().zip(g.iter()) {
            if gain.abs() >= 0.05 {
                // 1 oktav genişliğinde tepe/çukur süzgeci
                parts.push(format!("equalizer=f={f}:t=o:w=1:g={gain:.1}"));
            }
        }
    }
    if normalize {
        parts.push("dynaudnorm=f=150:g=15".into());
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("lavfi=[{}]", parts.join(","))
    }
}

struct Inst {
    mpv: Mpv,
    ev: Mpv,
    last_time: f64,
    duration: f64,
}

fn make(readahead: u32, ao: &Option<String>, name: &str) -> Result<Inst, String> {
    let mpv = Mpv::with_initializer(|i| {
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
        // MPRIS'i Vynz kendisi sunar (mpris.rs); mpv'nin betik ve eklentileri yüklenmesin.
        i.set_property("load-scripts", "no")?;
        if let Some(ao) = ao {
            i.set_property("ao", ao.as_str())?;
        }
        Ok(())
    })
    .map_err(|e| format!("mpv başlatılamadı: {e}"))?;
    let ev = mpv.create_client(Some(name)).map_err(|e| e.to_string())?;
    let _ = ev.disable_deprecated_events();
    let _ = ev.observe_property("time-pos", Format::Double, 0);
    let _ = ev.observe_property("duration", Format::Double, 1);
    let _ = ev.observe_property("pause", Format::Flag, 2);
    let _ = ev.observe_property("paused-for-cache", Format::Flag, 3);
    Ok(Inst { mpv, ev, last_time: -1.0, duration: 0.0 })
}

/// Çapraz geçiş durumu.
enum Fade {
    None,
    /// Sıradaki öbür örnekte duraklatılmış açıldı, tamponlanıyor.
    Armed,
    /// Sesler kayıyor; `from` eski örnek.
    Running { from: usize, started: Instant },
}

struct Engine {
    inst: Vec<Inst>,
    /// Etkin örnek (olayları dışarı verilen)
    cur: usize,
    volume: f64,
    crossfade: f64,
    looping: bool,
    af: String,
    readahead: u32,
    ao: Option<String>,
    /// Etkin örneğe eklenmiş sıradaki (çapraz geçişte öbür örnekte açılır)
    next: Option<(String, String)>,
    fade: Fade,
    /// Kullanıcı duraklattı: geçiş hazırlanmaz/başlamaz.
    paused: bool,
}

impl Engine {
    fn other(&self) -> usize {
        1 - self.cur
    }

    /// İkinci örneği gerektiğinde aç (geçiş hiç kullanılmazsa bellekte yer kaplamasın).
    fn ensure_second(&mut self) -> bool {
        if self.inst.len() == 2 {
            return true;
        }
        match make(self.readahead, &self.ao, "vynz-events-b") {
            Ok(i) => {
                let _ = i.mpv.set_property("af", self.af.as_str());
                self.inst.push(i);
                true
            }
            Err(e) => {
                eprintln!("çapraz geçiş için ikinci mpv açılamadı: {e}");
                false
            }
        }
    }

    fn set_vol(&self, i: usize, v: f64) {
        if let Some(x) = self.inst.get(i) {
            let _ = x.mpv.set_property("volume", (v * 100.0).clamp(0.0, 100.0));
        }
    }

    /// Süren geçişi hemen bitir: eski örnek durur, yeni tam seste.
    fn finish_fade(&mut self) {
        match std::mem::replace(&mut self.fade, Fade::None) {
            Fade::Running { from, .. } => {
                let _ = self.inst[from].mpv.command("stop", &[]);
                self.set_vol(self.cur, self.volume);
            }
            Fade::Armed => {
                let o = self.other();
                let _ = self.inst[o].mpv.command("stop", &[]);
            }
            Fade::None => {}
        }
    }

    /// Her döngüde: geçiş zamanı geldiyse hazırla/başlat, sürüyorsa sesleri kaydır.
    fn tick(&mut self, on_event: &impl Fn(Ev)) {
        match self.fade {
            Fade::Running { from, started } => {
                let t = (started.elapsed().as_secs_f64() / self.crossfade.max(0.1)).min(1.0);
                // eşit güç eğrisi: ortada ses düşmesin
                let up = (t * std::f64::consts::FRAC_PI_2).sin();
                let down = (t * std::f64::consts::FRAC_PI_2).cos();
                self.set_vol(self.cur, self.volume * up);
                self.set_vol(from, self.volume * down);
                if t >= 1.0 {
                    self.finish_fade();
                }
            }
            Fade::Armed if self.paused => {}
            Fade::Armed => {
                let a = &self.inst[self.cur];
                let left = a.duration - a.last_time;
                if a.duration > 0.0 && a.last_time >= 0.0 && left <= self.crossfade {
                    let o = self.other();
                    // Hazırlık sırasında biriken olaylar (StartFile, pause=true) yeni etkin
                    // örnekten dışarı sızmasın.
                    while let Some(Ok(_)) = self.inst[o].ev.wait_event(0.0) {}
                    let _ = self.inst[o].mpv.set_property("pause", false);
                    // Eski örnek listesindeki sıradakine kendisi geçmesin.
                    let _ = self.inst[self.cur].mpv.command("playlist-clear", &[]);
                    let from = self.cur;
                    self.cur = o;
                    self.inst[o].last_time = -1.0;
                    self.next = None;
                    self.fade = Fade::Running { from, started: Instant::now() };
                    on_event(Ev::Started(1));
                    let d = self.inst[o].mpv.get_property::<f64>("duration").unwrap_or(0.0);
                    if d > 0.0 {
                        self.inst[o].duration = d;
                        on_event(Ev::Duration(d));
                    }
                }
            }
            Fade::None => {
                if self.crossfade <= 0.0 || self.looping || self.paused {
                    return;
                }
                let Some((url, title)) = self.next.clone() else { return };
                let a = &self.inst[self.cur];
                // Geçişin iki katından kısa parçada geçiş yapılmaz.
                if a.duration < self.crossfade * 2.0 + 5.0 || a.last_time < 0.0 {
                    return;
                }
                if a.duration - a.last_time > self.crossfade + 4.0 {
                    return;
                }
                if !self.ensure_second() {
                    self.crossfade = 0.0;
                    return;
                }
                let o = self.other();
                self.set_vol(o, 0.0);
                let _ = self.inst[o].mpv.set_property("pause", true);
                if loadfile(&self.inst[o].mpv, &url, "replace", &title, 0.0).is_ok() {
                    self.fade = Fade::Armed;
                } else {
                    self.next = None;
                }
            }
        }
    }
}

fn run(rx: Receiver<Cmd>, readahead: u32, on_event: impl Fn(Ev), ready: Sender<Result<(), String>>) {
    let ao = std::env::var("VYNZ_AO").ok();
    let first = match make(readahead, &ao, "vynz-events") {
        Ok(i) => i,
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    let _ = ready.send(Ok(()));
    let mut en = Engine {
        inst: vec![first],
        cur: 0,
        volume: 1.0,
        crossfade: 0.0,
        looping: false,
        af: String::new(),
        readahead,
        ao,
        next: None,
        fade: Fade::None,
        paused: false,
    };
    let mut normalize = false;
    let mut eq: Option<[f32; 10]> = None;

    loop {
        loop {
            let cmd = match rx.try_recv() {
                Ok(c) => c,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => return,
            };
            let c = en.cur;
            match cmd {
                Cmd::Load { url, title, start } => {
                    en.finish_fade();
                    en.next = None;
                    let a = &mut en.inst[c];
                    a.last_time = -1.0;
                    a.duration = 0.0;
                    let _ = a.mpv.command("playlist-clear", &[]);
                    let _ = a.mpv.set_property("pause", false);
                    en.paused = false;
                    en.set_vol(c, en.volume);
                    if let Err(e) = loadfile(&en.inst[c].mpv, &url, "replace", &title, start) {
                        on_event(Ev::Error(e.to_string()));
                    } else {
                        on_event(Ev::Buffering(true));
                    }
                }
                Cmd::Append { url, title } => {
                    if matches!(en.fade, Fade::Armed) {
                        // Hazırlanan sıradaki değişti: bırak, yenisiyle yeniden hazırlanır.
                        en.finish_fade();
                    }
                    let _ = en.inst[c].mpv.command("playlist-clear", &[]);
                    if let Err(e) = loadfile(&en.inst[c].mpv, &url, "append", &title, 0.0) {
                        eprintln!("sıradaki eklenemedi: {e}");
                    }
                    en.next = Some((url, title));
                }
                Cmd::ClearNext => {
                    if matches!(en.fade, Fade::Armed) {
                        en.finish_fade();
                    }
                    en.next = None;
                    let _ = en.inst[c].mpv.command("playlist-clear", &[]);
                }
                Cmd::SetPause(p) => {
                    en.paused = p;
                    if p {
                        en.finish_fade();
                    }
                    let _ = en.inst[en.cur].mpv.set_property("pause", p);
                }
                Cmd::Seek(s) => {
                    en.finish_fade();
                    // mpv yeni konumu bildirene kadar eski konumla geçiş tetiklenmesin
                    en.inst[en.cur].last_time = -1.0;
                    let _ = en.inst[en.cur].mpv.command("seek", &[&format!("{s:.2}"), "absolute"]);
                }
                Cmd::Volume(v) => {
                    en.volume = v;
                    if !matches!(en.fade, Fade::Running { .. }) {
                        en.set_vol(c, v);
                    }
                }
                Cmd::LoopFile(on) => {
                    en.looping = on;
                    if on && matches!(en.fade, Fade::Armed) {
                        en.finish_fade();
                    }
                    let _ = en.inst[c].mpv.set_property("loop-file", if on { "inf" } else { "no" });
                }
                Cmd::Readahead(secs) => {
                    en.readahead = secs;
                    for i in &en.inst {
                        let _ = i.mpv.set_property("cache-secs", readahead_value(secs));
                    }
                }
                Cmd::Normalize(on) => {
                    normalize = on;
                    en.af = filter_chain(normalize, eq.as_ref());
                    for i in &en.inst {
                        let _ = i.mpv.set_property("af", en.af.as_str());
                    }
                }
                Cmd::Eq(g) => {
                    eq = g;
                    en.af = filter_chain(normalize, eq.as_ref());
                    for i in &en.inst {
                        let _ = i.mpv.set_property("af", en.af.as_str());
                    }
                }
                Cmd::Crossfade(s) => {
                    en.crossfade = s.clamp(0.0, 12.0);
                    if en.crossfade <= 0.0 && matches!(en.fade, Fade::Armed) {
                        en.finish_fade();
                    }
                }
            }
        }

        en.tick(&on_event);

        // Olaylar: etkin örneğinkiler dışarı verilir; öbür örnek yalnız boşaltılır.
        // Her örneğin kuyruğu beklemeden tamamen boşaltılır; hiç olay yoksa etkin örnekte kısa beklenir.
        let n = en.inst.len();
        let mut any = false;
        for k in 0..n {
            loop {
                let active = k == en.cur;
                let ev = match en.inst[k].ev.wait_event(0.0) {
                    Some(Ok(e)) => raw(e),
                    Some(Err(_)) => continue,
                    None => break,
                };
                any = true;
                if handle(&mut en, k, active, ev, &on_event) {
                    return;
                }
            }
        }
        if !any {
            let c = en.cur;
            let ev = match en.inst[c].ev.wait_event(0.04) {
                Some(Ok(e)) => Some(raw(e)),
                _ => None,
            };
            if let Some(ev) = ev {
                if handle(&mut en, c, true, ev, &on_event) {
                    return;
                }
            }
        }
    }
}

/// mpv olayının sahip olunan kopyası (olay, istemciyi ödünç alır).
enum Raw {
    Time(f64),
    Duration(f64),
    Pause(bool),
    Cache(bool),
    Start,
    Seek,
    Restart,
    /// (dosya sonu mu, hata mı)
    End(bool, bool),
    Shutdown,
    Other,
}

fn raw(ev: Event) -> Raw {
    match ev {
        Event::PropertyChange { name, change, .. } => match (name, change) {
            ("time-pos", PropertyData::Double(t)) => Raw::Time(t),
            ("duration", PropertyData::Double(d)) => Raw::Duration(d),
            ("pause", PropertyData::Flag(p)) => Raw::Pause(p),
            ("paused-for-cache", PropertyData::Flag(b)) => Raw::Cache(b),
            _ => Raw::Other,
        },
        Event::StartFile => Raw::Start,
        Event::Seek => Raw::Seek,
        Event::PlaybackRestart => Raw::Restart,
        Event::EndFile(r) => Raw::End(r == mpv_end_file_reason::Eof, r == mpv_end_file_reason::Error),
        Event::Shutdown => Raw::Shutdown,
        _ => Raw::Other,
    }
}

/// Tek mpv olayını işle; true = kapanış.
fn handle(en: &mut Engine, k: usize, active: bool, ev: Raw, on_event: &impl Fn(Ev)) -> bool {
    match ev {
        Raw::Time(t) => {
            let i = &mut en.inst[k];
            if active && (t - i.last_time).abs() >= 0.2 {
                i.last_time = t;
                on_event(Ev::Time(t));
            }
        }
        Raw::Duration(d) => {
            en.inst[k].duration = d;
            if active {
                on_event(Ev::Duration(d));
            }
        }
        Raw::Pause(p) if active => on_event(Ev::Paused(p)),
        Raw::Cache(b) if active => on_event(Ev::Buffering(b)),
        Raw::Start if active => {
            if matches!(en.fade, Fade::Armed) {
                en.finish_fade();
            }
            en.inst[k].last_time = -1.0;
            en.inst[k].duration = 0.0;
            let pos = en.inst[k].mpv.get_property::<i64>("playlist-pos").unwrap_or(0);
            if pos >= 1 {
                en.next = None; // mpv sıradakine kendisi geçti
            }
            on_event(Ev::Started(pos));
        }
        Raw::Seek if active => on_event(Ev::Seeked),
        Raw::Restart if active => on_event(Ev::Buffering(false)),
        Raw::End(eof, err) if active => {
            if matches!(en.fade, Fade::Armed) {
                en.finish_fade();
            }
            if eof {
                on_event(Ev::Ended);
            } else if err {
                on_event(Ev::Error("parça açılamadı".into()));
            }
        }
        Raw::End(_, true) if !active && matches!(en.fade, Fade::Armed) => {
            // Hazırlanan sıradaki açılamadı: geçişsiz devam (etkin örnek listesindeki ön-tampon)
            en.fade = Fade::None;
            en.next = None;
        }
        Raw::Shutdown => return true,
        _ => {}
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_chain_builds() {
        assert_eq!(filter_chain(false, None), "");
        assert_eq!(filter_chain(true, None), "lavfi=[dynaudnorm=f=150:g=15]");
        let flat = [0.0f32; 10];
        assert_eq!(filter_chain(false, Some(&flat)), "");
        let mut g = [0.0f32; 10];
        g[0] = 6.0;
        g[9] = -3.0;
        let s = filter_chain(true, Some(&g));
        assert!(s.starts_with("lavfi=[volume=-4.2dB,equalizer=f=32:t=o:w=1:g=6.0,equalizer=f=16000:t=o:w=1:g=-3.0,dynaudnorm"), "{s}");
    }
}
