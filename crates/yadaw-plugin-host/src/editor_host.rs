use anyhow::{Result, anyhow};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

#[cfg(unix)]
pub(crate) mod x11 {
    use anyhow::{Result, anyhow};
    use std::ptr;
    use x11_dl::xlib;

    pub struct X11State {
        pub xlib: xlib::Xlib,
        pub display: *mut xlib::Display,
        pub parent_win: xlib::Window,
        pub child_win: xlib::Window,
        pub wm_delete_window: u64,
        pub wm_protocols: u64,
        pub size: (u32, u32),
        pub expose_tick: u32,
    }

    unsafe impl Send for X11State {}

    pub fn open_display() -> Result<(xlib::Xlib, *mut xlib::Display)> {
        let xlib = xlib::Xlib::open().map_err(|_| anyhow!("Cannot open X11 shared library"))?;
        let display = unsafe { (xlib.XOpenDisplay)(ptr::null()) };
        if display.is_null() {
            return Err(anyhow!("Cannot open X11 display"));
        }
        Ok((xlib, display))
    }

    pub fn create_parent_window(
        xlib: &xlib::Xlib,
        display: *mut xlib::Display,
        width: u32,
        height: u32,
    ) -> Result<xlib::Window> {
        unsafe {
            let screen = (xlib.XDefaultScreen)(display);
            let root = (xlib.XRootWindow)(display, screen);
            let white = (xlib.XWhitePixel)(display, screen);

            let parent_win =
                (xlib.XCreateSimpleWindow)(display, root, 100, 100, width, height, 0, 0, white);

            (xlib.XSelectInput)(
                display,
                parent_win,
                (xlib::StructureNotifyMask
                    | xlib::ExposureMask
                    | xlib::SubstructureNotifyMask
                    | xlib::PropertyChangeMask) as i64,
            );

            let wm_delete_window = (xlib.XInternAtom)(display, c"WM_DELETE_WINDOW".as_ptr(), 0);
            (xlib.XSetWMProtocols)(
                display,
                parent_win,
                &wm_delete_window as *const _ as *mut _,
                1,
            );

            (xlib.XMapWindow)(display, parent_win);
            (xlib.XFlush)(display);

            Ok(parent_win)
        }
    }

    pub fn find_child_window(
        xlib: &xlib::Xlib,
        display: *mut xlib::Display,
        parent_win: xlib::Window,
    ) -> xlib::Window {
        unsafe {
            let mut root_ret: xlib::Window = 0;
            let mut parent_ret: xlib::Window = 0;
            let mut children: *mut xlib::Window = ptr::null_mut();
            let mut nchildren: std::os::raw::c_uint = 0;
            let ok = (xlib.XQueryTree)(
                display,
                parent_win,
                &mut root_ret,
                &mut parent_ret,
                &mut children,
                &mut nchildren,
            );
            let found = if ok != 0 && !children.is_null() && nchildren > 0 {
                *children.offset(0)
            } else {
                0
            };
            if !children.is_null() {
                (xlib.XFree)(children as *mut _);
            }
            found
        }
    }

    pub fn send_expose_event(
        xlib: &xlib::Xlib,
        display: *mut xlib::Display,
        win: xlib::Window,
        w: i32,
        h: i32,
    ) {
        unsafe {
            let mut ev: xlib::XEvent = std::mem::zeroed();
            ev.type_ = xlib::Expose as i32;
            let expose = &mut ev.expose;
            expose.type_ = xlib::Expose as i32;
            expose.window = win;
            expose.x = 0;
            expose.y = 0;
            expose.width = w;
            expose.height = h;
            expose.count = 0;
            (xlib.XSendEvent)(display, win, 0, 0, &mut ev);
            (xlib.XFlush)(display);
        }
    }

    pub fn cleanup_x11_window(xlib: &xlib::Xlib, display: *mut xlib::Display, win: xlib::Window) {
        unsafe {
            (xlib.XDestroyWindow)(display, win);
            (xlib.XCloseDisplay)(display);
        }
    }

    /// Returns `true` if the user closed the window (WM_DELETE_WINDOW).
    pub fn pump_events(state: &mut X11State) -> bool {
        let xlib = &state.xlib;
        let display = state.display;
        let mut closed = false;

        while unsafe { (xlib.XPending)(display) } > 0 {
            let mut event: xlib::XEvent = unsafe { std::mem::zeroed() };
            unsafe { (xlib.XNextEvent)(display, &mut event) };

            if unsafe { event.type_ } == xlib::ClientMessage as i32 {
                let msg = unsafe { event.client_message };
                if msg.message_type == state.wm_protocols
                    && msg.data.as_longs()[0] as u64 == state.wm_delete_window
                {
                    closed = true;
                }
            }
        }

        if state.child_win != 0 {
            state.expose_tick = state.expose_tick.wrapping_add(1);
            if state.expose_tick % 8 == 0 {
                let (w, h) = state.size;
                send_expose_event(xlib, display, state.child_win, w as i32, h as i32);
            }
        }

        closed
    }
}

/// Interface each plugin backend implements so the shared [`EditorHost`] can
/// manage the editor lifecycle uniformly.
pub trait EditorBackend: Send + 'static {
    /// Whether the plugin exposes an editor.
    fn has_editor(&self) -> bool;

    /// Try to open a floating (standalone) editor window.
    ///
    /// Return `Ok(true)` if the editor was opened – the host will not attempt
    /// embedded mode.  Return `Ok(false)` to fall through to embedded.
    fn try_open_floating(&mut self) -> Result<bool>;

    /// Open the editor embedded into an X11 parent window.
    fn open_embedded(&mut self, parent_window: u32) -> Result<()>;

    /// Tear down the editor (destroy views, release handles).
    fn close(&mut self) -> Result<()>;

    /// Preferred editor size in pixels.
    fn preferred_size(&self) -> Option<(u32, u32)>;

    /// Called each host-loop iteration for backend-specific idle work
    /// (e.g. CLAP callback pump, timer firing).
    fn on_idle(&mut self) {}

    /// Called when the editor host detected the user closed the window.
    fn on_gui_closed(&mut self) {}
}

enum EditorCommand {
    OpenEditor { result_tx: mpsc::Sender<Result<()>> },
    CloseEditor,
    Shutdown { result_tx: mpsc::Sender<()> },
}

pub struct EditorHost {
    cmd_tx: mpsc::Sender<EditorCommand>,
    join_handle: Option<thread::JoinHandle<()>>,
}

impl EditorHost {
    /// Spawn the background thread and start the loop immediately.
    /// Used by CLAP which needs the thread for on_idle even without an editor.
    pub fn spawn(backend: Box<dyn EditorBackend>) -> Result<Self> {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let join_handle = thread::Builder::new()
            .name("plugin-editor".into())
            .spawn(move || Self::thread_main(backend, cmd_rx))
            .map_err(|e| anyhow!("Failed to spawn editor thread: {e}"))?;
        Ok(Self {
            cmd_tx,
            join_handle: Some(join_handle),
        })
    }

    /// Open the editor (creates X11 parent window, calls backend).
    pub fn open_editor(&self) -> Result<()> {
        let (result_tx, result_rx) = mpsc::channel();
        self.cmd_tx
            .send(EditorCommand::OpenEditor { result_tx })
            .map_err(|_| anyhow!("Editor thread disconnected"))?;
        result_rx
            .recv_timeout(Duration::from_secs(10))
            .map_err(|_| anyhow!("Editor thread did not respond"))?
    }

    /// Close the editor.
    pub fn close_editor(&self) {
        let _ = self.cmd_tx.send(EditorCommand::CloseEditor);
    }

    /// Shut down the thread permanently.
    pub fn shutdown(&mut self) {
        if let Some(handle) = self.join_handle.take() {
            let (result_tx, result_rx) = mpsc::channel();
            let _ = self.cmd_tx.send(EditorCommand::Shutdown { result_tx });
            let _ = result_rx.recv_timeout(Duration::from_secs(2));
            let _ = handle.join();
        }
    }

    fn thread_main(mut backend: Box<dyn EditorBackend>, cmd_rx: mpsc::Receiver<EditorCommand>) {
        let mut x11_state: Option<x11::X11State> = None;

        loop {
            // Idle callback – always called (CLAP needs it for callback pump)
            backend.on_idle();

            match cmd_rx.recv_timeout(Duration::from_millis(16)) {
                Ok(EditorCommand::OpenEditor { result_tx }) => {
                    let result = Self::open_impl(&mut *backend, &mut x11_state);
                    let _ = result_tx.send(result);
                }
                Ok(EditorCommand::CloseEditor) => {
                    Self::close_impl(&mut *backend, &mut x11_state);
                }
                Ok(EditorCommand::Shutdown { result_tx }) => {
                    Self::close_impl(&mut *backend, &mut x11_state);
                    let _ = result_tx.send(());
                    break;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }

            if let Some(ref mut state) = x11_state {
                if x11::pump_events(state) {
                    backend.on_gui_closed();
                    Self::close_impl(&mut *backend, &mut x11_state);
                }
            }
        }

        Self::close_impl(&mut *backend, &mut x11_state);
    }

    fn open_impl(
        backend: &mut dyn EditorBackend,
        x11_state: &mut Option<x11::X11State>,
    ) -> Result<()> {
        if x11_state.is_some() {
            return Ok(());
        }

        // Try floating first (CLAP supports X11/Wayland floating, VST3 skips).
        if backend.try_open_floating()? {
            return Ok(());
        }

        // Fall back to X11 embedded.
        let (xlib, display) = x11::open_display()?;
        let size = backend.preferred_size().unwrap_or((800, 600));
        let parent_win = x11::create_parent_window(&xlib, display, size.0, size.1)?;

        backend.open_embedded(parent_win as u32)?;

        let child_win = x11::find_child_window(&xlib, display, parent_win);

        if let Some(ps) = backend.preferred_size() {
            unsafe {
                (xlib.XResizeWindow)(display, parent_win, ps.0, ps.1);
                (xlib.XSync)(display, 0);
            }
        }

        unsafe {
            (xlib.XMapSubwindows)(display, parent_win);
            (xlib.XSync)(display, 0);
        }

        // Extract wm atoms after creating the window
        let wm_delete_window =
            unsafe { (xlib.XInternAtom)(display, c"WM_DELETE_WINDOW".as_ptr(), 0) };
        let wm_protocols = unsafe { (xlib.XInternAtom)(display, c"WM_PROTOCOLS".as_ptr(), 0) };

        if child_win != 0 {
            let (w, h) = size;
            x11::send_expose_event(&xlib, display, child_win, w as i32, h as i32);
        }

        *x11_state = Some(x11::X11State {
            xlib,
            display,
            parent_win,
            child_win,
            wm_delete_window,
            wm_protocols,
            size,
            expose_tick: 0,
        });

        Ok(())
    }

    fn close_impl(backend: &mut dyn EditorBackend, x11_state: &mut Option<x11::X11State>) {
        let _ = backend.close();
        if let Some(state) = x11_state.take() {
            x11::cleanup_x11_window(&state.xlib, state.display, state.parent_win);
        }
    }
}

impl Drop for EditorHost {
    fn drop(&mut self) {
        self.shutdown();
    }
}
