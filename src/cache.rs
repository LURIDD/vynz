// Ses önbelleği: çalınan şarkı, mpv'nin zaten indirdiği veriden diske kaydedilir
// (stream-record). Ek indirme yoktur. Şarkı sonuna kadar atlamadan çalındıysa dosya
// tamamlanmış sayılır; yarım kalan ya da ileri sarılan kayıtlar silinir.
// Boyut sınırı aşılınca en uzun süredir çalınmayanlar silinir.

use std::path::PathBuf;
use std::time::SystemTime;

/// Bundan küçük kayıt bozuk sayılır.
const MIN_BYTES: u64 = 64 * 1024;

pub fn dir() -> PathBuf {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or(home.join(".cache")).join("vynz").join("audio")
}

fn safe(key: &str) -> String {
    key.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
}

pub fn final_path(key: &str) -> PathBuf {
    dir().join(format!("{}.mka", safe(key)))
}

pub fn part_path(key: &str) -> PathBuf {
    dir().join(format!("{}.part.mka", safe(key)))
}

/// Kayıt var mı (süreye dokunmaz; ön-hazırlık kontrolleri için).
pub fn exists(key: &str) -> Option<PathBuf> {
    let p = final_path(key);
    (std::fs::metadata(&p).ok()?.len() >= MIN_BYTES).then_some(p)
}

/// Tamamlanmış kayıt varsa yolunu döner ve "son kullanım" zamanını tazeler (çalarken kullan).
pub fn lookup(key: &str) -> Option<PathBuf> {
    let p = final_path(key);
    let len = std::fs::metadata(&p).ok()?.len();
    if len < MIN_BYTES {
        return None;
    }
    if let Ok(f) = std::fs::File::options().append(true).open(&p) {
        let _ = f.set_modified(SystemTime::now());
    }
    Some(p)
}

/// Kayıt yeri hazırla (önbellek kapalıysa None).
pub fn prepare(key: &str, enabled: bool) -> Option<PathBuf> {
    if !enabled {
        return None;
    }
    std::fs::create_dir_all(dir()).ok()?;
    let p = part_path(key);
    let _ = std::fs::remove_file(&p);
    Some(p)
}

/// Kayıt bitti: geçerliyse kalıcı yap, değilse sil.
pub fn finish(key: &str, valid: bool) {
    let part = part_path(key);
    let ok = valid && std::fs::metadata(&part).map(|m| m.len() >= MIN_BYTES).unwrap_or(false);
    if ok {
        let _ = std::fs::rename(&part, final_path(key));
    } else {
        let _ = std::fs::remove_file(&part);
    }
}

pub fn usage() -> u64 {
    std::fs::read_dir(dir())
        .map(|r| r.filter_map(|e| e.ok()?.metadata().ok()).map(|m| m.len()).sum())
        .unwrap_or(0)
}

/// Sınırı aşan eski kayıtları sil (yarım kalmış .part dosyaları da temizlenir).
pub fn enforce(limit: u64) {
    let Ok(rd) = std::fs::read_dir(dir()) else { return };
    let mut files: Vec<(PathBuf, u64, SystemTime)> = Vec::new();
    for e in rd.flatten() {
        let p = e.path();
        let Ok(m) = e.metadata() else { continue };
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.ends_with(".part.mka") {
            // Eski yarım kayıtlar (1 saatten eski) temizlenir.
            if m.modified().ok().and_then(|t| t.elapsed().ok()).is_some_and(|d| d.as_secs() > 3600) {
                let _ = std::fs::remove_file(&p);
            }
            continue;
        }
        files.push((p, m.len(), m.modified().unwrap_or(SystemTime::UNIX_EPOCH)));
    }
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    if total <= limit {
        return;
    }
    files.sort_by_key(|f| f.2); // en eski önce
    for (p, len, _) in files {
        if total <= limit {
            break;
        }
        if std::fs::remove_file(&p).is_ok() {
            total = total.saturating_sub(len);
        }
    }
}

/// `days` gündür çalınmayan kayıtları sil (0 = süre sınırı yok).
pub fn expire(days: u32) {
    if days == 0 {
        return;
    }
    let max_age = days as u64 * 86400;
    let Ok(rd) = std::fs::read_dir(dir()) else { return };
    for e in rd.flatten() {
        let Ok(m) = e.metadata() else { continue };
        let old = m.modified().ok().and_then(|t| t.elapsed().ok()).is_some_and(|d| d.as_secs() > max_age);
        if old {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

pub fn clear() {
    let _ = std::fs::remove_dir_all(dir());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    #[test]
    fn expire_removes_only_old_and_lookup_refreshes() {
        let tmp = std::env::temp_dir().join(format!("vynz-cache-test-{}", std::process::id()));
        // SAFETY: tek iş parçacıklı test; başka test bu değişkene bakmıyor.
        unsafe { std::env::set_var("XDG_CACHE_HOME", &tmp) };
        std::fs::create_dir_all(dir()).unwrap();
        for k in ["eski", "yeni", "eski-ama-calindi"] {
            std::fs::write(final_path(k), vec![0u8; MIN_BYTES as usize]).unwrap();
        }
        let old = SystemTime::now() - Duration::from_secs(40 * 86400);
        for k in ["eski", "eski-ama-calindi"] {
            std::fs::File::options().append(true).open(final_path(k)).unwrap().set_modified(old).unwrap();
        }
        assert!(exists("eski").is_some());
        assert!(lookup("eski-ama-calindi").is_some()); // çalındı: süre yenilenir
        expire(30);
        assert!(exists("eski").is_none());
        assert!(exists("yeni").is_some());
        assert!(exists("eski-ama-calindi").is_some());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
