use anyhow::Result;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendKind {
    Clap,
    Lv2,
}

pub type RtMidiEvent = MidiEvent;

#[derive(Clone, Debug)]
pub struct HostConfig {
    pub sample_rate: f64,
    pub max_block: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ParamKey {
    Clap(u32),   // CLAP parameter ID
    Lv2(String), // LV2 control port symbol
}

#[derive(Clone, Debug)]
pub struct UnifiedParamInfo {
    pub key: ParamKey,
    pub name: String,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub stepped: bool,
    pub enum_labels: Option<Vec<String>>,
}

#[derive(Clone, Debug)]
pub struct UnifiedPluginInfo {
    pub backend: BackendKind,
    pub uri: String, // CLAP: file:///path/lib.so#plugin_id; LV2: URI
    pub name: String,
    pub is_instrument: bool,
    pub audio_inputs: usize,
    pub audio_outputs: usize,
    pub has_midi: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct ProcessCtx {
    pub frames: usize,
    pub bpm: f32,
    pub time_samples: f64,
    pub loop_active: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct MidiEvent {
    pub status: u8,
    pub data1: u8,
    pub data2: u8,
    pub time_frames: i64,
}

pub trait PluginInstance {
    fn process(
        &mut self,
        ctx: &ProcessCtx,
        audio_in: &[&[f32]],
        audio_out: &mut [&mut [f32]],
        events: &[MidiEvent],
    ) -> anyhow::Result<()>;

    fn set_param(&mut self, key: &ParamKey, value: f32);
    fn get_param(&self, key: &ParamKey) -> Option<f32>;
    fn params(&self) -> &[UnifiedParamInfo];

    fn save_state(&mut self) -> Option<Vec<u8>> {
        None
    }
    fn load_state(&mut self, _data: &[u8]) -> bool {
        false
    }
}

pub trait PluginBackend: Send + Sync {
    fn kind(&self) -> BackendKind;
    fn init(&self, cfg: &HostConfig) -> Result<()>;
    fn scan(&self) -> Result<Vec<UnifiedPluginInfo>>;
    fn instantiate(&self, uri: &str) -> Result<Box<dyn PluginInstance>>;
}
