use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct MachineInfo {
    pub machine_name: String,
    pub kernel: String,
    pub arch: String,
}

#[derive(Clone, Debug)]
pub struct CpuMetrics {
    pub id: usize,
    pub usage_percent: f64,
    pub frequency_khz: Option<u64>,
    pub temperature_c: Option<f64>,
}

#[derive(Clone, Debug)]
pub struct SensorSummary {
    pub label: String,
    pub temperature_c: f64,
}

#[derive(Clone, Debug, Default)]
pub struct AcceleratorMetrics {
    pub gpu: Option<GpuMetrics>,
    pub npu: Option<NpuMetrics>,
}

#[derive(Clone, Debug)]
pub struct GpuMetrics {
    pub usage_percent: Option<f64>,
    pub frequency_hz: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct NpuMetrics {
    pub usage_percent: Option<f64>,
    pub per_core_usage_percent: Vec<f64>,
    pub frequency_hz: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SensorSource {
    ThermalZone,
    Hwmon,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum SensorKind {
    Cpu,
    Gpu,
    Npu,
    Soc,
    Storage,
    Network,
    Wireless,
    Unknown,
}

#[derive(Clone, Debug)]
pub struct SensorReading {
    #[allow(dead_code)]
    pub source: SensorSource,
    pub device_name: String,
    pub label: String,
    pub kind: SensorKind,
    pub temperature_c: f64,
}

#[derive(Clone, Debug)]
pub struct LoadAverage {
    #[allow(dead_code)]
    pub one: f64,
    #[allow(dead_code)]
    pub five: f64,
    #[allow(dead_code)]
    pub fifteen: f64,
}

#[derive(Clone, Debug)]
pub struct UsageMetric {
    pub total_bytes: u64,
    #[allow(dead_code)]
    pub used_bytes: u64,
    pub used_percent: f64,
}

impl UsageMetric {
    pub fn from_total_and_available(total_bytes: u64, available_bytes: u64) -> Self {
        let used_bytes = total_bytes.saturating_sub(available_bytes);
        Self::from_total_and_used(total_bytes, used_bytes)
    }

    pub fn from_total_and_used(total_bytes: u64, used_bytes: u64) -> Self {
        let used_percent = if total_bytes == 0 {
            0.0
        } else {
            (used_bytes as f64 / total_bytes as f64) * 100.0
        };

        Self {
            total_bytes,
            used_bytes,
            used_percent,
        }
    }
}

#[derive(Clone, Debug)]
pub struct MemoryMetrics {
    pub ram: UsageMetric,
    pub swap: UsageMetric,
    pub zram: UsageMetric,
}

#[derive(Clone, Debug)]
pub struct ProcessEntry {
    pub pid: u32,
    pub user: String,
    pub state: char,
    pub cpu_percent: f64,
    pub mem_percent: f64,
    pub rss_bytes: u64,
    pub command: String,
}

#[derive(Clone, Debug)]
pub struct NetworkIoEntry {
    pub name: String,
    pub rx_bytes_per_sec: f64,
    pub tx_bytes_per_sec: f64,
    pub total_bytes_per_sec: f64,
}

#[derive(Clone, Debug)]
pub struct DiskIoEntry {
    pub name: String,
    pub read_bytes_per_sec: f64,
    pub write_bytes_per_sec: f64,
    pub total_bytes_per_sec: f64,
}

#[derive(Clone, Debug)]
pub struct SystemSnapshot {
    pub machine: MachineInfo,
    #[allow(dead_code)]
    pub load_average: LoadAverage,
    pub memory: MemoryMetrics,
    pub process_list: Vec<ProcessEntry>,
    pub uptime: Duration,
    pub overall_usage_percent: f64,
    pub cpus: Vec<CpuMetrics>,
    pub network_io: Vec<NetworkIoEntry>,
    pub disk_io: Vec<DiskIoEntry>,
    pub sensor_summary: Vec<SensorSummary>,
    pub accelerators: AcceleratorMetrics,
    #[allow(dead_code)]
    pub sensors: Vec<SensorReading>,
}

#[derive(Clone, Copy, Debug)]
pub struct CpuCounters {
    pub idle: u64,
    pub total: u64,
}

#[derive(Clone, Debug)]
pub struct RawCpuSample {
    pub id: usize,
    pub counters: CpuCounters,
    pub frequency_khz: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct RawSnapshot {
    pub captured_at: Instant,
    pub machine: MachineInfo,
    pub uptime: Duration,
    pub load_average: LoadAverage,
    pub total_mem_bytes: u64,
    pub available_mem_bytes: u64,
    pub swap_total_bytes: u64,
    pub swap_free_bytes: u64,
    pub overall_cpu: CpuCounters,
    pub cpus: Vec<RawCpuSample>,
    pub processes: Vec<RawProcessSample>,
    pub network: Vec<RawNetworkSample>,
    pub disks: Vec<RawDiskSample>,
    pub swaps: Vec<RawSwapSample>,
    pub accelerators: AcceleratorMetrics,
    pub sensors: Vec<SensorReading>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ProcessKey {
    pub pid: u32,
    pub start_time_ticks: u64,
}

#[derive(Clone, Debug)]
pub struct RawProcessSample {
    pub key: ProcessKey,
    pub user: String,
    pub state: char,
    pub total_time_ticks: u64,
    pub rss_bytes: u64,
    pub command: String,
}

#[derive(Clone, Debug)]
pub struct RawNetworkSample {
    pub name: String,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}

#[derive(Clone, Debug)]
pub struct RawDiskSample {
    pub name: String,
    pub read_bytes: u64,
    pub write_bytes: u64,
}

#[derive(Clone, Debug)]
pub struct RawSwapSample {
    pub name: String,
    pub size_bytes: u64,
    pub used_bytes: u64,
}
