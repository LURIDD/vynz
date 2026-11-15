// Öneri algoritması: dinleme geçmişinden kişisel ana sayfa rafları.
//
// 1. Tohumlar: puanı en yüksek şarkılar (her sanatçıdan en çok bir tane, en çok 5).
// 2. Her tohum için YouTube Music radyosu (benzer şarkılar).
// 3. Aday puanı = Σ tohum ağırlığı × radyodaki yakınlık
//                 + sevilen sanatçı bonusu − sevilmeyen sanatçı cezası
//                 − yakın zamanda çalındıysa ceza; atlananlar elenir.
// 4. Çeşitlilik: her sanatçıdan en çok 2 şarkı.

use std::collections::{HashMap, HashSet};

use crate::history::{artist_id, History};
use crate::model::{Section, Source, TrackInfo};

pub const MAX_SEEDS: usize = 5;

/// Radyosu alınacak tohum şarkılar ve ağırlıkları (yalnız YouTube Music: radyo oradan).
pub fn seeds(h: &History) -> Vec<(TrackInfo, f32)> {
    let mut scored: Vec<(f32, TrackInfo)> = h.track_scores().into_values().filter(|(s, t)| *s > 0.0 && t.source == Source::Ytm).collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    // En son dinlenen şarkı da güncel ruh halini yansıtır.
    if let Some(last) = h.recent(1).into_iter().next().filter(|t| t.source == Source::Ytm) {
        if !scored.iter().take(MAX_SEEDS).any(|(_, t)| t.key() == last.key()) {
            scored.insert(0, (scored.first().map(|s| s.0).unwrap_or(1.0), last));
        }
    }
    let mut artists = HashSet::new();
    let max = scored.first().map(|s| s.0).unwrap_or(1.0).max(0.01);
    scored
        .into_iter()
        .filter(|(_, t)| artists.insert(artist_id(t)))
        .take(MAX_SEEDS)
        .map(|(s, t)| (t, (s / max).clamp(0.2, 1.0)))
        .collect()
}

fn diversify(list: Vec<(f32, TrackInfo)>, per_artist: usize, n: usize) -> Vec<TrackInfo> {
    let mut count: HashMap<String, usize> = HashMap::new();
    let mut out = Vec::new();
    for (_, t) in list {
        let c = count.entry(artist_id(&t)).or_insert(0);
        if *c >= per_artist {
            continue;
        }
        *c += 1;
        out.push(t);
        if out.len() >= n {
            break;
        }
    }
    out
}

/// Kişisel raflar. `radios`: tohum anahtarı -> o şarkının radyosu.
pub fn build(h: &History, seeds: &[(TrackInfo, f32)], radios: &HashMap<String, Vec<TrackInfo>>) -> Vec<Section> {
    let mut out = Vec::new();
    if h.is_empty() {
        return out;
    }
    let artist_score = h.artist_scores();
    let skipped = h.recently_skipped(14);
    let fresh = h.played_since(36);
    let seed_keys: HashSet<String> = seeds.iter().map(|s| s.0.key()).collect();
    let known_artists: HashSet<&String> = artist_score.iter().filter(|(_, s)| **s > 0.0).map(|(a, _)| a).collect();

    // Aday havuzu
    let mut pool: HashMap<String, (f32, usize, TrackInfo)> = HashMap::new();
    for (seed, w) in seeds {
        let Some(list) = radios.get(&seed.key()) else { continue };
        let len = list.len().max(1) as f32;
        for (rank, t) in list.iter().enumerate() {
            let key = t.key();
            if seed_keys.contains(&key) || skipped.contains(&key) {
                continue;
            }
            let e = pool.entry(key).or_insert((0.0, 0, t.clone()));
            e.0 += w * (1.0 - rank as f32 / len);
            e.1 += 1;
        }
    }
    let mut ranked: Vec<(f32, TrackInfo)> = pool
        .values()
        .map(|(s, hits, t)| {
            let a = artist_score.get(&artist_id(t)).copied().unwrap_or(0.0);
            let mut score = *s + 0.15 * (*hits as f32 - 1.0);
            if a > 0.0 {
                score += 0.25 * (a / 5.0).min(1.0);
            } else if a < -0.5 {
                score -= 0.5;
            }
            if fresh.contains(&t.key()) {
                score *= 0.4;
            }
            (score, t.clone())
        })
        .collect();
    ranked.sort_by(|a, b| b.0.total_cmp(&a.0));

    // Senin için karışım
    let mix = diversify(ranked.clone(), 2, 20);
    let mix_keys: HashSet<String> = mix.iter().map(TrackInfo::key).collect();
    if !mix.is_empty() {
        out.push(Section { style: 0, title: "Senin için karışım".into(), items: mix });
    }

    // Kaldığın yerden
    let recent = h.recent(14);
    if !recent.is_empty() {
        out.push(Section { style: 0, title: "Kaldığın yerden".into(), items: recent });
    }

    // "<Sanatçı> ve benzerleri": en sevilen iki sanatçının radyosu
    for (seed, _) in seeds.iter().take(2) {
        let Some(list) = radios.get(&seed.key()) else { continue };
        let items = diversify(list.iter().map(|t| (0.0, t.clone())).filter(|(_, t)| !skipped.contains(&t.key())).collect(), 2, 14);
        if items.len() >= 4 {
            let name = seed.artist.split(',').next().unwrap_or("").trim().to_string();
            out.push(Section { style: 0, title: format!("{name} ve benzerleri"), items });
        }
    }

    // Yeni keşifler: hiç dinlemediğin sanatçılar, karışımda olmayanlar
    let discover = diversify(
        ranked
            .iter()
            .filter(|(_, t)| !known_artists.contains(&artist_id(t)) && !mix_keys.contains(&t.key()))
            .cloned()
            .collect(),
        1,
        16,
    );
    if discover.len() >= 4 {
        out.push(Section { style: 0, title: "Yeni keşifler".into(), items: discover });
    }

    // En çok dinlediklerin
    let mut top: Vec<(f32, TrackInfo)> = h.track_scores().into_values().filter(|(s, _)| *s > 0.5).collect();
    top.sort_by(|a, b| b.0.total_cmp(&a.0));
    let top: Vec<TrackInfo> = top.into_iter().take(14).map(|(_, t)| t).collect();
    if top.len() >= 3 {
        out.push(Section { style: 0, title: "En çok dinlediklerin".into(), items: top });
    }

    if !h.likes.is_empty() {
        out.push(Section { style: 0, title: "Beğendiklerin".into(), items: h.likes.iter().take(20).cloned().collect() });
    }
    out
}

/// Hızlı seçim kutucukları: son dinlenenler + en sevilenler + karışımdan, 8 tane.
pub fn quick_picks(h: &History, personal: &[Section], fallback: &[Section]) -> Vec<TrackInfo> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let mut push = |t: &TrackInfo, out: &mut Vec<TrackInfo>| {
        if out.len() < 8 && seen.insert(t.key()) {
            out.push(t.clone());
        }
    };
    for t in h.recent(4) {
        push(&t, &mut out);
    }
    for s in personal.iter().chain(fallback.iter()) {
        for t in s.items.iter().take(3) {
            push(t, &mut out);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::Listen;

    fn tr(id: &str, artist: &str) -> TrackInfo {
        TrackInfo { source: Source::Ytm, id: id.into(), title: id.into(), artist: artist.into(), album: String::new(), duration: 200.0, thumb: String::new(), url: None }
    }

    fn listen(t: TrackInfo, played: f32) -> Listen {
        Listen { ts: crate::history::now(), track: t, played }
    }

    #[test]
    fn seeds_are_diverse_and_skips_ignored() {
        let mut h = History::default();
        for _ in 0..3 {
            h.listens.push(listen(tr("a1", "A"), 200.0));
            h.listens.push(listen(tr("a2", "A"), 200.0));
        }
        h.listens.push(listen(tr("b1", "B"), 190.0));
        h.listens.push(listen(tr("c1", "C"), 5.0)); // atlandı
        let s = seeds(&h);
        let artists: Vec<String> = s.iter().map(|(t, _)| t.artist.clone()).collect();
        assert!(artists.contains(&"A".to_string()) && artists.contains(&"B".to_string()));
        assert!(!artists.contains(&"C".to_string()));
        assert_eq!(artists.iter().filter(|a| *a == "A").count(), 1);
    }

    #[test]
    fn mix_excludes_skipped_and_limits_artists() {
        let mut h = History::default();
        h.listens.push(listen(tr("seed", "S"), 200.0));
        h.listens.push(listen(tr("bad", "X"), 4.0));
        let radio = vec![tr("seed", "S"), tr("bad", "X"), tr("r1", "R"), tr("r2", "R"), tr("r3", "R"), tr("q1", "Q")];
        let mut radios = HashMap::new();
        radios.insert(tr("seed", "S").key(), radio);
        let secs = build(&h, &[(tr("seed", "S"), 1.0)], &radios);
        let mix = &secs.iter().find(|s| s.title == "Senin için karışım").unwrap().items;
        assert!(!mix.iter().any(|t| t.id == "bad" || t.id == "seed"));
        assert!(mix.iter().filter(|t| t.artist == "R").count() <= 2);
        assert_eq!(mix.first().map(|t| t.id.as_str()), Some("r1"));
    }
}
