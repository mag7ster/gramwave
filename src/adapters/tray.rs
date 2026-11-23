use fastframe_shell::Waker;
use ksni::blocking::TrayMethods;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex, Weak,
    },
    time::{Duration, Instant},
};
use winit::window::Window;

use super::window_activation::WindowActivation;
use crate::{application::controller::tray_icon, tr};

pub(crate) enum Event {
    Toggle(Option<String>),
    Show(Option<String>),
    FocusRequested {
        window: Weak<Window>,
        sent_at: Instant,
        hide_if_denied: bool,
    },
    PlayPause,
    Next,
    Quit,
}

pub(crate) struct Tray {
    _handle: ksni::blocking::Handle<Item>,
    shown: Arc<AtomicBool>,
    events: mpsc::Receiver<Event>,
    window: Arc<Mutex<Option<WindowActivation>>>,
}

impl Tray {
    pub(crate) fn spawn(wake: Waker) -> Option<Self> {
        let (sender, events) = mpsc::channel();
        let shown = Arc::new(AtomicBool::new(true));
        let window = Arc::new(Mutex::new(None));
        let sandboxed = std::path::Path::new("/.flatpak-info").exists()
            || std::env::var_os("FLATPAK_ID").is_some();
        let handle = Item {
            sender,
            wake,
            shown: shown.clone(),
            window: window.clone(),
        }
        .disable_dbus_name(sandboxed)
        .assume_sni_available(true)
        .spawn()
        .ok()?;
        Some(Self {
            _handle: handle,
            shown,
            events,
            window,
        })
    }

    pub(crate) fn is_shown(&self) -> bool {
        self.shown.load(Ordering::Acquire)
    }

    pub(crate) fn events(&self) -> Vec<Event> {
        self.events.try_iter().collect()
    }

    pub(crate) fn set_window(&self, window: Option<WindowActivation>) {
        *self.window.lock().unwrap() = window;
    }

    pub(crate) fn dispatch_window_events(&self) {
        let mut window = self.window.lock().unwrap();
        if window
            .as_mut()
            .is_some_and(|window| !window.dispatch_pending())
        {
            *window = None;
        }
    }

    pub(crate) fn activate_window(&self, token: Option<&str>) -> bool {
        self.window
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|window| window.activate(token))
    }

    pub(crate) fn targets_window(&self, window: &Weak<Window>) -> bool {
        self.window
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|active| std::ptr::eq(Arc::as_ptr(&active.window), window.as_ptr()))
    }
}

struct Item {
    sender: mpsc::Sender<Event>,
    wake: Waker,
    shown: Arc<AtomicBool>,
    window: Arc<Mutex<Option<WindowActivation>>>,
}

impl Item {
    fn send(&self, event: Event) {
        if self.sender.send(event).is_ok() {
            self.wake.wake();
        }
    }

    fn request_window(&self, hide_if_denied: bool) {
        let token = activation_token();
        let sent_at = Instant::now();
        let event = self.window.lock().unwrap().as_mut().and_then(|window| {
            // EGL may be waiting for a frame while the display has already queued focus events.
            (window.dispatch_pending() && !window.focused() && window.activate(token.as_deref()))
                .then(|| Event::FocusRequested {
                    window: Arc::downgrade(&window.window),
                    sent_at,
                    hide_if_denied,
                })
        });
        self.send(event.unwrap_or_else(|| {
            if hide_if_denied {
                Event::Toggle(token)
            } else {
                Event::Show(token)
            }
        }));
    }
}

fn activation_token() -> Option<String> {
    let current = || ksni::current_xdg_activation_token().filter(|token| !token.is_empty());
    current().or_else(|| {
        // Some panels deliver the token just after the event.
        std::thread::sleep(Duration::from_millis(1));
        current()
    })
}

impl ksni::Tray for Item {
    fn id(&self) -> String {
        "tgplayer".into()
    }

    fn title(&self) -> String {
        "Gramwave".into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        const SIZE: usize = 64;
        let mut data = tray_icon(SIZE);
        for pixel in data.as_chunks_mut::<4>().0 {
            pixel.rotate_right(1);
        }
        vec![ksni::Icon {
            width: SIZE as i32,
            height: SIZE as i32,
            data,
        }]
    }

    fn watcher_online(&self) {
        self.shown.store(true, Ordering::Release);
    }

    fn watcher_offline(&self, _: ksni::OfflineReason) -> bool {
        self.shown.store(false, Ordering::Release);
        true
    }

    fn activate(&mut self, _: i32, _: i32) {
        self.request_window(true);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        fn action(label: String, activate: fn(&mut Item)) -> ksni::MenuItem<Item> {
            ksni::menu::StandardItem {
                label: label.replace('_', "__"),
                activate: Box::new(activate),
                ..Default::default()
            }
            .into()
        }

        vec![
            action(tr("Show player"), |item| item.request_window(false)),
            action(tr("Play / pause"), |item| item.send(Event::PlayPause)),
            action(tr("Next"), |item| item.send(Event::Next)),
            ksni::MenuItem::Separator,
            action(tr("Quit"), |item| item.send(Event::Quit)),
        ]
    }
}
