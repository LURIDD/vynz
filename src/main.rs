// vynz: 1. aşama. Arayüz iskeleti, örnek veri ve sahte oynatma saati.
// Gerçek ses (libmpv) 2. aşamada bu saatin yerini alacak.

use std::rc::Rc;
use std::time::Duration;

use slint::{Color, ModelRc, Timer, TimerMode, VecModel};

slint::include_modules!();

fn rgb(hex: u32) -> Color {
    Color::from_rgb_u8((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

fn album(title: &str, artist: &str, c: [u32; 4], kind: i32, accent: i32) -> Album {
    Album {
        title: title.into(),
        artist: artist.into(),
        c1: rgb(c[0]),
        c2: rgb(c[1]),
        c3: rgb(c[2]),
        motif: rgb(c[3]),
        kind,
        accent,
    }
}

fn track(title: &str, artist: &str, album: &str, secs: f32, quality: &str, cover: i32) -> Track {
    Track {
        title: title.into(),
        artist: artist.into(),
        album: album.into(),
        duration: secs,
        quality: quality.into(),
        cover,
    }
}

fn main() -> Result<(), slint::PlatformError> {
    let ui = AppWindow::new()?;

    let albums = vec![
        album("Elektronik Türküler", "Erkin Koray", [0xF4A261, 0xB84028, 0x231016, 0xFFE8C2], 0, 1),
        album("Mordechai", "Khruangbin", [0x065F46, 0x047857, 0x022C22, 0x6EE7B7], 1, 2),
        album("Dünya Yalan Söylüyor", "Mor ve Ötesi", [0x1E3A8A, 0x1D4ED8, 0x090D1A, 0x93C5FD], 2, 3),
        album("Random Access Memories", "Daft Punk", [0x4C1D95, 0x9F1239, 0x0F071E, 0xFB7185], 3, 4),
        album("All Melody", "Nils Frahm", [0x334155, 0x1E293B, 0x0F172A, 0xE2E8F0], 4, 0),
    ];
    let tracks = vec![
        track("Cemalim", "Erkin Koray", "Elektronik Türküler", 228.0, "FLAC 96 kHz", 0),
        track("Time (You and I)", "Khruangbin", "Mordechai", 342.0, "ALAC 24-bit", 1),
        track("Bir Derdim Var", "Mor ve Ötesi", "Dünya Yalan Söylüyor", 214.0, "FLAC 44.1 kHz", 2),
        track("Instant Crush", "Daft Punk", "Random Access Memories", 337.0, "Opus 160", 3),
        track("Sunson", "Nils Frahm", "All Melody", 552.0, "FLAC 48 kHz", 4),
    ];
    let accents: Vec<i32> = tracks.iter().map(|t| albums[t.cover as usize].accent).collect();
    let n = tracks.len() as i32;

    ui.set_albums(ModelRc::new(VecModel::from(albums)));
    ui.set_tracks(ModelRc::new(VecModel::from(tracks.clone())));
    ui.set_queue(ModelRc::new(VecModel::from(tracks[1..].to_vec())));

    let accents = Rc::new(accents);
    let durations: Rc<Vec<f32>> = Rc::new(tracks.iter().map(|t| t.duration).collect());

    let select = {
        let ui = ui.as_weak();
        let accents = accents.clone();
        move |i: i32| {
            let Some(ui) = ui.upgrade() else { return };
            let i = i.rem_euclid(n);
            ui.set_now_index(i);
            ui.set_position(0.0);
            ui.set_playing(true);
            ui.global::<Theme>().set_accent_index(accents[i as usize]);
        }
    };
    select(0);
    ui.set_position(134.0);

    ui.on_play_track({
        let select = select.clone();
        move |i| select(i)
    });
    ui.on_next({
        let ui = ui.as_weak();
        let select = select.clone();
        move || {
            if let Some(ui) = ui.upgrade() {
                select(ui.get_now_index() + 1)
            }
        }
    });
    ui.on_prev({
        let ui = ui.as_weak();
        let select = select.clone();
        move || {
            let Some(ui) = ui.upgrade() else { return };
            if ui.get_position() > 3.0 {
                ui.set_position(0.0);
            } else {
                select(ui.get_now_index() - 1)
            }
        }
    });
    ui.on_toggle_play({
        let ui = ui.as_weak();
        move || {
            if let Some(ui) = ui.upgrade() {
                ui.set_playing(!ui.get_playing())
            }
        }
    });
    ui.on_seek({
        let ui = ui.as_weak();
        let durations = durations.clone();
        move |f| {
            if let Some(ui) = ui.upgrade() {
                ui.set_position(f * durations[ui.get_now_index() as usize])
            }
        }
    });

    // Sahte oynatma saati
    let clock = Timer::default();
    clock.start(TimerMode::Repeated, Duration::from_millis(250), {
        let ui = ui.as_weak();
        let select = select.clone();
        move || {
            let Some(ui) = ui.upgrade() else { return };
            if !ui.get_playing() {
                return;
            }
            let pos = ui.get_position() + 0.25;
            if pos >= durations[ui.get_now_index() as usize] {
                select(ui.get_now_index() + 1);
            } else {
                ui.set_position(pos);
            }
        }
    });

    ui.run()
}
