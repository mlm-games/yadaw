pub mod plugin_facade;

#[cfg(feature = "clap-host")]
pub mod backend_clap;

#[cfg(feature = "lv2-legacy")]
pub mod backend_lv2;

#[cfg(feature = "lv2-legacy")]
pub mod lv2_plugin_host;

#[cfg(feature = "lv2-legacy")]
pub mod plugin_host;

/// Must be called early at process startup, before any other X11 calls
/// (especially before winit/eframe initializes X11).
#[cfg(all(unix, not(target_os = "android")))]
pub fn init_xlib_threads_early() {
    if let Ok(xlib) = x11_dl::xlib::Xlib::open() {
        unsafe {
            let ok = (xlib.XInitThreads)();
            if ok == 0 {
                log::warn!("XInitThreads returned 0");
            }
        }
    }
}

pub use plugin_facade::HostFacade;

#[cfg(feature = "lv2-legacy")]
pub mod legacy {
    pub use crate::lv2_plugin_host::{
        ControlPortInfo, LV2PluginHost, LV2PluginInstance, PluginInfo,
    };
    pub use crate::plugin_host::{
        ensure, get_available_plugins, init, instantiate, is_initialized, shutdown, with_host,
    };
}
