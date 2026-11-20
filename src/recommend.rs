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

/// Zevkler ekranındaki müzik türleri: (kimlik, görünen ad, örnek şarkı arama sorgusu).
pub const GENRES: &[(&str, &str, &str)] = &[
    ("tr-rock", "Türkçe Rock", "Türkçe rock en iyi şarkılar"),
    ("tr-pop", "Türkçe Pop", "Türkçe pop hit şarkılar"),
    ("tr-rap", "Türkçe Rap", "Türkçe rap hip hop"),
    ("indie", "Indie & Alternatif", "indie alternative songs"),
    ("lofi", "Lo-Fi & Chill", "lofi chill beats"),
    ("synthwave", "Elektronik & Synthwave", "synthwave electronic retrowave"),
    ("metal", "Metal & Hard Rock", "metal hard rock songs"),
    ("hiphop", "Hip-Hop & R&B", "hip hop r&b songs"),
    ("jazz", "Caz & Blues", "jazz blues songs"),
    ("classical", "Klasik & Piyano", "classical piano music"),
    ("arabesk", "Arabesk & Fantazi", "arabesk klasik şarkılar"),
    ("phonk", "Phonk & Drift", "phonk drift music"),
    ("anime", "Anime & Oyun", "anime ost game soundtrack"),
    ("retro", "80'ler & 90'lar", "80s 90s retro hits"),
];

pub fn genre_query(id: &str) -> Option<&'static str> {
    GENRES.iter().find(|(gid, _, _)| *gid == id).map(|(_, _, q)| *q)
}

/// Radyosu alınacak tohum şarkılar ve ağırlıkları (yalnız YouTube Music: radyo oradan).
#[cfg(test)]
pub fn seeds(h: &History) -> Vec<(TrackInfo, f32)> {
    seeds_with_tastes(h, &[])
}

/// Geçmiş ve kullanıcının Zevkler ekranında seçtiği örnek şarkıları harmanlar.
/// Not: Yalnızca Ana Sayfa önerileri içindir; kullanıcının başlattığı Radyo özelliğini etkilemez.
pub fn seeds_with_tastes(h: &History, taste_tracks: &[TrackInfo]) -> Vec<(TrackInfo, f32)> {
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
    let hist_seeds: Vec<(TrackInfo, f32)> = scored
        .into_iter()
        .filter(|(_, t)| artists.insert(artist_id(t)))
        .take(MAX_SEEDS)
        .map(|(s, t)| (t, (s / max).clamp(0.2, 1.0)))
        .collect();

    if taste_tracks.is_empty() {
        return hist_seeds;
    }

    // Dinleme geçmişi azken Zevkler ağırlıklı, geçmiş büyüdükçe geçmiş ağırlıklı (~%70 geçmiş + %30 zevkler).
    let listen_count = h.listens.len() + h.likes.len() * 2;
    let max_from_hist = if listen_count < 6 { 1 } else if listen_count < 20 { 3 } else { 4 };
    let taste_weight = if listen_count < 6 { 0.9 } else if listen_count < 20 { 0.65 } else { 0.45 };

    let mut out = Vec::new();
    let mut used_artists = HashSet::new();
    for (t, w) in hist_seeds.into_iter().take(max_from_hist) {
        if used_artists.insert(artist_id(&t)) {
            out.push((t, w));
        }
    }
    for t in taste_tracks.iter().filter(|t| t.source == Source::Ytm) {
        if out.len() >= MAX_SEEDS {
            break;
        }
        if used_artists.insert(artist_id(t)) {
            out.push((t.clone(), taste_weight));
        }
    }
    out
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
#[cfg(test)]
pub fn build(h: &History, seeds: &[(TrackInfo, f32)], radios: &HashMap<String, Vec<TrackInfo>>) -> Vec<Section> {
    build_with_blocked(h, seeds, radios, &HashSet::new())
}

/// Kişisel raflar + isteğe bağlı "Önerme" sanatçı filtresi (yalnızca Ana Sayfa).
pub fn build_with_blocked(
    h: &History,
    seeds: &[(TrackInfo, f32)],
    radios: &HashMap<String, Vec<TrackInfo>>,
    blocked_artists: &HashSet<String>,
) -> Vec<Section> {
    let mut out = Vec::new();
    let artist_score = h.artist_scores();
    let skipped = h.recently_skipped(14);
    let disliked_tracks = h.disliked_track_keys();
    let disliked_artists = h.disliked_artist_ids();
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
            let aid = artist_id(t);
            if seed_keys.contains(&key) || skipped.contains(&key) || disliked_tracks.contains(&key) || disliked_artists.contains(&aid) {
                continue;
            }
            if !blocked_artists.is_empty() && blocked_artists.contains(&aid) && !known_artists.contains(&aid) {
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
        // Geçmiş henüz yoksa tohumlar YouTube Music'in sana özel önerilerinden gelir.
        let title = if h.is_empty() { "Bugün için karışım" } else { "Senin için karışım" };
        out.push(Section { style: 0, cards: Vec::new(), title: title.into(), items: mix });
    }

    // Kaldığın yerden
    let recent = h.recent(14);
    if !recent.is_empty() {
        out.push(Section { style: 0, cards: Vec::new(), title: "Kaldığın yerden".into(), items: recent });
    }

    // "<Sanatçı> ve benzerleri": en sevilen iki sanatçının radyosu
    for (seed, _) in seeds.iter().take(2) {
        let Some(list) = radios.get(&seed.key()) else { continue };
        let items = diversify(list.iter().map(|t| (0.0, t.clone())).filter(|(_, t)| !skipped.contains(&t.key())).collect(), 2, 14);
        if items.len() >= 4 && !h.is_empty() {
            let name = seed.artist.split(',').next().unwrap_or("").trim().to_string();
            out.push(Section { style: 0, cards: Vec::new(), title: format!("{name} ve benzerleri"), items });
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
        out.push(Section { style: 0, cards: Vec::new(), title: "Yeni keşifler".into(), items: discover });
    }

    // En çok dinlediklerin
    let mut top: Vec<(f32, TrackInfo)> = h.track_scores().into_values().filter(|(s, _)| *s > 0.5).collect();
    top.sort_by(|a, b| b.0.total_cmp(&a.0));
    let top: Vec<TrackInfo> = top.into_iter().take(14).map(|(_, t)| t).collect();
    if top.len() >= 3 {
        out.push(Section { style: 0, cards: Vec::new(), title: "En çok dinlediklerin".into(), items: top });
    }

    if !h.likes.is_empty() {
        out.push(Section { style: 0, cards: Vec::new(), title: "Beğendiklerin".into(), items: h.likes.iter().take(20).cloned().collect() });
    }
    out
}

/// Hızlı erişim kutucukları: son dinlenenler + en sevilenler + karışımdan, 16 tane.
pub fn quick_picks(h: &History, personal: &[Section], fallback: &[Section]) -> Vec<TrackInfo> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let mut push = |t: &TrackInfo, out: &mut Vec<TrackInfo>| {
        if out.len() < 16 && !h.is_disliked(t) && seen.insert(t.key()) {
            out.push(t.clone());
        }
    };
    for t in h.recent(6) {
        push(&t, &mut out);
    }
    for s in personal.iter().chain(fallback.iter()) {
        for t in s.items.iter().take(4) {
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

    #[test]
    fn tastes_blend_with_history_and_block_filters_artists() {
        let h = History::default();
        let tastes = vec![tr("t1", "RockBand"), tr("t2", "SynthDuo")];
        let s = seeds_with_tastes(&h, &tastes);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].0.id, "t1");

        let radio = vec![tr("good", "IndieAct"), tr("blocked_song", "UnwantedArtist")];
        let mut radios = HashMap::new();
        radios.insert(tr("t1", "RockBand").key(), radio);
        let mut blocked = HashSet::new();
        blocked.insert(artist_id(&tr("x", "UnwantedArtist")));
        let secs = build_with_blocked(&h, &s, &radios, &blocked);
        let mix = &secs.iter().find(|s| s.title == "Bugün için karışım").unwrap().items;
        assert!(mix.iter().any(|t| t.id == "good"));
        assert!(!mix.iter().any(|t| t.id == "blocked_song"));
    }
}
