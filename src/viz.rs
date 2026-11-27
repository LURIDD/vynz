// Canlı spektrum: Linux'ta cava'yı ham çıktı kipinde çalıştırıp bant değerlerini okur;
// Windows'ta sistem çıkışını WASAPI loopback ile dinleyip FFT'yi kendimiz yaparız (cava yok).
// Yalnız gerektiğinde (profil, müziğe tepki) ve müzik çalarken çalışır.

pub const BARS: usize = 32;

#[cfg(target_os = "linux")]
mod imp {
    use std::io::{BufRead, BufReader};
    use std::process::{Child, Command, Stdio};
    use std::thread;

    use super::BARS;

    pub struct Viz {
        child: Child,
    }

    impl Viz {
        /// `on_frame` her karede (≈30/sn) 0..1 arası BARS değerle çağrılır (okuma iş parçacığından).
        pub fn start(on_frame: impl Fn(Vec<f32>) + Send + 'static) -> Result<Viz, String> {
            let dir = crate::cache::dir().parent().map(|p| p.to_path_buf()).ok_or("önbellek yok")?;
            let _ = std::fs::create_dir_all(&dir);
            let conf = dir.join("cava.conf");
            let text = format!(
                "[general]\nbars = {BARS}\nframerate = 30\n\n[input]\nmethod = pipewire\nsource = auto\n\n[output]\nmethod = raw\nraw_target = /dev/stdout\ndata_format = ascii\nascii_max_range = 100\nbar_delimiter = 59\nframe_delimiter = 10\n\n[smoothing]\nnoise_reduction = 70\n"
            );
            std::fs::write(&conf, text).map_err(|e| e.to_string())?;
            let mut child = Command::new("cava")
                .arg("-p")
                .arg(&conf)
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| format!("cava başlatılamadı: {e}"))?;
            let out = child.stdout.take().ok_or("cava çıktısı yok")?;
            thread::spawn(move || {
                for line in BufReader::new(out).lines() {
                    let Ok(line) = line else { break };
                    let v: Vec<f32> = line
                        .split(';')
                        .filter(|x| !x.is_empty())
                        .filter_map(|x| x.trim().parse::<f32>().ok())
                        .map(|x| (x / 100.0).clamp(0.0, 1.0))
                        .collect();
                    if v.len() == BARS {
                        on_frame(v);
                    }
                }
            });
            Ok(Viz { child })
        }
    }

    impl Drop for Viz {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

#[cfg(target_os = "windows")]
mod imp {
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::{Duration, Instant};

    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

    use super::{Bands, BARS, FFT_N};

    pub struct Viz {
        _stream: cpal::Stream,
        stop: Arc<AtomicBool>,
    }

    impl Viz {
        /// `on_frame` her karede (≈30/sn) 0..1 arası BARS değerle çağrılır (iş parçacığından).
        pub fn start(on_frame: impl Fn(Vec<f32>) + Send + 'static) -> Result<Viz, String> {
            let host = cpal::default_host();
            let dev = host.default_output_device().ok_or("ses çıkışı yok")?;
            let sup = dev.default_output_config().map_err(|e| e.to_string())?;
            let rate = sup.sample_rate().0 as f32;
            let ch = sup.channels().max(1) as usize;
            let fmt = sup.sample_format();
            let cfg: cpal::StreamConfig = sup.into();
            // Son örnekler (mono); işleyici iş parçacığı FFT_N kadarını okur.
            let ring: Arc<Mutex<(VecDeque<f32>, Instant)>> =
                Arc::new(Mutex::new((VecDeque::with_capacity(FFT_N * 2), Instant::now())));
            fn push(ring: &Mutex<(VecDeque<f32>, Instant)>, ch: usize, it: impl Iterator<Item = f32>) {
                let mut g = ring.lock().unwrap();
                let mut acc = 0.0f32;
                for (i, x) in it.enumerate() {
                    acc += x;
                    if i % ch == ch - 1 {
                        g.0.push_back(acc / ch as f32);
                        acc = 0.0;
                    }
                }
                while g.0.len() > FFT_N * 2 {
                    g.0.pop_front();
                }
                g.1 = Instant::now();
            }
            let err = |e: cpal::StreamError| eprintln!("spektrum akışı: {e}");
            // Çıkış aygıtında giriş akışı = WASAPI loopback (sistemin çaldığı ses).
            let stream = match fmt {
                cpal::SampleFormat::F32 => {
                    let r = ring.clone();
                    dev.build_input_stream(&cfg, move |d: &[f32], _: &cpal::InputCallbackInfo| push(&r, ch, d.iter().copied()), err, None)
                }
                cpal::SampleFormat::I16 => {
                    let r = ring.clone();
                    dev.build_input_stream(&cfg, move |d: &[i16], _: &cpal::InputCallbackInfo| push(&r, ch, d.iter().map(|&x| x as f32 / 32768.0)), err, None)
                }
                cpal::SampleFormat::I32 => {
                    let r = ring.clone();
                    dev.build_input_stream(&cfg, move |d: &[i32], _: &cpal::InputCallbackInfo| push(&r, ch, d.iter().map(|&x| x as f32 / 2147483648.0)), err, None)
                }
                cpal::SampleFormat::U16 => {
                    let r = ring.clone();
                    dev.build_input_stream(&cfg, move |d: &[u16], _: &cpal::InputCallbackInfo| push(&r, ch, d.iter().map(|&x| (x as f32 - 32768.0) / 32768.0)), err, None)
                }
                other => return Err(format!("desteklenmeyen örnek biçimi: {other:?}")),
            }
            .map_err(|e| e.to_string())?;
            stream.play().map_err(|e| e.to_string())?;

            let stop = Arc::new(AtomicBool::new(false));
            let stop2 = stop.clone();
            thread::spawn(move || {
                let mut bands = Bands::new(rate);
                let mut buf = vec![0.0f32; FFT_N];
                while !stop2.load(Ordering::Relaxed) {
                    thread::sleep(Duration::from_millis(33));
                    let fresh = {
                        let g = ring.lock().unwrap();
                        let n = g.0.len().min(FFT_N);
                        buf.iter_mut().for_each(|x| *x = 0.0);
                        for (i, x) in g.0.iter().skip(g.0.len() - n).enumerate() {
                            buf[FFT_N - n + i] = *x;
                        }
                        // Loopback sessizlikte paket göndermez: veri eskidiyse sustur.
                        g.1.elapsed() < Duration::from_millis(150)
                    };
                    if !fresh {
                        buf.iter_mut().for_each(|x| *x = 0.0);
                    }
                    on_frame(bands.frame(&buf));
                }
            });
            let _ = BARS;
            Ok(Viz { _stream: stream, stop })
        }
    }

    impl Drop for Viz {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
mod imp {
    pub struct Viz;

    impl Viz {
        pub fn start(_on_frame: impl Fn(Vec<f32>) + Send + 'static) -> Result<Viz, String> {
            Err("Bu sistemde spektrum desteklenmiyor".into())
        }
    }
}

pub use imp::Viz;

/// Windows'ta FFT pencere boyu (48 kHz'de ~43 ms).
#[allow(dead_code)]
const FFT_N: usize = 2048;

/// Ham örneklerden cava benzeri BARS bant: Hann penceresi + FFT, 50 Hz–12 kHz arası
/// logaritmik bantlar, kendiliğinden kazanç (autosens) ve yerçekimli düşüş.
#[allow(dead_code)]
struct Bands {
    rate: f32,
    edges: Vec<usize>,
    window: Vec<f32>,
    gain: f32,
    prev: Vec<f32>,
    fall: Vec<f32>,
}

#[allow(dead_code)]
impl Bands {
    fn new(rate: f32) -> Bands {
        let bin = |hz: f32| ((hz / rate * FFT_N as f32).round() as usize).clamp(1, FFT_N / 2 - 1);
        let (lo, hi) = (50.0f32, 12_000.0f32.min(rate * 0.45));
        let mut edges: Vec<usize> = (0..=BARS).map(|i| bin(lo * (hi / lo).powf(i as f32 / BARS as f32))).collect();
        // Alt bantlarda her bant en az bir kutu alsın
        for i in 1..edges.len() {
            if edges[i] <= edges[i - 1] {
                edges[i] = edges[i - 1] + 1;
            }
        }
        let window = (0..FFT_N)
            .map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (FFT_N - 1) as f32).cos())
            .collect();
        Bands { rate, edges, window, gain: 1.0, prev: vec![0.0; BARS], fall: vec![0.0; BARS] }
    }

    fn frame(&mut self, samples: &[f32]) -> Vec<f32> {
        let mut re: Vec<f32> = samples.iter().zip(&self.window).map(|(x, w)| x * w).collect();
        let mut im = vec![0.0f32; FFT_N];
        fft(&mut re, &mut im);
        let mut out = vec![0.0f32; BARS];
        for b in 0..BARS {
            let (a, z) = (self.edges[b], self.edges[b + 1].max(self.edges[b] + 1));
            let mut m = 0.0f32;
            for k in a..z.min(FFT_N / 2) {
                m = m.max((re[k] * re[k] + im[k] * im[k]).sqrt());
            }
            // Yüksek frekanslar doğal olarak zayıf: hafif eğim
            let tilt = 1.0 + b as f32 / BARS as f32 * 2.0;
            out[b] = (m * tilt).sqrt();
        }
        // autosens: tepe 1'i aşarsa kazancı hızla düşür, uzun süre düşükse yavaşça artır
        let peak = out.iter().cloned().fold(0.0f32, f32::max) * self.gain;
        if peak > 1.0 {
            self.gain *= 0.85 / peak.max(1e-6) + 0.15;
        } else if peak > 0.0001 {
            self.gain *= 1.002;
        }
        self.gain = self.gain.clamp(1e-4, 50.0);
        for b in 0..BARS {
            let v = (out[b] * self.gain).clamp(0.0, 1.0);
            // yükselişte anında, inişte yerçekimiyle
            if v >= self.prev[b] {
                self.prev[b] = v;
                self.fall[b] = 0.0;
            } else {
                self.fall[b] += 0.004;
                self.prev[b] = (self.prev[b] - self.fall[b]).max(v);
            }
            out[b] = self.prev[b];
        }
        let _ = self.rate;
        out
    }
}

/// Yerinde, özyinelemesiz radix-2 FFT (uzunluk 2'nin kuvveti).
#[allow(dead_code)]
fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -2.0 * std::f32::consts::PI / len as f32;
        let (wr, wi) = (ang.cos(), ang.sin());
        for start in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1.0f32, 0.0f32);
            for k in 0..len / 2 {
                let (a, b) = (start + k, start + k + len / 2);
                let tr = re[b] * cr - im[b] * ci;
                let ti = re[b] * ci + im[b] * cr;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                let ncr = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = ncr;
            }
        }
        len <<= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fft_finds_tone() {
        let rate = 48_000.0;
        // 1 kHz sinüs
        let s: Vec<f32> = (0..FFT_N).map(|i| (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / rate).sin() * 0.5).collect();
        let mut b = Bands::new(rate);
        let mut f = vec![0.0; BARS];
        for _ in 0..30 {
            f = b.frame(&s);
        }
        let top = f.iter().enumerate().max_by(|a, c| a.1.total_cmp(c.1)).unwrap().0;
        // 1 kHz, 50 Hz–12 kHz log ölçekte ~%55 konumda
        let expect = ((1000.0f32 / 50.0).ln() / (12000.0f32 / 50.0).ln() * BARS as f32) as usize;
        assert!((top as i32 - expect as i32).abs() <= 1, "tepe bant {top}, beklenen ~{expect}: {f:?}");
        assert!(f[top] > 0.5);
        // sessizlik sıfıra iner
        let z = vec![0.0; FFT_N];
        for _ in 0..200 {
            f = b.frame(&z);
        }
        assert!(f.iter().all(|&x| x < 0.01), "{f:?}");
    }
}
