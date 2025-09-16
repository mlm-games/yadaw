use crate::model::plugin_api::{
    BackendKind, HostConfig, PluginBackend, PluginInstance, UnifiedPluginInfo,
};
use anyhow::{Result, anyhow};

pub struct HostFacade {
    backends: Vec<Box<dyn PluginBackend>>,
}

impl HostFacade {
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
        for b in &self.backends {
            if b.kind() == backend {
                return b.instantiate(uri);
            }
        }
        Err(anyhow!("Backend not available: {:?}", backend))
    }
}
