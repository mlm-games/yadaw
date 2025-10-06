use parking_lot::RwLock;
use rayon::iter::{IntoParallelRefMutIterator, ParallelIterator};
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct PerformanceMetrics {
    pub cpu_usage: f32,
    pub memory_usage: usize,
    pub disk_streaming_rate: f32,
    pub audio_buffer_health: f32,
    pub plugin_processing_time: Duration,
    pub xruns: usize,
    pub latency_ms: f32,
}

pub struct PerformanceMonitor {
    metrics_history: Arc<RwLock<VecDeque<PerformanceMetrics>>>,
    max_history_size: usize,
    last_update: Instant,
    xrun_count: usize,
    audio_callback_times: VecDeque<Duration>,
    optimization_hints: Vec<OptimizationHint>,
}

#[derive(Debug, Clone)]
pub struct OptimizationHint {
    pub category: OptimizationCategory,
    pub severity: Severity,
    pub message: String,
    pub suggestion: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum OptimizationCategory {
    CPU,
    Memory,
    Disk,
    Latency,
    Plugins,
    BufferSize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Severity {
    Info,
    Warning,
    Critical,
}

impl Default for PerformanceMonitor {
    fn default() -> Self {
        Self::new()
    }
}

impl PerformanceMonitor {
    pub fn new() -> Self {
        Self {
            metrics_history: Arc::new(RwLock::new(VecDeque::with_capacity(1000))),
            max_history_size: 1000,
            last_update: Instant::now(),
            xrun_count: 0,
            audio_callback_times: VecDeque::with_capacity(100),
            optimization_hints: Vec::new(),
        }
    }

    pub fn update_metrics(&mut self, metrics: PerformanceMetrics) {
        {
            let mut history = self.metrics_history.write();
            if history.len() >= self.max_history_size {
                history.pop_front();
            }
            history.push_back(metrics.clone());
        }

        // Check for performance issues and generate hints
        self.analyze_performance(&metrics);
        self.last_update = Instant::now();
    }

    fn analyze_performance(&mut self, metrics: &PerformanceMetrics) {
        self.optimization_hints.clear();

        // CPU usage analysis
        if metrics.cpu_usage > 0.9 {
            self.optimization_hints.push(OptimizationHint {
                category: OptimizationCategory::CPU,
                severity: Severity::Critical,
                message: "CPU usage is critically high".to_string(),
                suggestion: "Consider freezing tracks or increasing buffer size".to_string(),
            });
        } else if metrics.cpu_usage > 0.7 {
            self.optimization_hints.push(OptimizationHint {
                category: OptimizationCategory::CPU,
                severity: Severity::Warning,
                message: "CPU usage is high".to_string(),
                suggestion: "Consider disabling some plugins or using sends for effects"
                    .to_string(),
            });
        }

        // Memory usage analysis
        let memory_mb = metrics.memory_usage / (1024 * 1024);
        if memory_mb > 4096 {
            self.optimization_hints.push(OptimizationHint {
                category: OptimizationCategory::Memory,
                severity: Severity::Warning,
                message: format!("High memory usage: {} MB", memory_mb),
                suggestion: "Consider purging unused samples or closing other applications"
                    .to_string(),
            });
        }

        // Buffer health analysis
        if metrics.audio_buffer_health < 0.3 {
            self.optimization_hints.push(OptimizationHint {
                category: OptimizationCategory::BufferSize,
                severity: Severity::Critical,
                message: "Audio buffer underruns detected".to_string(),
                suggestion: "Increase buffer size or reduce processing load".to_string(),
            });
        }

        // Latency analysis
        if metrics.latency_ms > 20.0 {
            self.optimization_hints.push(OptimizationHint {
                category: OptimizationCategory::Latency,
                severity: Severity::Info,
                message: format!("Latency is {} ms", metrics.latency_ms),
                suggestion: "For recording, consider using direct monitoring".to_string(),
            });
        }

        // Plugin processing time
        if metrics.plugin_processing_time > Duration::from_millis(10) {
            self.optimization_hints.push(OptimizationHint {
                category: OptimizationCategory::Plugins,
                severity: Severity::Warning,
                message: "Plugin processing is taking too long".to_string(),
                suggestion: "Consider using lighter plugins or freezing plugin-heavy tracks"
                    .to_string(),
            });
        }
    }

    pub fn record_xrun(&mut self) {
        self.xrun_count += 1;
    }

    pub fn record_audio_callback_time(&mut self, duration: Duration) {
        if self.audio_callback_times.len() >= 100 {
            self.audio_callback_times.pop_front();
        }
        self.audio_callback_times.push_back(duration);
    }

    pub fn get_average_callback_time(&self) -> Duration {
        if self.audio_callback_times.is_empty() {
            return Duration::ZERO;
        }

        let total: Duration = self.audio_callback_times.iter().sum();
        total / self.audio_callback_times.len() as u32
    }

    pub fn get_current_metrics(&self) -> Option<PerformanceMetrics> {
        self.metrics_history.read().back().cloned()
    }

    pub fn get_optimization_hints(&self) -> &[OptimizationHint] {
        &self.optimization_hints
    }

    pub fn get_metrics_history(&self, last_n: usize) -> Vec<PerformanceMetrics> {
        let history = self.metrics_history.read();
        history.iter().rev().take(last_n).rev().cloned().collect()
    }

    pub fn reset_xrun_count(&mut self) {
        self.xrun_count = 0;
    }

    pub fn get_xrun_count(&self) -> usize {
        self.xrun_count
    }
}

// Resource pool for efficient memory management
pub struct ResourcePool<T> {
    pool: Arc<RwLock<Vec<T>>>,
    factory: Box<dyn Fn() -> T + Send + Sync>,
    max_size: usize,
}

impl<T: Clone + Send + Sync + 'static> ResourcePool<T> {
    pub fn new(factory: impl Fn() -> T + Send + Sync + 'static, max_size: usize) -> Self {
        Self {
            pool: Arc::new(RwLock::new(Vec::with_capacity(max_size))),
            factory: Box::new(factory),
            max_size,
        }
    }

    pub fn acquire(&self) -> PooledResource<T> {
        let mut pool = self.pool.write();
        let resource = pool.pop().unwrap_or_else(|| (self.factory)());
        PooledResource {
            resource: Some(resource),
            pool: Arc::clone(&self.pool),
        }
    }

    pub fn clear(&self) {
        self.pool.write().clear();
    }

    pub fn size(&self) -> usize {
        self.pool.read().len()
    }
}

pub struct PooledResource<T> {
    resource: Option<T>,
    pool: Arc<RwLock<Vec<T>>>,
}

impl<T> PooledResource<T> {
    pub fn get(&self) -> &T {
        self.resource.as_ref().unwrap()
    }

    pub fn get_mut(&mut self) -> &mut T {
        self.resource.as_mut().unwrap()
    }
}

impl<T> Drop for PooledResource<T> {
    fn drop(&mut self) {
        if let Some(resource) = self.resource.take() {
            let mut pool = self.pool.write();
            if pool.len() < pool.capacity() {
                pool.push(resource);
            }
        }
    }
}

// Buffer cache for efficient audio streaming
pub struct BufferCache {
    cache: Arc<RwLock<lru::LruCache<u64, Vec<f32>>>>,
    max_size: usize,
    hits: Arc<RwLock<usize>>,
    misses: Arc<RwLock<usize>>,
}

impl BufferCache {
    pub fn new(max_size: usize) -> Self {
        Self {
            cache: Arc::new(RwLock::new(lru::LruCache::new(
                std::num::NonZeroUsize::new(max_size).unwrap(),
            ))),
            max_size,
            hits: Arc::new(RwLock::new(0)),
            misses: Arc::new(RwLock::new(0)),
        }
    }

    pub fn get(&self, key: u64) -> Option<Vec<f32>> {
        let mut cache = self.cache.write();
        if let Some(buffer) = cache.get(&key) {
            *self.hits.write() += 1;
            Some(buffer.clone())
        } else {
            *self.misses.write() += 1;
            None
        }
    }

    pub fn insert(&self, key: u64, buffer: Vec<f32>) {
        self.cache.write().put(key, buffer);
    }

    pub fn hit_rate(&self) -> f32 {
        let hits = *self.hits.read() as f32;
        let misses = *self.misses.read() as f32;
        let total = hits + misses;
        if total > 0.0 { hits / total } else { 0.0 }
    }

    pub fn clear(&self) {
        self.cache.write().clear();
        *self.hits.write() = 0;
        *self.misses.write() = 0;
    }
}

// Thread pool for parallel processing
pub struct ProcessingThreadPool {
    pool: rayon::ThreadPool,
    num_threads: usize,
}

impl ProcessingThreadPool {
    pub fn new(num_threads: Option<usize>) -> Self {
        let num_threads = num_threads.unwrap_or_else(|| num_cpus::get().saturating_sub(1).max(1));

        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(num_threads)
            .thread_name(|i| format!("yadaw-worker-{}", i))
            .build()
            .unwrap();

        Self { pool, num_threads }
    }

    pub fn process_parallel<T, F>(&self, items: &mut [T], processor: F)
    where
        T: Send,
        F: Fn(&mut T) + Send + Sync,
    {
        self.pool.install(|| {
            items.par_iter_mut().for_each(processor);
        });
    }

    pub fn num_threads(&self) -> usize {
        self.num_threads
    }
}

// Optimization settings
#[derive(Debug, Clone)]
pub struct OptimizationSettings {
    pub enable_multicore: bool,
    pub cache_size_mb: usize,
    pub prefetch_ahead_seconds: f32,
    pub auto_freeze_threshold: f32,
    pub auto_purge_unused: bool,
    pub process_in_background: bool,
}

impl Default for OptimizationSettings {
    fn default() -> Self {
        Self {
            enable_multicore: true,
            cache_size_mb: 512,
            prefetch_ahead_seconds: 2.0,
            auto_freeze_threshold: 0.8,
            auto_purge_unused: true,
            process_in_background: true,
        }
    }
}
