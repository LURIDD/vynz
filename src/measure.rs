// Metin genişliği ölçümü: aktif söz satırını gerçek genişliğe göre satırlara bölmek için.
// Slint Rust tarafına metin ölçümü vermiyor; aynı yazı tipi dosyalarından ölçüyoruz.

use ttf_parser::{Face, Tag};

static JAKARTA: &[u8] = include_bytes!("../ui/fonts/PlusJakartaSans.ttf");
static SERIF_ITALIC: &[u8] = include_bytes!("../ui/fonts/InstrumentSerif-Italic.ttf");
static MANROPE: &[u8] = include_bytes!("../ui/fonts/Manrope.ttf");

/// Söz stilleri (ui/parts.slint LyricStyle ile aynı): yazı tipi, kalınlık, harf aralığı (px).
const STYLES: [(usize, f32, f32); 3] = [(0, 800.0, -1.2), (1, 400.0, 0.0), (2, 600.0, -0.4)];

thread_local! {
    static FACES: Vec<Face<'static>> = [JAKARTA, SERIF_ITALIC, MANROPE]
        .iter()
        .filter_map(|d| Face::parse(d, 0).ok())
        .collect();
}

/// LyricStyle.size ile aynı: piksel cinsinden yazı boyutu.
pub fn font_size(style: i32, size: i32) -> f32 {
    let base = match size {
        0 => 30.0,
        2 => 46.0,
        _ => 38.0,
    };
    if style == 1 { base * 1.18 } else { base }
}

/// `text`in mantıksal piksel genişliği.
pub fn width(style: i32, text: &str, px: f32) -> f32 {
    let (fi, wght, spacing) = STYLES[style.clamp(0, 2) as usize];
    FACES.with(|faces| {
        let Some(face) = faces.get(fi) else { return text.chars().count() as f32 * px * 0.55 };
        let mut face = face.clone();
        let _ = face.set_variation(Tag::from_bytes(b"wght"), wght);
        let scale = px / face.units_per_em() as f32;
        text.chars()
            .map(|c| {
                let adv = face.glyph_index(c).and_then(|g| face.glyph_hor_advance(g)).unwrap_or((face.units_per_em() / 2) as u16);
                adv as f32 * scale + spacing
            })
            .sum()
    })
}

/// Kelimeleri `max` genişliğe sığacak satırlara böl (açgözlü; Slint'in kelime kaydırmasıyla aynı mantık).
/// Dönen: her kelimenin satır numarası.
pub fn wrap(style: i32, words: &[&str], px: f32, gap: f32, max: f32) -> Vec<usize> {
    let mut rows = Vec::with_capacity(words.len());
    let (mut row, mut used) = (0usize, 0f32);
    for w in words {
        let ww = width(style, w, px);
        if used > 0.0 && used + gap + ww > max {
            row += 1;
            used = ww;
        } else {
            used += if used > 0.0 { gap + ww } else { ww };
        }
        rows.push(row);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths_are_sane_and_wrap_breaks() {
        let w = width(0, "Mockingbird", 38.0);
        assert!(w > 150.0 && w < 320.0, "{w}");
        assert!(width(0, "Mockingbird", 38.0) > width(2, "Mockingbird", 38.0) * 0.9);
        let words = ["But", "it's", "just", "something", "we", "have", "no", "control", "over"];
        let r = wrap(0, &words, 38.0, 10.0, 300.0);
        assert!(*r.last().unwrap() >= 2);
        assert_eq!(r[0], 0);
    }
}

