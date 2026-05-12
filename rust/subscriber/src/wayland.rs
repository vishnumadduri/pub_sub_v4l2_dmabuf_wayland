use wayland_client::{
    delegate_noop,
    protocol::{wl_compositor, wl_registry, wl_surface},
    Connection, Dispatch, EventQueue, Proxy, QueueHandle,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};
use wayland_egl::WlEglSurface;

// ---- Application state for the Dispatch implementations -------------------

pub struct AppState {
    pub compositor:   Option<wl_compositor::WlCompositor>,
    pub wm_base:      Option<xdg_wm_base::XdgWmBase>,
    pub configured:   bool,
    pub should_close: bool,
}

impl AppState {
    fn new() -> Self {
        Self {
            compositor:   None,
            wm_base:      None,
            configured:   false,
            should_close: false,
        }
    }
}

// wl_registry: bind compositor and xdg_wm_base on global advertisement.
impl Dispatch<wl_registry::WlRegistry, ()> for AppState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global { name, interface, version } = event {
            match interface.as_str() {
                "wl_compositor" => {
                    state.compositor = Some(
                        registry.bind::<wl_compositor::WlCompositor, _, _>(
                            name, version.min(4), qh, (),
                        )
                    );
                }
                "xdg_wm_base" => {
                    state.wm_base = Some(
                        registry.bind::<xdg_wm_base::XdgWmBase, _, _>(name, 1, qh, ())
                    );
                }
                _ => {}
            }
        }
    }
}

// xdg_wm_base: respond to compositor ping.
impl Dispatch<xdg_wm_base::XdgWmBase, ()> for AppState {
    fn event(
        _: &mut Self,
        wm: &xdg_wm_base::XdgWmBase,
        event: xdg_wm_base::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm.pong(serial);
        }
    }
}

// xdg_surface: ack the configure event, then mark the window as configured.
impl Dispatch<xdg_surface::XdgSurface, ()> for AppState {
    fn event(
        state: &mut Self,
        xdg_surf: &xdg_surface::XdgSurface,
        event: xdg_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_surface::Event::Configure { serial } = event {
            xdg_surf.ack_configure(serial);
            state.configured = true;
        }
    }
}

// xdg_toplevel: detect window close.
impl Dispatch<xdg_toplevel::XdgToplevel, ()> for AppState {
    fn event(
        state: &mut Self,
        _: &xdg_toplevel::XdgToplevel,
        event: xdg_toplevel::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_toplevel::Event::Close = event {
            state.should_close = true;
        }
    }
}

delegate_noop!(AppState: ignore wl_compositor::WlCompositor);
delegate_noop!(AppState: ignore wl_surface::WlSurface);

// ---- WaylandWindow ---------------------------------------------------------

pub struct WaylandWindow {
    pub conn:         Connection,
    pub queue:        EventQueue<AppState>,
    pub state:        AppState,
    _surface:         wl_surface::WlSurface,
    _xdg_surface:     xdg_surface::XdgSurface,
    _xdg_toplevel:    xdg_toplevel::XdgToplevel,
    egl_surface:      WlEglSurface,
    pub width:        u32,
    pub height:       u32,
}

impl WaylandWindow {
    pub fn create(width: u32, height: u32, title: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let conn = Connection::connect_to_env()?;
        let mut queue = conn.new_event_queue::<AppState>();
        let qh = queue.handle();

        let _registry = conn.display().get_registry(&qh, ());
        let mut state = AppState::new();

        // First roundtrip: populate global bindings.
        queue.roundtrip(&mut state)?;

        let compositor = state.compositor.take().ok_or("wl_compositor not found")?;
        let wm_base    = state.wm_base.take().ok_or("xdg_wm_base not found")?;

        let surface   = compositor.create_surface(&qh, ());
        let xdg_surf  = wm_base.get_xdg_surface(&surface, &qh, ());
        let toplevel  = xdg_surf.get_toplevel(&qh, ());
        toplevel.set_title(title.into());
        toplevel.set_app_id("dmabuf.subscriber".into());
        surface.commit();

        // Wait for the initial xdg_surface configure handshake.
        while !state.configured {
            queue.blocking_dispatch(&mut state)?;
        }

        let egl_surface = WlEglSurface::new(surface.id(), width as i32, height as i32)
            .map_err(|_| "wl_egl_window_create failed")?;

        Ok(Self {
            conn,
            queue,
            state,
            _surface:      surface,
            _xdg_surface:  xdg_surf,
            _xdg_toplevel: toplevel,
            egl_surface,
            width,
            height,
        })
    }

    /// Raw pointer to the underlying wl_egl_window (for EGL surface creation).
    pub fn egl_window_ptr(&self) -> *mut std::ffi::c_void {
        self.egl_surface.ptr() as *mut std::ffi::c_void
    }

    /// Raw pointer to the underlying wl_display (for EGL display creation).
    pub fn display_ptr(&self) -> *mut std::ffi::c_void {
        self.conn.backend().display_ptr() as *mut std::ffi::c_void
    }

    /// Non-blocking: flush outgoing requests and dispatch any pending events.
    /// Returns false if the window was closed or a fatal error occurred.
    pub fn dispatch_pending(&mut self) -> bool {
        if self.state.should_close {
            return false;
        }
        let _ = self.queue.flush();
        match self.queue.dispatch_pending(&mut self.state) {
            Ok(_)  => !self.state.should_close,
            Err(_) => false,
        }
    }

    pub fn should_close(&self) -> bool {
        self.state.should_close
    }
}
