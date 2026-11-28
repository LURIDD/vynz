// Tek kopya: açılırken çalışan bir Vynz varsa ona "öne gel" deyip çık.
// Yalnız yerel döngü (127.0.0.1) üzerinden sabit bir bağlantı noktası; mesaj tek satır:
// "show" (öne gel) ya da "quit" (`vynz --quit`: tepside/minide olsa da düzgünce kapan).

use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::time::Duration;

const PORT: u16 = 47613;

/// VYNZ_PORT: testte ayrı bağlantı noktası (kullanıcının açık Vynz'ine dokunmadan --quit denemek için).
fn port() -> u16 {
    std::env::var("VYNZ_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(PORT)
}

fn addr() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], port()))
}

/// Test kopyaları (VYNZ_TITLE) ve VYNZ_MULTI kilidi atlar; VYNZ_PORT verilirse yine açıktır.
pub fn enabled() -> bool {
    std::env::var_os("VYNZ_PORT").is_some()
        || (std::env::var_os("VYNZ_TITLE").is_none() && std::env::var_os("VYNZ_MULTI").is_none())
}

/// Çalışan kopyaya ulaşılırsa true (bu süreç çıkmalı).
pub fn signal_existing() -> bool {
    send("show")
}

/// Çalışan kopyaya komut gönder; karşı taraf Vynz ise ve aldıysa true.
pub fn send(cmd: &str) -> bool {
    let Ok(mut s) = TcpStream::connect_timeout(&addr(), Duration::from_millis(300)) else { return false };
    let _ = s.set_read_timeout(Some(Duration::from_millis(500)));
    if s.write_all(format!("{cmd}\n").as_bytes()).is_err() {
        return false;
    }
    // Karşı taraf Vynz mi? "ok" bekle (başka bir program bu bağlantı noktasındaysa yine aç)
    let mut line = String::new();
    BufReader::new(s).read_line(&mut line).is_ok() && line.trim() == "ok"
}

/// Dinleyiciyi başlat; her "show"/"quit" isteğinde `on_cmd` çağrılır (dinleme iş parçacığından).
pub fn listen(on_cmd: impl Fn(&str) + Send + 'static) {
    let Ok(l) = TcpListener::bind(addr()) else { return };
    std::thread::spawn(move || {
        for c in l.incoming() {
            let Ok(mut c) = c else { continue };
            let _ = c.set_read_timeout(Some(Duration::from_millis(500)));
            let mut line = String::new();
            if BufReader::new(&c).read_line(&mut line).is_ok() {
                let cmd = line.trim();
                if cmd == "show" || cmd == "quit" {
                    let _ = c.write_all(b"ok\n");
                    on_cmd(cmd);
                }
            }
        }
    });
}
