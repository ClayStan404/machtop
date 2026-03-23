mod model;
mod procfs;
mod profile;
mod sysfs;

use std::collections::HashMap;
use std::time::Instant;

use anyhow::Result;

pub use model::{
    CpuMetrics, DiskIoEntry, MachineInfo, MemoryMetrics, NetworkIoEntry, ProcessEntry,
    SensorSummary, SystemSnapshot, UsageMetric,
};

use model::{
    CpuCounters, ProcessKey, RawCpuSample, RawDiskSample, RawNetworkSample, RawProcessSample,
    RawSnapshot,
};
use procfs::ProcfsReader;
use profile::BoardProfile;
use sysfs::SysfsReader;

pub struct MetricsSampler {
    procfs: ProcfsReader,
    sysfs: SysfsReader,
    machine: MachineInfo,
    previous: Option<RawSnapshot>,
    last_process_parse_errors: usize,
}

impl MetricsSampler {
    pub fn new() -> Result<Self> {
        let procfs = ProcfsReader::new("/proc");
        let sysfs = SysfsReader::new("/sys");
        let machine = sysfs.machine_info(&procfs)?;

        Ok(Self {
            procfs,
            sysfs,
            machine,
            previous: None,
            last_process_parse_errors: 0,
        })
    }

    pub fn sample(&mut self) -> Result<SystemSnapshot> {
        let current = self.collect_raw_snapshot()?;
        let profile = BoardProfile::detect(&self.machine, &current.sensors);
        let snapshot = derive_snapshot(self.previous.as_ref(), &current, profile);

        self.previous = Some(current);
        Ok(snapshot)
    }

    fn collect_raw_snapshot(&mut self) -> Result<RawSnapshot> {
        let captured_at = Instant::now();
        let cpu_stat = self.procfs.read_cpu_stat()?;
        let memory = self.procfs.read_memory_info()?;
        let swaps = self.procfs.read_swaps()?;
        let load_average = self.procfs.read_load_average()?;
        let uptime = self.procfs.read_uptime()?;
        let frequencies = self.sysfs.read_cpu_frequencies(cpu_stat.per_cpu.len());
        let cpus = cpu_stat
            .per_cpu
            .into_iter()
            .enumerate()
            .map(|(id, counters)| RawCpuSample {
                id,
                counters,
                frequency_khz: frequencies.get(&id).copied().flatten(),
            })
            .collect();
        let network = self.procfs.read_network_samples()?;
        let disks = self.procfs.read_disk_samples()?;
        let accelerators = self.sysfs.read_accelerators();
        let sensors = self.sysfs.read_sensors()?;
        let (processes, process_parse_errors) =
            self.procfs.read_process_samples(self.procfs.page_size())?;
        self.last_process_parse_errors = process_parse_errors;

        Ok(RawSnapshot {
            captured_at,
            machine: self.machine.clone(),
            uptime,
            load_average,
            total_mem_bytes: memory.total_bytes,
            available_mem_bytes: memory.available_bytes,
            swap_total_bytes: memory.swap_total_bytes,
            swap_free_bytes: memory.swap_free_bytes,
            overall_cpu: cpu_stat.overall,
            cpus,
            processes,
            network,
            disks,
            swaps,
            accelerators,
            sensors,
        })
    }
}

fn derive_snapshot(
    previous: Option<&RawSnapshot>,
    current: &RawSnapshot,
    profile: BoardProfile,
) -> SystemSnapshot {
    let reset_cpu_deltas = previous
        .map(|previous| previous.cpus.len() != current.cpus.len())
        .unwrap_or(true);
    let previous_processes = if reset_cpu_deltas {
        HashMap::new()
    } else {
        previous
            .map(|snapshot| {
                snapshot
                    .processes
                    .iter()
                    .map(|process| (process.key, process.total_time_ticks))
                    .collect::<HashMap<_, _>>()
            })
            .unwrap_or_default()
    };
    let total_cpu_delta = if reset_cpu_deltas {
        0
    } else {
        previous
            .map(|snapshot| {
                current
                    .overall_cpu
                    .total
                    .saturating_sub(snapshot.overall_cpu.total)
            })
            .unwrap_or(0)
    };
    let elapsed_seconds = previous
        .map(|snapshot| {
            current
                .captured_at
                .saturating_duration_since(snapshot.captured_at)
                .as_secs_f64()
        })
        .unwrap_or(0.0);

    let overall_usage_percent = if reset_cpu_deltas {
        0.0
    } else {
        previous
            .map(|snapshot| usage_percent(snapshot.overall_cpu, current.overall_cpu))
            .unwrap_or(0.0)
    };

    let sensor_summary = profile.sensor_summary(&current.sensors);
    let cpus = current
        .cpus
        .iter()
        .enumerate()
        .map(|(index, cpu)| CpuMetrics {
            id: cpu.id,
            usage_percent: previous
                .filter(|_| !reset_cpu_deltas)
                .and_then(|snapshot| snapshot.cpus.get(index))
                .map(|previous_cpu| usage_percent(previous_cpu.counters, cpu.counters))
                .unwrap_or(0.0),
            frequency_khz: cpu.frequency_khz,
            temperature_c: profile.per_cpu_temperature(cpu.id, &current.sensors),
        })
        .collect();
    let memory = derive_memory_metrics(current);
    let network_io = derive_network_io(&current.network, previous, elapsed_seconds);
    let disk_io = derive_disk_io(&current.disks, previous, elapsed_seconds);
    let process_list = derive_process_list(
        &current.processes,
        &previous_processes,
        current.total_mem_bytes,
        current.cpus.len(),
        total_cpu_delta,
    );

    SystemSnapshot {
        machine: current.machine.clone(),
        load_average: current.load_average.clone(),
        memory,
        process_list,
        uptime: current.uptime,
        overall_usage_percent,
        cpus,
        network_io,
        disk_io,
        sensor_summary,
        accelerators: current.accelerators.clone(),
        sensors: current.sensors.clone(),
    }
}

fn derive_memory_metrics(current: &RawSnapshot) -> MemoryMetrics {
    let ram =
        UsageMetric::from_total_and_available(current.total_mem_bytes, current.available_mem_bytes);
    let swap =
        UsageMetric::from_total_and_available(current.swap_total_bytes, current.swap_free_bytes);
    let (zram_total_bytes, zram_used_bytes) = current
        .swaps
        .iter()
        .filter(|swap| swap.name.starts_with("zram"))
        .fold((0_u64, 0_u64), |(total, used), swap| {
            (
                total.saturating_add(swap.size_bytes),
                used.saturating_add(swap.used_bytes),
            )
        });
    let zram = UsageMetric::from_total_and_used(zram_total_bytes, zram_used_bytes);

    MemoryMetrics { ram, swap, zram }
}

fn derive_process_list(
    current: &[RawProcessSample],
    previous_processes: &HashMap<ProcessKey, u64>,
    total_mem_bytes: u64,
    cpu_count: usize,
    total_cpu_delta: u64,
) -> Vec<ProcessEntry> {
    let mut process_list = current
        .iter()
        .map(|process| {
            let previous_ticks = previous_processes
                .get(&process.key)
                .copied()
                .unwrap_or(process.total_time_ticks);
            let delta_ticks = process.total_time_ticks.saturating_sub(previous_ticks);
            let cpu_percent = if total_cpu_delta == 0 || cpu_count == 0 {
                0.0
            } else {
                (delta_ticks as f64 * cpu_count as f64 / total_cpu_delta as f64) * 100.0
            };
            let mem_percent = if total_mem_bytes == 0 {
                0.0
            } else {
                (process.rss_bytes as f64 / total_mem_bytes as f64) * 100.0
            };

            ProcessEntry {
                pid: process.key.pid,
                user: process.user.clone(),
                state: process.state,
                cpu_percent,
                mem_percent,
                rss_bytes: process.rss_bytes,
                command: process.command.clone(),
            }
        })
        .collect::<Vec<_>>();

    process_list.sort_by(|left, right| {
        right
            .cpu_percent
            .total_cmp(&left.cpu_percent)
            .then_with(|| right.mem_percent.total_cmp(&left.mem_percent))
            .then_with(|| left.pid.cmp(&right.pid))
    });
    process_list
}

fn derive_network_io(
    current: &[RawNetworkSample],
    previous: Option<&RawSnapshot>,
    elapsed_seconds: f64,
) -> Vec<NetworkIoEntry> {
    let previous = previous
        .map(|snapshot| {
            snapshot
                .network
                .iter()
                .map(|sample| (sample.name.as_str(), (sample.rx_bytes, sample.tx_bytes)))
                .collect::<HashMap<_, _>>()
        })
        .unwrap_or_default();
    let mut entries = current
        .iter()
        .filter(|sample| !should_hide_network_device(&sample.name))
        .map(|sample| {
            let (previous_rx, previous_tx) = previous
                .get(sample.name.as_str())
                .copied()
                .unwrap_or((sample.rx_bytes, sample.tx_bytes));
            let rx_bytes_per_sec = bytes_per_second(previous_rx, sample.rx_bytes, elapsed_seconds);
            let tx_bytes_per_sec = bytes_per_second(previous_tx, sample.tx_bytes, elapsed_seconds);

            NetworkIoEntry {
                name: sample.name.clone(),
                rx_bytes_per_sec,
                tx_bytes_per_sec,
                total_bytes_per_sec: rx_bytes_per_sec + tx_bytes_per_sec,
            }
        })
        .collect::<Vec<_>>();

    entries.sort_by(|left, right| {
        right
            .total_bytes_per_sec
            .total_cmp(&left.total_bytes_per_sec)
            .then_with(|| left.name.cmp(&right.name))
    });
    entries
}

fn derive_disk_io(
    current: &[RawDiskSample],
    previous: Option<&RawSnapshot>,
    elapsed_seconds: f64,
) -> Vec<DiskIoEntry> {
    let previous = previous
        .map(|snapshot| {
            snapshot
                .disks
                .iter()
                .map(|sample| {
                    (
                        sample.name.as_str(),
                        (sample.read_bytes, sample.write_bytes),
                    )
                })
                .collect::<HashMap<_, _>>()
        })
        .unwrap_or_default();
    let mut entries = current
        .iter()
        .filter(|sample| !should_hide_disk_device(&sample.name))
        .map(|sample| {
            let (previous_read, previous_write) = previous
                .get(sample.name.as_str())
                .copied()
                .unwrap_or((sample.read_bytes, sample.write_bytes));
            let read_bytes_per_sec =
                bytes_per_second(previous_read, sample.read_bytes, elapsed_seconds);
            let write_bytes_per_sec =
                bytes_per_second(previous_write, sample.write_bytes, elapsed_seconds);

            DiskIoEntry {
                name: sample.name.clone(),
                read_bytes_per_sec,
                write_bytes_per_sec,
                total_bytes_per_sec: read_bytes_per_sec + write_bytes_per_sec,
            }
        })
        .collect::<Vec<_>>();

    entries.sort_by(|left, right| {
        right
            .total_bytes_per_sec
            .total_cmp(&left.total_bytes_per_sec)
            .then_with(|| left.name.cmp(&right.name))
    });
    entries
}

fn bytes_per_second(previous: u64, current: u64, elapsed_seconds: f64) -> f64 {
    if elapsed_seconds <= f64::EPSILON {
        0.0
    } else {
        current.saturating_sub(previous) as f64 / elapsed_seconds
    }
}

fn should_hide_network_device(name: &str) -> bool {
    name == "lo"
        || name.starts_with("veth")
        || name.starts_with("docker")
        || name.starts_with("br-")
        || name.starts_with("virbr")
        || name.starts_with("tun")
        || name.starts_with("tap")
}

fn should_hide_disk_device(name: &str) -> bool {
    name.starts_with("loop")
        || name.starts_with("ram")
        || name.starts_with("dm-")
        || name.starts_with("zram")
        || name.starts_with("mtdblock")
        || is_partition_device(name)
}

fn is_partition_device(name: &str) -> bool {
    is_letter_disk_partition(name, "sd")
        || is_letter_disk_partition(name, "vd")
        || is_letter_disk_partition(name, "xvd")
        || is_p_suffixed_partition(name, "nvme")
        || is_p_suffixed_partition(name, "mmcblk")
}

fn is_letter_disk_partition(name: &str, prefix: &str) -> bool {
    let Some(rest) = name.strip_prefix(prefix) else {
        return false;
    };
    let split_at = rest
        .char_indices()
        .find(|(_, ch)| ch.is_ascii_digit())
        .map(|(index, _)| index);
    let Some(split_at) = split_at else {
        return false;
    };
    let (letters, digits) = rest.split_at(split_at);
    !letters.is_empty()
        && !digits.is_empty()
        && letters.chars().all(|ch| ch.is_ascii_lowercase())
        && digits.chars().all(|ch| ch.is_ascii_digit())
}

fn is_p_suffixed_partition(name: &str, prefix: &str) -> bool {
    let Some(rest) = name.strip_prefix(prefix) else {
        return false;
    };
    let Some((base, partition)) = rest.rsplit_once('p') else {
        return false;
    };
    !base.is_empty()
        && !partition.is_empty()
        && base.chars().all(|ch| ch.is_ascii_alphanumeric())
        && partition.chars().all(|ch| ch.is_ascii_digit())
}

fn usage_percent(previous: CpuCounters, current: CpuCounters) -> f64 {
    let total_delta = current.total.saturating_sub(previous.total);
    let idle_delta = current.idle.saturating_sub(previous.idle);

    if total_delta == 0 {
        return 0.0;
    }

    let busy_delta = total_delta.saturating_sub(idle_delta);
    (busy_delta as f64 / total_delta as f64) * 100.0
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;
    use crate::metrics::model::{
        AcceleratorMetrics, LoadAverage, RawSwapSample, SensorKind, SensorReading, SensorSource,
    };

    #[test]
    fn derives_process_cpu_after_delta() {
        let machine = MachineInfo {
            machine_name: "Generic".into(),
            kernel: "6.0".into(),
            arch: "x86_64".into(),
        };
        let start = Instant::now();
        let previous = RawSnapshot {
            captured_at: start,
            machine: machine.clone(),
            uptime: Duration::from_secs(1),
            load_average: LoadAverage {
                one: 0.1,
                five: 0.2,
                fifteen: 0.3,
            },
            total_mem_bytes: 8 * 1024,
            available_mem_bytes: 4 * 1024,
            swap_total_bytes: 2 * 1024,
            swap_free_bytes: 1024,
            overall_cpu: CpuCounters {
                idle: 100,
                total: 200,
            },
            cpus: vec![RawCpuSample {
                id: 0,
                counters: CpuCounters {
                    idle: 50,
                    total: 100,
                },
                frequency_khz: Some(2_000_000),
            }],
            processes: vec![RawProcessSample {
                key: ProcessKey {
                    pid: 1,
                    start_time_ticks: 10,
                },
                user: "root".into(),
                state: 'S',
                total_time_ticks: 20,
                rss_bytes: 1024,
                command: "init".into(),
            }],
            network: vec![RawNetworkSample {
                name: "eth0".into(),
                rx_bytes: 1_000,
                tx_bytes: 2_000,
            }],
            disks: vec![RawDiskSample {
                name: "nvme0n1".into(),
                read_bytes: 5_000,
                write_bytes: 10_000,
            }],
            swaps: vec![RawSwapSample {
                name: "zram0".into(),
                size_bytes: 2 * 1024,
                used_bytes: 1024,
            }],
            accelerators: AcceleratorMetrics::default(),
            sensors: vec![SensorReading {
                source: SensorSource::Hwmon,
                device_name: "k10temp".into(),
                label: "Tctl".into(),
                kind: SensorKind::Cpu,
                temperature_c: 42.0,
            }],
        };
        let current = RawSnapshot {
            overall_cpu: CpuCounters {
                idle: 120,
                total: 260,
            },
            cpus: vec![RawCpuSample {
                id: 0,
                counters: CpuCounters {
                    idle: 60,
                    total: 130,
                },
                frequency_khz: Some(2_100_000),
            }],
            processes: vec![RawProcessSample {
                total_time_ticks: 40,
                ..previous.processes[0].clone()
            }],
            network: vec![RawNetworkSample {
                name: "eth0".into(),
                rx_bytes: 5_000,
                tx_bytes: 6_000,
            }],
            disks: vec![RawDiskSample {
                name: "nvme0n1".into(),
                read_bytes: 9_000,
                write_bytes: 20_000,
            }],
            captured_at: start + Duration::from_secs(1),
            uptime: Duration::from_secs(2),
            ..previous.clone()
        };

        let snapshot = derive_snapshot(Some(&previous), &current, BoardProfile::GenericLinux);

        assert!(snapshot.overall_usage_percent > 0.0);
        assert_eq!(snapshot.process_list.len(), 1);
        assert!(snapshot.process_list[0].cpu_percent > 0.0);
        assert_eq!(snapshot.network_io.len(), 1);
        assert!(snapshot.network_io[0].total_bytes_per_sec > 0.0);
        assert_eq!(snapshot.disk_io.len(), 1);
        assert!(snapshot.disk_io[0].total_bytes_per_sec > 0.0);
        assert_eq!(snapshot.memory.ram.used_percent, 50.0);
        assert_eq!(snapshot.memory.swap.used_percent, 50.0);
        assert_eq!(snapshot.memory.zram.used_percent, 50.0);
        assert_eq!(snapshot.sensor_summary[0].label, "cpu");
    }

    #[test]
    fn resets_usage_when_cpu_topology_changes() {
        let machine = MachineInfo {
            machine_name: "Generic".into(),
            kernel: "6.0".into(),
            arch: "x86_64".into(),
        };
        let start = Instant::now();
        let previous = RawSnapshot {
            captured_at: start,
            machine: machine.clone(),
            uptime: Duration::from_secs(1),
            load_average: LoadAverage {
                one: 0.1,
                five: 0.2,
                fifteen: 0.3,
            },
            total_mem_bytes: 8 * 1024,
            available_mem_bytes: 4 * 1024,
            swap_total_bytes: 0,
            swap_free_bytes: 0,
            overall_cpu: CpuCounters {
                idle: 100,
                total: 200,
            },
            cpus: vec![RawCpuSample {
                id: 0,
                counters: CpuCounters {
                    idle: 50,
                    total: 100,
                },
                frequency_khz: None,
            }],
            processes: Vec::new(),
            network: Vec::new(),
            disks: Vec::new(),
            swaps: Vec::new(),
            accelerators: AcceleratorMetrics::default(),
            sensors: Vec::new(),
        };
        let current = RawSnapshot {
            overall_cpu: CpuCounters {
                idle: 120,
                total: 260,
            },
            cpus: vec![
                RawCpuSample {
                    id: 0,
                    counters: CpuCounters {
                        idle: 60,
                        total: 130,
                    },
                    frequency_khz: None,
                },
                RawCpuSample {
                    id: 1,
                    counters: CpuCounters {
                        idle: 60,
                        total: 130,
                    },
                    frequency_khz: None,
                },
            ],
            captured_at: start + Duration::from_secs(1),
            uptime: Duration::from_secs(2),
            ..previous.clone()
        };

        let snapshot = derive_snapshot(Some(&previous), &current, BoardProfile::GenericLinux);

        assert_eq!(snapshot.overall_usage_percent, 0.0);
        assert!(snapshot.cpus.iter().all(|cpu| cpu.usage_percent == 0.0));
    }

    #[test]
    fn filters_virtual_devices_and_sorts_by_total_rate() {
        let start = Instant::now();
        let previous = RawSnapshot {
            captured_at: start,
            machine: MachineInfo {
                machine_name: "Generic".into(),
                kernel: "6.0".into(),
                arch: "x86_64".into(),
            },
            uptime: Duration::from_secs(1),
            load_average: LoadAverage {
                one: 0.1,
                five: 0.2,
                fifteen: 0.3,
            },
            total_mem_bytes: 8 * 1024,
            available_mem_bytes: 4 * 1024,
            swap_total_bytes: 0,
            swap_free_bytes: 0,
            overall_cpu: CpuCounters {
                idle: 100,
                total: 200,
            },
            cpus: Vec::new(),
            processes: Vec::new(),
            network: vec![
                RawNetworkSample {
                    name: "lo".into(),
                    rx_bytes: 100,
                    tx_bytes: 100,
                },
                RawNetworkSample {
                    name: "eth0".into(),
                    rx_bytes: 1_000,
                    tx_bytes: 1_000,
                },
                RawNetworkSample {
                    name: "wlan0".into(),
                    rx_bytes: 500,
                    tx_bytes: 500,
                },
            ],
            disks: vec![
                RawDiskSample {
                    name: "loop0".into(),
                    read_bytes: 10,
                    write_bytes: 10,
                },
                RawDiskSample {
                    name: "mmcblk1p1".into(),
                    read_bytes: 20,
                    write_bytes: 20,
                },
                RawDiskSample {
                    name: "zram0".into(),
                    read_bytes: 30,
                    write_bytes: 30,
                },
                RawDiskSample {
                    name: "mtdblock0".into(),
                    read_bytes: 40,
                    write_bytes: 40,
                },
                RawDiskSample {
                    name: "sda".into(),
                    read_bytes: 1_000,
                    write_bytes: 2_000,
                },
                RawDiskSample {
                    name: "nvme0n1".into(),
                    read_bytes: 3_000,
                    write_bytes: 4_000,
                },
            ],
            swaps: vec![
                RawSwapSample {
                    name: "zram0".into(),
                    size_bytes: 1_000,
                    used_bytes: 250,
                },
                RawSwapSample {
                    name: "zram1".into(),
                    size_bytes: 3_000,
                    used_bytes: 750,
                },
                RawSwapSample {
                    name: "swapfile".into(),
                    size_bytes: 8_000,
                    used_bytes: 4_000,
                },
            ],
            accelerators: AcceleratorMetrics::default(),
            sensors: Vec::new(),
        };
        let current = RawSnapshot {
            captured_at: start + Duration::from_secs(1),
            network: vec![
                RawNetworkSample {
                    name: "lo".into(),
                    rx_bytes: 500,
                    tx_bytes: 500,
                },
                RawNetworkSample {
                    name: "eth0".into(),
                    rx_bytes: 10_000,
                    tx_bytes: 5_000,
                },
                RawNetworkSample {
                    name: "wlan0".into(),
                    rx_bytes: 2_000,
                    tx_bytes: 1_000,
                },
            ],
            disks: vec![
                RawDiskSample {
                    name: "loop0".into(),
                    read_bytes: 100,
                    write_bytes: 100,
                },
                RawDiskSample {
                    name: "mmcblk1p1".into(),
                    read_bytes: 200,
                    write_bytes: 200,
                },
                RawDiskSample {
                    name: "zram0".into(),
                    read_bytes: 300,
                    write_bytes: 300,
                },
                RawDiskSample {
                    name: "mtdblock0".into(),
                    read_bytes: 400,
                    write_bytes: 400,
                },
                RawDiskSample {
                    name: "sda".into(),
                    read_bytes: 2_000,
                    write_bytes: 3_000,
                },
                RawDiskSample {
                    name: "nvme0n1".into(),
                    read_bytes: 12_000,
                    write_bytes: 10_000,
                },
            ],
            ..previous.clone()
        };

        let snapshot = derive_snapshot(Some(&previous), &current, BoardProfile::GenericLinux);

        assert_eq!(
            snapshot
                .network_io
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            vec!["eth0", "wlan0"]
        );
        assert_eq!(
            snapshot
                .disk_io
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            vec!["nvme0n1", "sda"]
        );
        assert_eq!(snapshot.memory.zram.used_percent, 25.0);
    }
}
