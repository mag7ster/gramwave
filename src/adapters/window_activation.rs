use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use std::{collections::HashMap, io::ErrorKind, sync::Arc};
use wayland_client::{
    backend::{Backend, ObjectId, WaylandError},
    delegate_noop,
    globals::{registry_queue_init, GlobalListContents},
    protocol::{wl_registry, wl_seat, wl_surface::WlSurface},
    Connection, Dispatch, EventQueue, Proxy, QueueHandle,
};
use wayland_protocols::xdg::activation::v1::client::xdg_activation_v1::{self, XdgActivationV1};
use wayland_protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1 as toplevel, zwlr_foreign_toplevel_manager_v1 as manager,
};
use winit::window::Window;

pub(crate) fn app_id(demo: bool) -> &'static str {
    if demo {
        "tgplayer-demo"
    } else {
        "tgplayer"
    }
}

pub(crate) struct WindowActivation {
    conn: Connection,
    queue: EventQueue<Windows>,
    state: Windows,
    surface: WlSurface,
    xdg: Option<XdgActivationV1>,
    manager: Option<manager::ZwlrForeignToplevelManagerV1>,
    seat: Option<wl_seat::WlSeat>,
    pub(crate) window: Arc<Window>,
}

impl WindowActivation {
    pub(crate) fn new(window: Arc<Window>, app_id: &'static str) -> Option<Self> {
        let RawWindowHandle::Wayland(surface) = window.window_handle().ok()?.as_raw() else {
            return None;
        };
        let RawDisplayHandle::Wayland(display) = window.display_handle().ok()?.as_raw() else {
            return None;
        };
        // The native window owns both handles and outlives every proxy stored here.
        let backend = unsafe { Backend::from_foreign_display(display.display.as_ptr().cast()) };
        let conn = Connection::from_backend(backend);
        let surface =
            unsafe { ObjectId::from_ptr(WlSurface::interface(), surface.surface.as_ptr().cast()) }
                .ok()?;
        let surface = WlSurface::from_id(&conn, surface).ok()?;
        let (globals, queue) = registry_queue_init(&conn).ok()?;
        let qh = queue.handle();
        let xdg = globals.bind(&qh, 1..=1, ()).ok();
        let manager = globals.bind(&qh, 1..=1, ()).ok();
        let seat = if manager.is_some() {
            globals.bind(&qh, 1..=5, ()).ok()
        } else {
            None
        };
        globals.destroy();
        let activation = Self {
            conn,
            queue,
            state: Windows {
                app_id,
                handles: HashMap::new(),
            },
            surface,
            xdg,
            manager,
            seat,
            window,
        };
        activation.flush().then_some(activation)
    }

    pub(crate) fn focused(&self) -> bool {
        self.state
            .target()
            .and_then(|(_, state)| state.focused)
            .unwrap_or_else(|| self.window.has_focus() && self.window.is_minimized() != Some(true))
    }

    pub(crate) fn activate(&self, token: Option<&str>) -> bool {
        if let (Some(token), Some(xdg)) = (token, &self.xdg) {
            return xdg
                .send_request(xdg_activation_v1::Request::Activate {
                    token: token.to_owned(),
                    surface: self.surface.clone(),
                })
                .is_ok()
                && self.flush();
        }
        let (Some(seat), Some((handle, _))) = (&self.seat, self.state.target()) else {
            return false;
        };
        handle.unset_minimized();
        handle
            .send_request(toplevel::Request::Activate { seat: seat.clone() })
            .is_ok()
            && self.flush()
    }

    pub(crate) fn dispatch_pending(&mut self) -> bool {
        self.queue.dispatch_pending(&mut self.state).is_ok()
    }

    fn flush(&self) -> bool {
        match self.conn.flush() {
            Ok(()) => true,
            // Winit will flush the same display again on its next event-loop iteration.
            Err(WaylandError::Io(error)) => error.kind() == ErrorKind::WouldBlock,
            Err(_) => false,
        }
    }
}

impl Drop for WindowActivation {
    fn drop(&mut self) {
        self.dispatch_pending();
        if let Some(manager) = &self.manager {
            manager.stop();
        }
        for (handle, _) in self.state.handles.drain() {
            handle.destroy();
        }
        if let Some(seat) = self.seat.as_ref().filter(|seat| seat.version() >= 5) {
            seat.release();
        }
        if let Some(xdg) = &self.xdg {
            xdg.destroy();
        }
        self.flush();
    }
}

struct Windows {
    app_id: &'static str,
    handles: HashMap<toplevel::ZwlrForeignToplevelHandleV1, ToplevelState>,
}

#[derive(Default)]
struct ToplevelState {
    ours: bool,
    focused: Option<bool>,
}

impl Windows {
    fn target(&self) -> Option<(&toplevel::ZwlrForeignToplevelHandleV1, &ToplevelState)> {
        let mut matches = self.handles.iter().filter(|(_, state)| state.ours);
        let handle = matches.next()?;
        matches.next().is_none().then_some(handle)
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Windows {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
delegate_noop!(Windows: ignore wl_seat::WlSeat);
delegate_noop!(Windows: ignore XdgActivationV1);

impl Dispatch<manager::ZwlrForeignToplevelManagerV1, ()> for Windows {
    fn event(
        state: &mut Self,
        _: &manager::ZwlrForeignToplevelManagerV1,
        event: manager::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let manager::Event::Toplevel { toplevel } = event {
            state.handles.insert(toplevel, ToplevelState::default());
        }
    }

    wayland_client::event_created_child!(Windows, manager::ZwlrForeignToplevelManagerV1, [
        manager::EVT_TOPLEVEL_OPCODE => (toplevel::ZwlrForeignToplevelHandleV1, ())
    ]);
}

impl Dispatch<toplevel::ZwlrForeignToplevelHandleV1, ()> for Windows {
    fn event(
        state: &mut Self,
        handle: &toplevel::ZwlrForeignToplevelHandleV1,
        event: toplevel::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            toplevel::Event::AppId { app_id } => {
                state.handles.entry(handle.clone()).or_default().ours = app_id == state.app_id;
            }
            toplevel::Event::State { state: flags } => {
                let (flags, _) = flags.as_chunks::<4>();
                let has = |flag: toplevel::State| {
                    flags
                        .iter()
                        .any(|bytes| u32::from_ne_bytes(*bytes) == flag as u32)
                };
                state.handles.entry(handle.clone()).or_default().focused =
                    Some(has(toplevel::State::Activated) && !has(toplevel::State::Minimized));
            }
            toplevel::Event::Closed => {
                state.handles.remove(handle);
                handle.destroy();
            }
            _ => {}
        }
    }
}
