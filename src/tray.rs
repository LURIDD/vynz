// Windows sistem tepsisi (tray): çarpıya basınca müzik kesilmeden arka planda devam eder.
// Linux'ta MPRIS/Quickshell kullanıldığı için no-op olarak derlenir.

#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub enum TrayAct {
    Show,
    PlayPause,
    Next,
    Quit,
}

#[cfg(target_os = "windows")]
mod imp {
    use std::thread;

    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

    use super::TrayAct;

    const ICON_PNG: &[u8] = include_bytes!("../design/logo/vynz-32.png");

    pub struct Tray {
        _icon: TrayIcon,
    }

    impl Tray {
        pub fn start(on_act: impl Fn(TrayAct) + Send + 'static) -> Option<Tray> {
            let img = image::load_from_memory(ICON_PNG).ok()?.to_rgba8();
            let (w, h) = img.dimensions();
            let icon = Icon::from_rgba(img.into_raw(), w, h).ok()?;

            let show = MenuItem::new("Vynz'i göster", true, None);
            let play = MenuItem::new("Oynat / Duraklat", true, None);
            let next = MenuItem::new("Sonraki şarkı", true, None);
            let quit = MenuItem::new("Çıkış", true, None);

            let show_id = show.id().clone();
            let play_id = play.id().clone();
            let next_id = next.id().clone();
            let quit_id = quit.id().clone();

            let menu = Menu::new();
            let _ = menu.append(&show);
            let _ = menu.append(&PredefinedMenuItem::separator());
            let _ = menu.append(&play);
            let _ = menu.append(&next);
            let _ = menu.append(&PredefinedMenuItem::separator());
            let _ = menu.append(&quit);

            let tray = TrayIconBuilder::new()
                .with_tooltip("Vynz")
                .with_icon(icon)
                .with_menu(Box::new(menu))
                .with_menu_on_left_click(false)
                .build()
                .ok()?;

            let menu_rx = MenuEvent::receiver().clone();
            let tray_rx = TrayIconEvent::receiver().clone();
            let on_act = std::sync::Arc::new(on_act);

            let act_m = on_act.clone();
            thread::Builder::new()
                .name("vynz-tray-menu".into())
                .spawn(move || {
                    while let Ok(ev) = menu_rx.recv() {
                        if ev.id == show_id {
                            act_m(TrayAct::Show);
                        } else if ev.id == play_id {
                            act_m(TrayAct::PlayPause);
                        } else if ev.id == next_id {
                            act_m(TrayAct::Next);
                        } else if ev.id == quit_id {
                            act_m(TrayAct::Quit);
                        }
                    }
                })
                .ok();

            thread::Builder::new()
                .name("vynz-tray-click".into())
                .spawn(move || {
                    while let Ok(ev) = tray_rx.recv() {
                        match ev {
                            TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. }
                            | TrayIconEvent::DoubleClick { button: MouseButton::Left, .. } => {
                                on_act(TrayAct::Show);
                            }
                            _ => {}
                        }
                    }
                })
                .ok();

            Some(Tray { _icon: tray })
        }
    }
}

#[cfg(not(target_os = "windows"))]
#[allow(dead_code)]
mod imp {
    use super::TrayAct;

    pub struct Tray;

    impl Tray {
        pub fn start(_on_act: impl Fn(TrayAct) + Send + 'static) -> Option<Tray> {
            None
        }
    }
}

#[allow(unused_imports)]
pub use imp::Tray;
