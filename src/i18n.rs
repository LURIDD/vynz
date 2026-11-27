//! Arayüz dili. Kaynak metinler Türkçe; çeviriler `lang/<dil>/LC_MESSAGES/vynz.po`.
//! Slint tarafı aynı dosyayı derlemede gömer (`@tr`), Rust tarafı burada okur.
//! Biçim: `{}` sırayla, `{n}` sayı (çoğul biçimde) yer tutucusu.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

/// Desteklenen diller: (kod, gömülü .po). Kod Slint'in gömülü çeviri adıyla
/// (lang/<kod>/) aynıdır; "tr" kaynak dildir, dosyası yok.
const LANGS: &[(&str, &str)] = &[
    ("tr", ""),
    ("en", include_str!("../lang/en/LC_MESSAGES/vynz.po")),
    ("pt_BR", include_str!("../lang/pt_BR/LC_MESSAGES/vynz.po")),
];

/// LANGS içindeki sıra (0 Türkçe, kaynak)
static LANG: AtomicU8 = AtomicU8::new(0);

struct Catalog {
    one: HashMap<String, String>,
    /// msgid -> (tekil, çoğul)
    plural: HashMap<String, (String, String)>,
}

fn catalog(i: usize) -> &'static Catalog {
    static C: OnceLock<Vec<Catalog>> = OnceLock::new();
    &C.get_or_init(|| LANGS.iter().map(|(_, po)| parse_po(po)).collect())[i]
}

/// Ayardaki değer ("" sistem, ya da dil kodu) ve sistem dili ile etkin dil kodu.
pub fn resolve(setting: &str) -> &'static str {
    if let Some((code, _)) = LANGS.iter().find(|(c, _)| *c == setting) {
        return code;
    }
    let sys = sys_locale::get_locale().unwrap_or_default().to_lowercase();
    if sys.starts_with("tr") {
        "tr"
    } else if sys.starts_with("pt") {
        "pt_BR"
    } else {
        "en"
    }
}

/// Etkin dili ayarla (Slint çevirisi dahil).
pub fn set(lang: &str) {
    let i = LANGS.iter().position(|(c, _)| *c == lang).unwrap_or(1);
    LANG.store(i as u8, Ordering::Relaxed);
    // Slint: "" kaynak dil (Türkçe), diğerleri gömülü çeviri
    let _ = slint::select_bundled_translation(if i == 0 { "" } else { LANGS[i].0 });
}

/// Etkin dil kodu ("tr", "en", "pt_BR").
pub fn lang() -> &'static str {
    LANGS[LANG.load(Ordering::Relaxed) as usize].0
}

pub fn is_tr() -> bool {
    LANG.load(Ordering::Relaxed) == 0
}

/// Sistem bölgesi (iki harf, büyük): "en-US" -> "US". İçerik konumu için.
pub fn region() -> Option<String> {
    let l = sys_locale::get_locale()?;
    let r = l.split(['-', '_']).nth(1)?;
    let r = r.split('.').next()?;
    (r.len() == 2 && r.chars().all(|c| c.is_ascii_alphabetic())).then(|| r.to_ascii_uppercase())
}

/// YouTube Music içerik dili ve bölgesi (ytmusicapi): Türkçede TR, diğerlerinde sistem bölgesi.
pub fn content() -> (&'static str, String) {
    match lang() {
        "tr" => ("tr", "TR".into()),
        "pt_BR" => ("pt", region().unwrap_or_else(|| "BR".into())),
        _ => ("en", region().unwrap_or_else(|| "US".into())),
    }
}

/// Tek metin çevirisi.
pub fn t(s: &str) -> String {
    let i = LANG.load(Ordering::Relaxed) as usize;
    if i > 0 {
        if let Some(x) = catalog(i).one.get(s) {
            return x.clone();
        }
    }
    s.to_string()
}

/// Çoğul metin: `{n}` sayıyla değişir.
pub fn plural(s: &str, n: i64) -> String {
    let i = LANG.load(Ordering::Relaxed) as usize;
    let f = if i > 0 {
        catalog(i).plural.get(s).map(|(a, b)| if n == 1 { a.as_str() } else { b.as_str() }).unwrap_or(s)
    } else {
        s
    };
    f.replace("{n}", &n.to_string())
}

/// `{}` yer tutucularını sırayla doldur (`{0}`, `{1}` de olur).
pub fn fill(f: &str, args: &[String]) -> String {
    let mut out = String::with_capacity(f.len() + 16);
    let mut next = 0;
    let mut rest = f;
    while let Some(i) = rest.find('{') {
        out.push_str(&rest[..i]);
        let tail = &rest[i + 1..];
        match tail.find('}') {
            Some(j) if j == 0 || tail[..j].chars().all(|c| c.is_ascii_digit()) => {
                let idx = if j == 0 { let k = next; next += 1; k } else { tail[..j].parse().unwrap_or(0) };
                out.push_str(args.get(idx).map(String::as_str).unwrap_or(""));
                rest = &tail[j + 1..];
            }
            _ => {
                out.push('{');
                rest = tail;
            }
        }
    }
    out.push_str(rest);
    out
}

/// `tr!("metin")`, `tr!("{} şarkı eklendi", n)`
#[macro_export]
macro_rules! tr {
    ($s:literal) => { $crate::i18n::t($s) };
    ($s:literal, $($a:expr),+ $(,)?) => { $crate::i18n::fill(&$crate::i18n::t($s), &[$(format!("{}", $a)),+]) };
}

/// `trn!("{n} şarkı", sayı)`
#[macro_export]
macro_rules! trn {
    ($s:literal, $n:expr) => { $crate::i18n::plural($s, ($n) as i64) };
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    let s = s.strip_prefix('"').and_then(|x| x.strip_suffix('"')).unwrap_or(s);
    let mut out = String::new();
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(o) => out.push(o),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn parse_po(src: &str) -> Catalog {
    let mut one = HashMap::new();
    let mut plural = HashMap::new();
    let (mut id, mut idp, mut s0, mut s1, mut s) = (String::new(), None::<String>, String::new(), String::new(), String::new());
    let mut cur: u8 = 0; // 1 id, 2 idp, 3 str, 4 str0, 5 str1
    let mut ctx = false; // bağlamlı girdi (yalnız Slint kullanır)
    let mut flush = |id: &mut String, idp: &mut Option<String>, s: &mut String, s0: &mut String, s1: &mut String| {
        if !id.is_empty() {
            if idp.is_some() {
                if !s0.is_empty() {
                    plural.insert(id.clone(), (s0.clone(), if s1.is_empty() { s0.clone() } else { s1.clone() }));
                }
            } else if !s.is_empty() {
                one.insert(id.clone(), s.clone());
            }
        }
        id.clear();
        *idp = None;
        s.clear();
        s0.clear();
        s1.clear();
    };
    for line in src.lines() {
        let l = line.trim();
        if l.starts_with('#') || l.is_empty() {
            continue;
        }
        if l.starts_with("msgctxt ") {
            flush(&mut id, &mut idp, &mut s, &mut s0, &mut s1);
            ctx = true;
            cur = 0;
            continue;
        }
        if let Some(r) = l.strip_prefix("msgid_plural ") {
            idp = Some(unquote(r));
            cur = 2;
        } else if let Some(r) = l.strip_prefix("msgid ") {
            if !ctx {
                flush(&mut id, &mut idp, &mut s, &mut s0, &mut s1);
            }
            id = if ctx { String::new() } else { unquote(r) };
            ctx = false;
            cur = 1;
        } else if let Some(r) = l.strip_prefix("msgstr[0] ") {
            s0 = unquote(r);
            cur = 4;
        } else if let Some(r) = l.strip_prefix("msgstr[1] ") {
            s1 = unquote(r);
            cur = 5;
        } else if let Some(r) = l.strip_prefix("msgstr ") {
            s = unquote(r);
            cur = 3;
        } else if l.starts_with('"') {
            let x = unquote(l);
            match cur {
                1 => id.push_str(&x),
                2 => idp.get_or_insert_with(String::new).push_str(&x),
                3 => s.push_str(&x),
                4 => s0.push_str(&x),
                5 => s1.push_str(&x),
                _ => {}
            }
        }
    }
    flush(&mut id, &mut idp, &mut s, &mut s0, &mut s1);
    Catalog { one, plural }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn po_and_fill() {
        let c = parse_po("msgid \"\"\nmsgstr \"Content-Type: x\\n\"\n\nmsgid \"Kuyruk\"\nmsgstr \"Queue\"\n\nmsgid \"{n} şarkı\"\nmsgid_plural \"{n} şarkı\"\nmsgstr[0] \"{n} song\"\nmsgstr[1] \"{n} songs\"\n\nmsgid \"uzun \"\n\"satır\"\nmsgstr \"long \"\n\"line\"\n");
        assert_eq!(c.one["Kuyruk"], "Queue");
        assert_eq!(c.one["uzun satır"], "long line");
        assert_eq!(c.plural["{n} şarkı"].1, "{n} songs");
        assert!(!c.one.contains_key(""));
        assert_eq!(fill("{} / {}", &["1".into(), "2".into()]), "1 / 2");
        assert_eq!(fill("{1} {0} {x}", &["a".into(), "b".into()]), "b a {x}");
    }

    #[test]
    fn catalogs_parse() {
        for i in 1..LANGS.len() {
            let c = catalog(i);
            assert!(c.one.len() > 300, "{}: {}", LANGS[i].0, c.one.len());
            for (k, v) in &c.one {
                assert_eq!(k.matches("{}").count(), v.matches("{}").count(), "{} yer tutucu: {k}", LANGS[i].0);
            }
            for (k, (a, b)) in &c.plural {
                assert!(a.contains("{n}") && b.contains("{n}"), "{} çoğul: {k}", LANGS[i].0);
            }
        }
    }
}
