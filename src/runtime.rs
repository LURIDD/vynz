// Çözücü çalışma ortamı: Vynz veri klasöründe izole uv + Python + yt-dlp + ytmusicapi + deno.
// Sistem Python'una dokunmaz; Linux ve Windows'ta aynı düzende çalışır.
// yt-dlp birkaç günde bir arka planda sessizce güncellenir.

use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::lyrics::USER_AGENT;

/// yt-dlp otomatik güncelleme aralığı (3 gün).
const UPDATE_INTERVAL_SECS: u64 = 3 * 24 * 3600;
const PYTHON_VERSION: &str = "3.12";

#[derive(Clone, Debug)]
pub struct Runtime {
    pub python: PathBuf,
    pub deno: Option<PathBuf>,
    pub bin_dir: Option<PathBuf>,
}

static SETUP_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn lock() -> std::sync::MutexGuard<'static, ()> {
    SETUP_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
}

pub fn root_dir() -> PathBuf {
    crate::paths::data_dir().join("runtime")
}

fn bin_dir() -> PathBuf {
    root_dir().join("bin")
}

fn venv_dir() -> PathBuf {
    root_dir().join("venv")
}

fn python_install_dir() -> PathBuf {
    root_dir().join("python")
}

fn uv_path() -> PathBuf {
    bin_dir().join(if cfg!(windows) { "uv.exe" } else { "uv" })
}

fn deno_path() -> PathBuf {
    bin_dir().join(if cfg!(windows) { "deno.exe" } else { "deno" })
}

fn venv_python() -> PathBuf {
    if cfg!(windows) {
        venv_dir().join("Scripts").join("python.exe")
    } else {
        venv_dir().join("bin").join("python")
    }
}

fn stamp_path() -> PathBuf {
    root_dir().join("last_update")
}

fn ready_marker() -> PathBuf {
    root_dir().join(".ready")
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Windows'ta alt süreç başlatırken siyah konsol penceresi çıkmasını önler.
pub fn hide_window(cmd: &mut Command) {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = cmd;
    }
}

fn managed_runtime_if_ready() -> Option<Runtime> {
    let py = venv_python();
    let deno = deno_path();
    if py.is_file() && deno.is_file() && ready_marker().is_file() {
        Some(Runtime { python: py, deno: Some(deno), bin_dir: Some(bin_dir()) })
    } else {
        None
    }
}

fn system_python_has_deps() -> bool {
    let py = if cfg!(windows) { "python" } else { "python3" };
    let mut cmd = Command::new(py);
    hide_window(&mut cmd);
    cmd.args(["-I", "-c", "import yt_dlp, ytmusicapi"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Çalışma ortamını hazırla:
/// - Yönetilen ortam hazırsa anında onu döner.
/// - Hazır değilse ve sistem Python'unda yt-dlp+ytmusicapi varsa hemen onu kullanır,
///   yönetilen ortamı (uv + deno + venv) arka planda sessizce kurar (bir sonraki başlatmada ona geçer).
/// - Sistemde yoksa (temiz Windows / Python'suz Linux) şimdi kurar ve onu döner.
pub fn ensure_ready() -> Runtime {
    if let Some(rt) = managed_runtime_if_ready() {
        return rt;
    }
    if system_python_has_deps() {
        std::thread::Builder::new()
            .name("vynz-runtime-bootstrap".into())
            .spawn(|| {
                let _g = lock();
                if managed_runtime_if_ready().is_none() {
                    let _ = bootstrap();
                }
            })
            .ok();
        return fallback_runtime();
    }
    let _g = lock();
    if let Some(rt) = managed_runtime_if_ready() {
        return rt;
    }
    match bootstrap() {
        Ok(rt) => rt,
        Err(e) => {
            if std::env::var_os("VYNZ_DEBUG").is_some() {
                eprintln!("[runtime] kurulum başarısız, sistem python kullanılacak: {e}");
            }
            fallback_runtime()
        }
    }
}

fn fallback_runtime() -> Runtime {
    let deno = deno_path();
    let has_deno = deno.is_file();
    Runtime {
        python: PathBuf::from(if cfg!(windows) { "python" } else { "python3" }),
        deno: has_deno.then_some(deno),
        bin_dir: has_deno.then_some(bin_dir()),
    }
}

/// deno yolu: önce yönetilen ortamdaki, yoksa PATH'teki.
pub fn deno_bin() -> Option<PathBuf> {
    let managed = deno_path();
    if managed.is_file() {
        return Some(managed);
    }
    let name = if cfg!(windows) { "deno.exe" } else { "deno" };
    std::env::var_os("PATH")
        .and_then(|p| std::env::split_paths(&p).map(|d| d.join(name)).find(|f| f.is_file()))
}

pub fn is_ready() -> bool {
    managed_runtime_if_ready().is_some()
}

pub fn status_text() -> String {
    if managed_runtime_if_ready().is_some() {
        tr!("Hazır (izole uv + deno + yt-dlp)")
    } else if system_python_has_deps() {
        tr!("Sistem Python (izole ortam arka planda hazırlanıyor)")
    } else {
        tr!("İlk kurulum bekleniyor…")
    }
}

/// Ayarlar'dan tek tıkla yt-dlp güncelleme.
pub fn update_now(on_done: impl FnOnce(Result<String, String>) + Send + 'static) {
    std::thread::Builder::new()
        .name("vynz-ytdlp-update-now".into())
        .spawn(move || {
            let _g = lock();
            let res = if managed_runtime_if_ready().is_none() {
                bootstrap().map(|_| tr!("Çözücü ortamı kuruldu ve güncel"))
            } else {
                let mut cmd = Command::new(uv_path());
                hide_window(&mut cmd);
                match cmd
                    .args(["pip", "install", "--python"])
                    .arg(venv_python())
                    .args(["--upgrade", "yt-dlp", "ytmusicapi"])
                    .env("UV_PYTHON_INSTALL_DIR", python_install_dir())
                    .stdin(Stdio::null())
                    .output()
                {
                    Ok(out) if out.status.success() => {
                        let _ = std::fs::write(stamp_path(), now_secs().to_string());
                        Ok(tr!("yt-dlp ve ytmusicapi güncellendi"))
                    }
                    Ok(out) => Err(String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("güncellenemedi").into()),
                    Err(e) => Err(e.to_string()),
                }
            };
            on_done(res);
        })
        .ok();
}

/// Uygulama açıldıktan sonra arka planda çağrılır: yt-dlp 3 günden eskiyse sessizce günceller.
pub fn maybe_update_background() {
    std::thread::Builder::new()
        .name("vynz-ytdlp-update".into())
        .spawn(|| {
            if !ready_marker().is_file() || !uv_path().is_file() || !venv_python().is_file() {
                return;
            }
            let last: u64 = std::fs::read_to_string(stamp_path())
                .ok()
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(0);
            if now_secs().saturating_sub(last) < UPDATE_INTERVAL_SECS {
                return;
            }
            let _g = lock();
            let mut cmd = Command::new(uv_path());
            hide_window(&mut cmd);
            let ok = cmd
                .args(["pip", "install", "--python"])
                .arg(venv_python())
                .args(["--upgrade", "yt-dlp"])
                .env("UV_PYTHON_INSTALL_DIR", python_install_dir())
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success());
            if ok {
                let _ = std::fs::write(stamp_path(), now_secs().to_string());
            }
        })
        .ok();
}

fn bootstrap() -> Result<Runtime, String> {
    std::fs::create_dir_all(bin_dir()).map_err(|e| format!("runtime/bin oluşturulamadı: {e}"))?;
    std::fs::create_dir_all(python_install_dir()).map_err(|e| format!("runtime/python oluşturulamadı: {e}"))?;

    let uv = uv_path();
    if !uv.is_file() {
        download_uv(&uv)?;
    }

    let deno = deno_path();
    if !deno.is_file() {
        download_deno(&deno)?;
    }

    let py = venv_python();
    if !py.is_file() {
        let mut cmd = Command::new(&uv);
        hide_window(&mut cmd);
        let out = cmd
            .args(["venv", "--python", PYTHON_VERSION, "--managed-python", "--clear"])
            .arg(venv_dir())
            .env("UV_PYTHON_INSTALL_DIR", python_install_dir())
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("uv venv çalıştırılamadı: {e}"))?;
        if !out.status.success() {
            return Err(format!("uv venv hatası: {}", String::from_utf8_lossy(&out.stderr).trim()));
        }
    }

    let mut cmd = Command::new(&uv);
    hide_window(&mut cmd);
    let out = cmd
        .args(["pip", "install", "--python"])
        .arg(&py)
        .args(["--upgrade", "yt-dlp", "ytmusicapi"])
        .env("UV_PYTHON_INSTALL_DIR", python_install_dir())
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("uv pip install çalıştırılamadı: {e}"))?;
    if !out.status.success() {
        return Err(format!("uv pip hatası: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }

    let _ = std::fs::write(stamp_path(), now_secs().to_string());
    let _ = std::fs::write(ready_marker(), "1");
    Ok(Runtime { python: py, deno: Some(deno), bin_dir: Some(bin_dir()) })
}

fn http_get_bytes(url: &str) -> Result<Vec<u8>, String> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(120)))
        .build()
        .new_agent();
    let mut resp = agent
        .get(url)
        .header("User-Agent", USER_AGENT)
        .call()
        .map_err(|e| format!("indirme hatası ({url}): {e}"))?;
    let mut buf = Vec::new();
    resp.body_mut()
        .as_reader()
        .read_to_end(&mut buf)
        .map_err(|e| format!("okuma hatası ({url}): {e}"))?;
    Ok(buf)
}

fn set_executable(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(path).map_err(|e| e.to_string())?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(path, perms).map_err(|e| e.to_string())?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

fn extract_from_zip(bytes: &[u8], target_name: &str, dest: &Path) -> Result<(), String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| format!("zip açılamadı: {e}"))?;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| e.to_string())?;
        let fname = Path::new(file.name()).file_name().and_then(|s| s.to_str()).unwrap_or("");
        if fname.eq_ignore_ascii_case(target_name) {
            let tmp = dest.with_extension("tmp");
            let mut out = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
            std::io::copy(&mut file, &mut out).map_err(|e| e.to_string())?;
            drop(out);
            set_executable(&tmp)?;
            std::fs::rename(&tmp, dest).map_err(|e| e.to_string())?;
            return Ok(());
        }
    }
    Err(format!("zip içinde {target_name} bulunamadı"))
}

fn extract_from_targz(bytes: &[u8], target_name: &str, dest: &Path) -> Result<(), String> {
    let gz = flate2::read::GzDecoder::new(Cursor::new(bytes));
    let mut archive = tar::Archive::new(gz);
    for entry in archive.entries().map_err(|e| format!("tar.gz açılamadı: {e}"))? {
        let mut entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path().map_err(|e| e.to_string())?;
        if path.file_name().and_then(|s| s.to_str()) == Some(target_name) {
            let tmp = dest.with_extension("tmp");
            let mut out = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
            std::io::copy(&mut entry, &mut out).map_err(|e| e.to_string())?;
            drop(out);
            set_executable(&tmp)?;
            std::fs::rename(&tmp, dest).map_err(|e| e.to_string())?;
            return Ok(());
        }
    }
    Err(format!("tar.gz içinde {target_name} bulunamadı"))
}

fn uv_asset_url() -> Result<&'static str, String> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Ok("https://github.com/astral-sh/uv/releases/latest/download/uv-x86_64-unknown-linux-gnu.tar.gz"),
        ("linux", "aarch64") => Ok("https://github.com/astral-sh/uv/releases/latest/download/uv-aarch64-unknown-linux-gnu.tar.gz"),
        ("windows", "x86_64") => Ok("https://github.com/astral-sh/uv/releases/latest/download/uv-x86_64-pc-windows-msvc.zip"),
        ("windows", "aarch64") => Ok("https://github.com/astral-sh/uv/releases/latest/download/uv-aarch64-pc-windows-msvc.zip"),
        (os, arch) => Err(format!("uv için desteklenmeyen hedef: {os}/{arch}")),
    }
}

fn deno_asset_url() -> Result<&'static str, String> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Ok("https://github.com/denoland/deno/releases/latest/download/deno-x86_64-unknown-linux-gnu.zip"),
        ("linux", "aarch64") => Ok("https://github.com/denoland/deno/releases/latest/download/deno-aarch64-unknown-linux-gnu.zip"),
        ("windows", "x86_64") => Ok("https://github.com/denoland/deno/releases/latest/download/deno-x86_64-pc-windows-msvc.zip"),
        ("windows", "aarch64") => Ok("https://github.com/denoland/deno/releases/latest/download/deno-aarch64-pc-windows-msvc.zip"),
        (os, arch) => Err(format!("deno için desteklenmeyen hedef: {os}/{arch}")),
    }
}

fn download_uv(dest: &Path) -> Result<(), String> {
    let url = uv_asset_url()?;
    let bytes = http_get_bytes(url)?;
    if url.ends_with(".zip") {
        extract_from_zip(&bytes, "uv.exe", dest)
    } else {
        extract_from_targz(&bytes, "uv", dest)
    }
}

fn download_deno(dest: &Path) -> Result<(), String> {
    let url = deno_asset_url()?;
    let bytes = http_get_bytes(url)?;
    let name = if cfg!(windows) { "deno.exe" } else { "deno" };
    extract_from_zip(&bytes, name, dest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_urls_resolve_for_current_target() {
        assert!(uv_asset_url().unwrap().contains("uv-"));
        assert!(deno_asset_url().unwrap().contains("deno-"));
        assert!(root_dir().ends_with("runtime"));
    }

    /// Gerçek uv + deno + Python 3.12 venv kurulumunu yalıtılmış geçici klasörde doğrular.
    /// Çalıştırma: `cargo test --bin vynz live_bootstrap -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_bootstrap() {
        let tmp = std::env::temp_dir().join(format!("vynz-rt-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        unsafe { std::env::set_var("XDG_DATA_HOME", &tmp) };
        let rt = bootstrap().expect("bootstrap failed");
        assert!(rt.python.is_file(), "python missing: {:?}", rt.python);
        assert!(rt.deno.as_ref().is_some_and(|p| p.is_file()), "deno missing: {:?}", rt.deno);
        let out = Command::new(&rt.python)
            .args(["-I", "-c", "import yt_dlp, ytmusicapi; print(yt_dlp.version.__version__)"])
            .output()
            .expect("venv python failed");
        assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
        println!("venv yt-dlp: {}", String::from_utf8_lossy(&out.stdout).trim());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
