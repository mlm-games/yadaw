#[cfg(all(not(target_os = "android"), not(target_arch = "wasm32")))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    yadaw::entry::run_app()
}

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(target_os = "android")]
fn main() {}
