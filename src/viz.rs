// Canlı spektrum: Linux'ta cava'yı ham çıktı kipinde çalıştırıp bant değerlerini okur.
// Yalnız profil ekranı açıkken ve müzik çalarken çalışır; sistemin ses çıkışını dinler.

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

#[cfg(not(target_os = "linux"))]
mod imp {
    pub struct Viz;

    impl Viz {
        pub fn start(_on_frame: impl Fn(Vec<f32>) + Send + 'static) -> Result<Viz, String> {
            Err("Bu sistemde spektrum desteklenmiyor".into())
        }
    }
}

pub use imp::Viz;
