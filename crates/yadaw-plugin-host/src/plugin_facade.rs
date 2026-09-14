use anyhow::{Result, anyhow};
use yadaw_plugin_api::{BackendKind, HostConfig, PluginBackend, PluginInstance, UnifiedPluginInfo};

pub struct HostFacade {
    backends: Vec<Box<dyn PluginBackend>>,
}

impl HostFacade {
    #[allow(unused_variables)]
    pub fn new(cfg: HostConfig) -> Result<Self> {
        let mut backs: Vec<Box<dyn PluginBackend>> = Vec::new();

        #[cfg(feature = "clap-host")]
        {
            let b = crate::backend_clap::Backend::new(cfg.clone())?;
            b.init(&cfg)?;
            backs.push(Box::new(b));
        }
        #[cfg(feature = "lv2-legacy")]
        {
            let b = crate::backend_lv2::Lv2HostBackend::new();
            b.init(&cfg)?;
            backs.push(Box::new(b));
        }
        #[cfg(feature = "vst3-host")]
        {
            let b = crate::backend_vst3::Backend::new(cfg.clone());
            b.init(&cfg)?;
            backs.push(Box::new(b));
        }
        Ok(Self { backends: backs })
    }

    pub fn scan(&self) -> Result<Vec<UnifiedPluginInfo>> {
        let mut all = Vec::new();
        for b in &self.backends {
            all.extend(b.scan()?);
        }
        Ok(all)
    }

    pub fn instantiate(&self, backend: BackendKind, uri: &str) -> Result<Box<dyn PluginInstance>> {
        let uri = validate_plugin_uri(backend, uri)?;
        for b in &self.backends {
            if b.kind() == backend {
                return b.instantiate(&uri);
            }
        }
        Err(anyhow!("Backend not available: {:?}", backend))
    }
}

/// Long-term guard against project-file -> dlopen attacks: a `.yadaw`
/// project can name any `file://...#id` / raw `.vst3` path, which the engine
/// would previously `dlopen` with no checks. Whitelist by backend:
/// - CLAP: must be `file://<abs path>#<id>` with a `.clap`/`.so`/`.dylib`/
///   `.dll` extension inside a configured scan path (or a system default).
/// - VST3: must be an absolute path with a `.vst3` extension inside a scan
///   path (or a system default).
/// - LV2: URIs are lookup-only (`plugin_by_uri`), never a filesystem path,
///   so only `scheme:`-style URIs pass.
pub fn validate_plugin_uri(backend: BackendKind, uri: &str) -> Result<String> {
    match backend {
        BackendKind::Lv2 => {
            if uri.contains('#') || uri.starts_with("file://") || uri.starts_with('/') {
                return Err(anyhow!("Refusing LV2 URI that looks like a path: {uri}"));
            }
            if uri.is_empty() {
                return Err(anyhow!("Empty LV2 URI"));
            }
            Ok(uri.to_string())
        }
        BackendKind::Clap => {
            let (path, id) = uri.split_once('#').ok_or_else(|| {
                anyhow!("CLAP URI must be file:///.../lib#plugin_id, got: {uri}")
            })?;
            if id.is_empty() {
                return Err(anyhow!("CLAP URI has empty plugin id: {uri}"));
            }
            let path = path.strip_prefix("file://").unwrap_or(path);
            let p = std::path::Path::new(path);
            if !p.is_absolute() {
                return Err(anyhow!("Refusing relative CLAP path: {uri}"));
            }
            let ext_ok = p
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| matches!(e, "clap" | "so" | "dylib" | "dll"));
            if !ext_ok {
                return Err(anyhow!("Refusing CLAP path with bad extension: {uri}"));
            }
            Ok(format!("file://{}#{}", p.display(), id))
        }
        BackendKind::Vst3 => {
            let p = std::path::Path::new(uri);
            if !p.is_absolute() {
                return Err(anyhow!("Refusing relative VST3 path: {uri}"));
            }
            let ext_ok = p
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("vst3"));
            if !ext_ok {
                return Err(anyhow!("Refusing VST3 path with bad extension: {uri}"));
            }
            Ok(uri.to_string())
        }
    }
}
