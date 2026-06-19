#[cfg(not(target_arch = "wasm32"))]
use once_cell::sync::Lazy;
#[cfg(not(target_arch = "wasm32"))]
use tokio::runtime::Runtime;

#[cfg(not(target_arch = "wasm32"))]
pub(crate) static RT: Lazy<Runtime> = Lazy::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
});

#[cfg(not(target_arch = "wasm32"))]
pub fn block_on<F: std::future::Future<Output = T>, T>(f: F) -> T {
    RT.block_on(f)
}

/// Spawns a future and returns a [`tokio::task::JoinHandle`].
/// Not available on wasm — use [`spawn_detached!`] instead.
#[cfg(not(target_arch = "wasm32"))]
#[macro_export]
macro_rules! spawn_task {
    ($fut:expr) => {
        $crate::runtime::RT.spawn($fut)
    };
}

/// Spawns a fire-and-forget future.
#[macro_export]
macro_rules! spawn_detached {
    ($fut:expr) => {{
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = $crate::runtime::RT.spawn($fut);
        }
        #[cfg(target_arch = "wasm32")]
        {
            wasm_bindgen_futures::spawn_local($fut);
        }
    }};
}
