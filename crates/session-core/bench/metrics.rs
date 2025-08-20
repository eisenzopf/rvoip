/// System metrics collection for benchmarking
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use sysinfo::{System, Pid};

#[derive(Debug, Clone)]
pub struct MetricSnapshot {
    pub timestamp: Duration,  // Time since start
    pub cpu_percent: f32,
    pub memory_mb: f64,
    pub thread_count: usize,
    pub active_calls: usize,
}

pub struct MetricsCollector {
    snapshots: Arc<Mutex<Vec<MetricSnapshot>>>,
    start_time: Instant,
    system: Arc<Mutex<System>>,
    process_pid: Pid,
}

impl MetricsCollector {
    pub fn new() -> Self {
        let mut system = System::new_all();
        system.refresh_all();
        
        let process_pid = Pid::from_u32(std::process::id());
        
        Self {
            snapshots: Arc::new(Mutex::new(Vec::new())),
            start_time: Instant::now(),
            system: Arc::new(Mutex::new(system)),
            process_pid,
        }
    }
    
    /// Start collecting metrics at regular intervals
    pub async fn start_collection(
        &self,
        interval: Duration,
        active_calls_counter: Arc<Mutex<usize>>,
    ) {
        let snapshots = self.snapshots.clone();
        let system = self.system.clone();
        let start_time = self.start_time;
        let pid = self.process_pid;
        
        tokio::spawn(async move {
            let mut interval_timer = tokio::time::interval(interval);
            
            loop {
                interval_timer.tick().await;
                
                let mut sys = system.lock().await;
                sys.refresh_process(pid);
                
                if let Some(process) = sys.process(pid) {
                    let active_calls = *active_calls_counter.lock().await;
                    
                    let snapshot = MetricSnapshot {
                        timestamp: start_time.elapsed(),
                        cpu_percent: process.cpu_usage(),
                        memory_mb: process.memory() as f64 / 1024.0 / 1024.0,
                        thread_count: 1, // Simplified for now
                        active_calls,
                    };
                    
                    let mut snaps = snapshots.lock().await;
                    snaps.push(snapshot);
                    
                    // Stop collecting after 15 seconds (enough for all calls to complete)
                    if start_time.elapsed() > Duration::from_secs(15) {
                        break;
                    }
                }
            }
        });
    }
    
    /// Get all collected snapshots
    pub async fn get_snapshots(&self) -> Vec<MetricSnapshot> {
        self.snapshots.lock().await.clone()
    }
    
    /// Calculate statistics from snapshots
    pub fn calculate_stats(snapshots: &[MetricSnapshot]) -> MetricsStats {
        if snapshots.is_empty() {
            return MetricsStats::default();
        }
        
        let cpu_values: Vec<f32> = snapshots.iter().map(|s| s.cpu_percent).collect();
        let memory_values: Vec<f64> = snapshots.iter().map(|s| s.memory_mb).collect();
        let thread_values: Vec<usize> = snapshots.iter().map(|s| s.thread_count).collect();
        
        MetricsStats {
            peak_cpu: cpu_values.iter().cloned().fold(f32::NEG_INFINITY, f32::max),
            avg_cpu: cpu_values.iter().sum::<f32>() / cpu_values.len() as f32,
            peak_memory: memory_values.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
            avg_memory: memory_values.iter().sum::<f64>() / memory_values.len() as f64,
            peak_threads: *thread_values.iter().max().unwrap_or(&0),
            avg_threads: thread_values.iter().sum::<usize>() / thread_values.len().max(1),
        }
    }
    
    /// Print formatted metrics table
    pub fn print_metrics_table(snapshots: &[MetricSnapshot]) {
        println!("\n╔════════════════════════════════════════════════════════════════╗");
        println!("║                    BENCHMARK METRICS                           ║");
        println!("╠════════════════════════════════════════════════════════════════╣");
        println!("║ Time(s) │ CPU(%) │ Memory(MB) │ Threads │ Active Calls       ║");
        println!("╠═════════╪════════╪════════════╪═════════╪════════════════════╣");
        
        for snapshot in snapshots {
            println!("║ {:6.1} │ {:6.1} │ {:10.1} │ {:7} │ {:18} ║",
                snapshot.timestamp.as_secs_f32(),
                snapshot.cpu_percent,
                snapshot.memory_mb,
                snapshot.thread_count,
                snapshot.active_calls,
            );
        }
        
        println!("╠════════════════════════════════════════════════════════════════╣");
        
        let stats = Self::calculate_stats(snapshots);
        println!("║ Peak CPU: {:5.1}% │ Peak Memory: {:6.0}MB │ Peak Threads: {:4} ║",
            stats.peak_cpu, stats.peak_memory, stats.peak_threads);
        println!("║ Avg CPU:  {:5.1}% │ Avg Memory:  {:6.0}MB │ Avg Threads:  {:4} ║",
            stats.avg_cpu, stats.avg_memory, stats.avg_threads);
        println!("╚════════════════════════════════════════════════════════════════╝");
    }
}

#[derive(Debug, Default)]
pub struct MetricsStats {
    pub peak_cpu: f32,
    pub avg_cpu: f32,
    pub peak_memory: f64,
    pub avg_memory: f64,
    pub peak_threads: usize,
    pub avg_threads: usize,
}