// Sistem tepsisi (tray): çarpıya basınca müzik kesilmeden arka planda devam eder.
// Windows: tray-icon. Linux: StatusNotifierItem (ksni, D-Bus); tepsi sunan bir panel
// (StatusNotifierWatcher) yoksa başlatılamaz ve çarpı uygulamayı kapatır.

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
        pub fn start(on_act: impl Fn(TrayAct) + Send + Sync + 'static) -> Option<Tray> {
            let img = image::load_from_memory(ICON_PNG).ok()?.to_rgba8();
            let (w, h) = img.dimensions();
            let icon = Icon::from_rgba(img.into_raw(), w, h).ok()?;

            let show = MenuItem::new(tr!("Vynz'i göster"), true, None);
            let play = MenuItem::new("Oynat / Duraklat", true, None);
            let next = MenuItem::new(tr!("Sonraki şarkı"), true, None);
            let quit = MenuItem::new(tr!("Çıkış"), true, None);

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

#[cfg(target_os = "linux")]
mod imp {
    use std::sync::Arc;

    use ksni::blocking::{Handle, TrayMethods};

    use super::TrayAct;

    const ICON_PNG: &[u8] = include_bytes!("../design/logo/vynz-32.png");

    type OnAct = Arc<dyn Fn(TrayAct) + Send + Sync>;

    struct Sni {
        on_act: OnAct,
        icon: Vec<ksni::Icon>,
    }

    impl ksni::Tray for Sni {
        fn id(&self) -> String {
            "vynz".into()
        }
        fn title(&self) -> String {
            "Vynz".into()
        }
        fn icon_name(&self) -> String {
            "vynz".into()
        }
        fn icon_pixmap(&self) -> Vec<ksni::Icon> {
            self.icon.clone()
        }
        fn tool_tip(&self) -> ksni::ToolTip {
            ksni::ToolTip { title: "Vynz".into(), ..Default::default() }
        }
        fn activate(&mut self, _x: i32, _y: i32) {
            (self.on_act)(TrayAct::Show);
        }
        fn secondary_activate(&mut self, _x: i32, _y: i32) {
            (self.on_act)(TrayAct::PlayPause);
        }
        fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
            use ksni::menu::StandardItem;
            let item = |label: String, act: TrayAct| -> ksni::MenuItem<Self> {
                StandardItem { label, activate: Box::new(move |t: &mut Self| (t.on_act)(act)), ..Default::default() }
                    .into()
            };
            vec![
                item(tr!("Vynz'i göster"), TrayAct::Show),
                ksni::MenuItem::Separator,
                item("Oynat / Duraklat".into(), TrayAct::PlayPause),
                item(tr!("Sonraki şarkı"), TrayAct::Next),
                ksni::MenuItem::Separator,
                item(tr!("Çıkış"), TrayAct::Quit),
            ]
        }
    }

    /// SNI simgesi ARGB32, ağ bayt sırası (büyük uçlu).
    fn icon() -> Vec<ksni::Icon> {
        let Ok(img) = image::load_from_memory(ICON_PNG) else { return Vec::new() };
        let img = img.to_rgba8();
        let (w, h) = img.dimensions();
        let data = img.pixels().flat_map(|p| [p[3], p[0], p[1], p[2]]).collect();
        vec![ksni::Icon { width: w as i32, height: h as i32, data }]
    }

    pub struct Tray {
        _handle: Handle<Sni>,
    }

    impl Tray {
        pub fn start(on_act: impl Fn(TrayAct) + Send + Sync + 'static) -> Option<Tray> {
            let sni = Sni { on_act: Arc::new(on_act), icon: icon() };
            match sni.spawn() {
                Ok(h) => Some(Tray { _handle: h }),
                Err(e) => {
                    eprintln!("tepsi başlatılamadı: {e}");
                    None
                }
            }
        }
    }
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
#[allow(dead_code)]
mod imp {
    use super::TrayAct;

    pub struct Tray;

    impl Tray {
        pub fn start(_on_act: impl Fn(TrayAct) + Send + Sync + 'static) -> Option<Tray> {
            None
        }
    }
}

#[allow(unused_imports)]
pub use imp::Tray;
