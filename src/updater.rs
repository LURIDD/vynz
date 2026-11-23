// Kendini güncelleme (Discord benzeri): açılışta GitHub'daki son sürüme bakılır; yenisi
// varsa küçük bir pencerede indirilir, SHA256SUMS ile doğrulanır, ikili yerinde
// değiştirilir ve uygulama yeniden açılır (oturum kaldığı yerden sürer).
//
// - Kaynak: https://api.github.com/repos/LURIDD/vynz/releases/latest (ön sürümler hariç).
//   Test için VYNZ_UPDATE_URL aynı biçimde bir JSON adresi verebilir (file:// değil, http).
// - Geliştirici derlemesi (cargo `target/` içinden çalışan) kendini güncellemez;
//   VYNZ_UPDATE_FORCE=1 bunu aşar (yalnız test).
// - Linux: arşivdeki `vynz` aynı klasöre `.new` olarak yazılır, sonra eskisinin üstüne
//   taşınır (çalışan dosya için güvenli). Windows: çalışan .exe ve .dll silinemez ama
//   yeniden adlandırılabilir: eskiler `.old` olur, açılışta temizlenir.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use sha2::{Digest, Sha256};

const API: &str = "https://api.github.com/repos/LURIDD/vynz/releases/latest";
pub const RELEASES_PAGE: &str = "https://github.com/LURIDD/vynz/releases/latest";

#[derive(Clone, Debug)]
pub struct Update {
    pub version: String,
    pub notes: String,
    pub asset_url: String,
    pub asset_name: String,
    pub sums_url: Option<String>,
}

pub fn current() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

fn asset_name() -> &'static str {
    if cfg!(target_os = "windows") { "vynz-windows-x86_64.zip" } else { "vynz-linux-x86_64.tar.gz" }
}

/// cargo ile derlenmiş, `target/` içinden çalışan kopya mı.
pub fn is_dev_build() -> bool {
    if std::env::var_os("VYNZ_UPDATE_FORCE").is_some() {
        return false;
    }
    let Ok(exe) = std::env::current_exe().and_then(|p| p.canonicalize()) else { return true };
    exe.components().any(|c| c.as_os_str() == "target")
}

/// "0.2.0", "v0.2.1", "0.3.0-beta.1" → karşılaştırılabilir (ön sürüm, aynı sayılı sürümden küçük).
fn parse(v: &str) -> Option<(u64, u64, u64, bool)> {
    let v = v.trim().trim_start_matches('v');
    let (core, pre) = match v.split_once('-') {
        Some((c, _)) => (c, true),
        None => (v, false),
    };
    let mut it = core.split('.').map(|x| x.parse::<u64>().ok());
    Some((it.next()??, it.next().flatten().unwrap_or(0), it.next().flatten().unwrap_or(0), !pre))
}

pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse(candidate), parse(current)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

/// Büyük paket indirmesi: toplam süre sınırı yok, yalnız bağlantı ve yanıt bekleme sınırı.
fn download_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_connect(Some(std::time::Duration::from_secs(15)))
        .timeout_recv_response(Some(std::time::Duration::from_secs(30)))
        .build()
        .into()
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(20)))
        .build()
        .into()
}

/// Son sürümü sor; yeniyse ve bu platform için paket varsa döner.
pub fn check() -> Result<Option<Update>, String> {
    let url = std::env::var("VYNZ_UPDATE_URL").unwrap_or_else(|_| API.to_string());
    let mut resp = agent()
        .get(&url)
        .header("User-Agent", "vynz-updater")
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| e.to_string())?;
    let text = resp.body_mut().read_to_string().map_err(|e| e.to_string())?;
    let v: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let tag = v["tag_name"].as_str().unwrap_or("").to_string();
    if v["prerelease"].as_bool().unwrap_or(false) || v["draft"].as_bool().unwrap_or(false) {
        return Ok(None);
    }
    if !is_newer(&tag, current()) {
        return Ok(None);
    }
    let assets = v["assets"].as_array().cloned().unwrap_or_default();
    let find = |name: &str| {
        assets
            .iter()
            .find(|a| a["name"].as_str() == Some(name))
            .and_then(|a| a["browser_download_url"].as_str())
            .map(str::to_string)
    };
    let Some(asset_url) = find(asset_name()) else { return Ok(None) };
    let notes: String = plain_notes(v["body"].as_str().unwrap_or("")).chars().take(600).collect();
    Ok(Some(Update {
        version: tag.trim_start_matches('v').to_string(),
        notes,
        asset_url,
        asset_name: asset_name().to_string(),
        sums_url: find("SHA256SUMS"),
    }))
}

/// Sürüm notlarındaki markdown işaretlerini sadeleştir (başlık #, kalın **, madde -/*).
fn plain_notes(md: &str) -> String {
    md.lines()
        .map(|l| {
            let t = l.trim_start_matches('#').trim().replace("**", "");
            match t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")) {
                Some(rest) => format!("• {rest}"),
                None => t,
            }
        })
        .filter(|l| !l.starts_with("<!--"))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

fn work_dir() -> PathBuf {
    crate::paths::cache_dir().join("update")
}

/// İndir, doğrula, kur. `cancel` true olursa yarıda bırakır (hiçbir dosyaya dokunulmamış olur).
pub fn install(u: &Update, cancel: &AtomicBool, progress: impl Fn(f32)) -> Result<(), String> {
    let dir = work_dir();
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let archive = dir.join(&u.asset_name);

    // 1) indir
    let mut resp = download_agent().get(&u.asset_url).header("User-Agent", "vynz-updater").call().map_err(|e| e.to_string())?;
    let total: u64 = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let mut reader = resp.body_mut().with_config().limit(400 * 1024 * 1024).reader();
    let mut file = std::fs::File::create(&archive).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 128 * 1024];
    let mut got = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("iptal edildi".into());
        }
        let n = reader.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        std::io::Write::write_all(&mut file, &buf[..n]).map_err(|e| e.to_string())?;
        hasher.update(&buf[..n]);
        got += n as u64;
        if total > 0 {
            progress((got as f32 / total as f32 * 0.9).min(0.9));
        }
    }
    drop(file);

    // 2) doğrula
    if let Some(sums) = &u.sums_url {
        let mut r = agent().get(sums).header("User-Agent", "vynz-updater").call().map_err(|e| e.to_string())?;
        let text = r.body_mut().read_to_string().map_err(|e| e.to_string())?;
        let want = text
            .lines()
            .find(|l| l.split_whitespace().last().map(|n| n.trim_start_matches('*')) == Some(u.asset_name.as_str()))
            .and_then(|l| l.split_whitespace().next())
            .ok_or("SHA256SUMS içinde paket yok")?
            .to_ascii_lowercase();
        let have: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
        if want != have {
            return Err("indirilen dosya doğrulanamadı (SHA-256 tutmuyor)".into());
        }
    }
    if cancel.load(Ordering::Relaxed) {
        return Err("iptal edildi".into());
    }

    // 3) aç
    let staged = dir.join("staged");
    std::fs::create_dir_all(&staged).map_err(|e| e.to_string())?;
    let files = extract(&archive, &staged)?;
    if files.is_empty() {
        return Err("pakette dosya yok".into());
    }
    progress(0.95);

    // 4) kur (bundan sonra iptal yok: yarım kurulum bırakılmaz)
    if cancel.load(Ordering::Relaxed) {
        return Err("iptal edildi".into());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe = exe.canonicalize().unwrap_or(exe);
    let target_dir = exe.parent().ok_or("uygulama klasörü bulunamadı")?.to_path_buf();
    let exe_name = exe.file_name().map(|n| n.to_owned()).unwrap_or_default();
    for f in files {
        let name = f.file_name().map(|n| n.to_owned()).unwrap_or_default();
        let lower = name.to_string_lossy().to_ascii_lowercase();
        let is_exe = lower == "vynz" || lower == "vynz.exe";
        let is_lib = lower.ends_with(".dll") || lower.ends_with(".so");
        if !is_exe && !is_lib {
            continue; // LICENSE, README
        }
        let dest = if is_exe { target_dir.join(&exe_name) } else { target_dir.join(&name) };
        replace_file(&f, &dest)?;
    }
    progress(1.0);
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

fn extract(archive: &Path, out: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    let name = archive.to_string_lossy().to_ascii_lowercase();
    if name.ends_with(".zip") {
        let f = std::fs::File::open(archive).map_err(|e| e.to_string())?;
        let mut z = zip::ZipArchive::new(f).map_err(|e| e.to_string())?;
        for i in 0..z.len() {
            let mut e = z.by_index(i).map_err(|e| e.to_string())?;
            if e.is_dir() {
                continue;
            }
            let Some(base) = e.enclosed_name().and_then(|p| p.file_name().map(|n| n.to_owned())) else { continue };
            let dest = out.join(base);
            let mut w = std::fs::File::create(&dest).map_err(|e| e.to_string())?;
            std::io::copy(&mut e, &mut w).map_err(|e| e.to_string())?;
            files.push(dest);
        }
    } else {
        let f = std::fs::File::open(archive).map_err(|e| e.to_string())?;
        let mut t = tar::Archive::new(flate2::read::GzDecoder::new(f));
        for e in t.entries().map_err(|e| e.to_string())? {
            let mut e = e.map_err(|e| e.to_string())?;
            if !e.header().entry_type().is_file() {
                continue;
            }
            let p = e.path().map_err(|e| e.to_string())?.into_owned();
            let Some(base) = p.file_name().map(|n| n.to_owned()) else { continue };
            let dest = out.join(base);
            e.unpack(&dest).map_err(|e| e.to_string())?;
            files.push(dest);
        }
    }
    Ok(files)
}

fn replace_file(src: &Path, dest: &Path) -> Result<(), String> {
    let tmp = dest.with_extension(match dest.extension() {
        Some(e) => format!("{}.new", e.to_string_lossy()),
        None => "new".into(),
    });
    std::fs::copy(src, &tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755));
    }
    #[cfg(windows)]
    {
        if dest.exists() {
            let old = dest.with_extension(match dest.extension() {
                Some(e) => format!("{}.old", e.to_string_lossy()),
                None => "old".into(),
            });
            let _ = std::fs::remove_file(&old);
            std::fs::rename(dest, &old).map_err(|e| format!("{}: {e}", dest.display()))?;
        }
    }
    std::fs::rename(&tmp, dest).map_err(|e| format!("{}: {e}", dest.display()))
}

/// Windows'ta önceki güncellemeden kalan .old dosyalarını sil.
pub fn cleanup_old() {
    let Ok(exe) = std::env::current_exe() else { return };
    let Some(dir) = exe.parent() else { return };
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            if e.file_name().to_string_lossy().ends_with(".old") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

/// Güncellenmiş ikiliyi başlat; eski süreç kapanana kadar bekler (--wait-pid).
pub fn relaunch() {
    let Ok(exe) = std::env::current_exe() else { return };
    // Linux'ta yerine taşınan dosyadan sonra /proc/self/exe " (deleted)" ekler.
    let exe = PathBuf::from(exe.to_string_lossy().trim_end_matches(" (deleted)"));
    let args: Vec<String> = std::env::args().skip(1).filter(|a| !a.starts_with("--wait-pid")).collect();
    let _ = std::process::Command::new(exe)
        .args(args)
        .arg(format!("--wait-pid={}", std::process::id()))
        .env("VYNZ_UPDATED", "1")
        .spawn();
}

/// `--wait-pid=N` verilmişse o süreç bitene kadar (en çok 8 sn) bekle.
pub fn wait_for_parent() {
    let Some(pid) = std::env::args().find_map(|a| a.strip_prefix("--wait-pid=").and_then(|p| p.parse::<u32>().ok())) else { return };
    let start = std::time::Instant::now();
    while start.elapsed() < std::time::Duration::from_secs(8) && alive(pid) {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

fn alive(pid: u32) -> bool {
    #[cfg(target_os = "linux")]
    {
        Path::new(&format!("/proc/{pid}")).exists()
    }
    #[cfg(not(target_os = "linux"))]
    {
        // Süreç sorgusu için ek bağımlılık yerine kısa bir bekleme yeter.
        let _ = pid;
        std::thread::sleep(std::time::Duration::from_millis(1500));
        false
    }
}

/// Güncelleme denetimi ve kurulum sırasında arayüzden iptal.
pub static CANCEL: AtomicBool = AtomicBool::new(false);
/// Bir kurulum sürüyor (aynı anda ikincisi başlamasın).
pub static BUSY: AtomicBool = AtomicBool::new(false);
/// Kapanışta yeniden başlat.
pub static RESTART: AtomicBool = AtomicBool::new(false);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_compare() {
        assert!(is_newer("v0.2.1", "0.2.0"));
        assert!(is_newer("0.3.0", "0.2.9"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(!is_newer("v0.2.0", "0.2.0"));
        assert!(!is_newer("0.1.0-alpha.4", "0.2.0"));
        assert!(is_newer("0.2.0", "0.2.0-beta.1"));
        assert!(!is_newer("0.2.0-beta.1", "0.2.0"));
        assert!(!is_newer("çöp", "0.2.0"));
        assert_eq!(plain_notes("## Yeni\n- **Ekolayzır** eklendi\n* geçiş"), "Yeni\n• Ekolayzır eklendi\n• geçiş");
    }
}
