use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};

use super::model::ProcessKey;
use super::model::{
    CpuCounters, LoadAverage, RawDiskSample, RawNetworkSample, RawProcessSample, RawSwapSample,
};

pub struct ProcfsReader {
    root: PathBuf,
    uid_map: HashMap<u32, String>,
    page_size: u64,
}

pub struct CpuStatSnapshot {
    pub overall: CpuCounters,
    pub per_cpu: Vec<CpuCounters>,
}

pub struct RawMemoryInfo {
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub swap_total_bytes: u64,
    pub swap_free_bytes: u64,
}

impl ProcfsReader {
    pub fn new(root: impl AsRef<Path>) -> Self {
        let root = root.as_ref().to_path_buf();
        let uid_map = read_passwd_users().unwrap_or_default();
        let page_size = page_size();

        Self {
            root,
            uid_map,
            page_size,
        }
    }

    pub fn page_size(&self) -> u64 {
        self.page_size
    }

    pub fn read_cpu_stat(&self) -> Result<CpuStatSnapshot> {
        let content = fs::read_to_string(self.root.join("stat")).context("reading /proc/stat")?;
        parse_proc_stat(&content)
    }

    pub fn read_load_average(&self) -> Result<LoadAverage> {
        let content =
            fs::read_to_string(self.root.join("loadavg")).context("reading /proc/loadavg")?;
        parse_load_average(&content)
    }

    pub fn read_memory_info(&self) -> Result<RawMemoryInfo> {
        let content =
            fs::read_to_string(self.root.join("meminfo")).context("reading /proc/meminfo")?;
        parse_meminfo(&content)
    }

    pub fn read_uptime(&self) -> Result<Duration> {
        let content =
            fs::read_to_string(self.root.join("uptime")).context("reading /proc/uptime")?;
        parse_uptime(&content)
    }

    pub fn read_network_samples(&self) -> Result<Vec<RawNetworkSample>> {
        let content =
            fs::read_to_string(self.root.join("net/dev")).context("reading /proc/net/dev")?;
        parse_net_dev(&content)
    }

    pub fn read_disk_samples(&self) -> Result<Vec<RawDiskSample>> {
        let content =
            fs::read_to_string(self.root.join("diskstats")).context("reading /proc/diskstats")?;
        parse_diskstats(&content)
    }

    pub fn read_swaps(&self) -> Result<Vec<RawSwapSample>> {
        let content = fs::read_to_string(self.root.join("swaps")).context("reading /proc/swaps")?;
        parse_swaps(&content)
    }

    pub fn read_kernel_release(&self) -> Result<String> {
        read_trimmed(self.root.join("sys/kernel/osrelease"))
            .context("reading /proc/sys/kernel/osrelease")
    }

    pub fn read_process_samples(&self, page_size: u64) -> Result<(Vec<RawProcessSample>, usize)> {
        let entries = fs::read_dir(&self.root).context("reading /proc for process metrics")?;
        let mut process_list = Vec::new();
        let mut parse_errors = 0_usize;

        for entry in entries {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !name.chars().all(|ch| ch.is_ascii_digit()) {
                continue;
            }

            match read_process_sample(&entry.path(), &self.uid_map, page_size) {
                Ok(Some(sample)) => process_list.push(sample),
                Ok(None) => {}
                Err(_) => {
                    parse_errors += 1;
                }
            }
        }

        Ok((process_list, parse_errors))
    }
}

pub fn parse_proc_stat(content: &str) -> Result<CpuStatSnapshot> {
    let mut counters = Vec::new();

    for line in content.lines() {
        if !line.starts_with("cpu") {
            continue;
        }

        let mut parts = line.split_whitespace();
        let name = parts.next().unwrap_or_default();
        if name != "cpu" && !name[3..].chars().all(|c| c.is_ascii_digit()) {
            continue;
        }

        let values: Vec<u64> = parts.filter_map(|part| part.parse::<u64>().ok()).collect();
        if values.len() < 4 {
            continue;
        }

        let idle = values[3] + values.get(4).copied().unwrap_or(0);
        let total = values.iter().sum();
        counters.push(CpuCounters { idle, total });
    }

    if counters.is_empty() {
        bail!("no cpu counters found in /proc/stat");
    }

    Ok(CpuStatSnapshot {
        overall: counters[0],
        per_cpu: counters.into_iter().skip(1).collect(),
    })
}

pub fn parse_load_average(content: &str) -> Result<LoadAverage> {
    let mut parts = content.split_whitespace();

    let one = parts
        .next()
        .context("missing 1-minute load average")?
        .parse::<f64>()
        .context("parsing 1-minute load average")?;
    let five = parts
        .next()
        .context("missing 5-minute load average")?
        .parse::<f64>()
        .context("parsing 5-minute load average")?;
    let fifteen = parts
        .next()
        .context("missing 15-minute load average")?
        .parse::<f64>()
        .context("parsing 15-minute load average")?;

    Ok(LoadAverage { one, five, fifteen })
}

pub fn parse_meminfo(content: &str) -> Result<RawMemoryInfo> {
    let mut total_kib = None;
    let mut available_kib = None;
    let mut swap_total_kib = None;
    let mut swap_free_kib = None;

    for line in content.lines() {
        if let Some(value) = line.strip_prefix("MemTotal:") {
            total_kib = parse_meminfo_kib(value);
        } else if let Some(value) = line.strip_prefix("MemAvailable:") {
            available_kib = parse_meminfo_kib(value);
        } else if let Some(value) = line.strip_prefix("SwapTotal:") {
            swap_total_kib = parse_meminfo_kib(value);
        } else if let Some(value) = line.strip_prefix("SwapFree:") {
            swap_free_kib = parse_meminfo_kib(value);
        }
    }

    let total_bytes = total_kib.context("MemTotal missing from /proc/meminfo")? * 1024;
    let available_bytes = available_kib.context("MemAvailable missing from /proc/meminfo")? * 1024;
    let swap_total_bytes = swap_total_kib.unwrap_or(0) * 1024;
    let swap_free_bytes = swap_free_kib.unwrap_or(0) * 1024;

    Ok(RawMemoryInfo {
        total_bytes,
        available_bytes,
        swap_total_bytes,
        swap_free_bytes,
    })
}

fn parse_meminfo_kib(value: &str) -> Option<u64> {
    value.split_whitespace().next()?.parse::<u64>().ok()
}

pub fn parse_uptime(content: &str) -> Result<Duration> {
    let seconds = content
        .split_whitespace()
        .next()
        .context("missing uptime seconds")?
        .parse::<f64>()
        .context("parsing uptime seconds")?;
    Ok(Duration::from_secs_f64(seconds))
}

pub fn parse_net_dev(content: &str) -> Result<Vec<RawNetworkSample>> {
    let mut samples = Vec::new();

    for line in content.lines().skip(2) {
        let Some((name, rest)) = line.split_once(':') else {
            continue;
        };
        let values = rest
            .split_whitespace()
            .map(str::parse::<u64>)
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("parsing /proc/net/dev counters")?;
        if values.len() < 9 {
            continue;
        }

        samples.push(RawNetworkSample {
            name: name.trim().to_string(),
            rx_bytes: values[0],
            tx_bytes: values[8],
        });
    }

    Ok(samples)
}

pub fn parse_diskstats(content: &str) -> Result<Vec<RawDiskSample>> {
    let mut samples = Vec::new();

    for line in content.lines() {
        let parts = line.split_whitespace().collect::<Vec<_>>();
        if parts.len() < 10 {
            continue;
        }

        let read_sectors = parts[5]
            .parse::<u64>()
            .with_context(|| format!("parsing read sectors for disk {}", parts[2]))?;
        let write_sectors = parts[9]
            .parse::<u64>()
            .with_context(|| format!("parsing write sectors for disk {}", parts[2]))?;

        samples.push(RawDiskSample {
            name: parts[2].to_string(),
            read_bytes: read_sectors.saturating_mul(512),
            write_bytes: write_sectors.saturating_mul(512),
        });
    }

    Ok(samples)
}

pub fn parse_swaps(content: &str) -> Result<Vec<RawSwapSample>> {
    let mut samples = Vec::new();

    for line in content.lines().skip(1) {
        let parts = line.split_whitespace().collect::<Vec<_>>();
        if parts.len() < 5 {
            continue;
        }

        let size_bytes = parts[2]
            .parse::<u64>()
            .with_context(|| format!("parsing swap size for {}", parts[0]))?
            .saturating_mul(1024);
        let used_bytes = parts[3]
            .parse::<u64>()
            .with_context(|| format!("parsing swap used for {}", parts[0]))?
            .saturating_mul(1024);
        let name = Path::new(parts[0])
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(parts[0])
            .to_string();

        samples.push(RawSwapSample {
            name,
            size_bytes,
            used_bytes,
        });
    }

    Ok(samples)
}

fn read_process_sample(
    process_path: &Path,
    uid_map: &HashMap<u32, String>,
    page_size: u64,
) -> Result<Option<RawProcessSample>> {
    let pid = process_path
        .file_name()
        .and_then(|name| name.to_str())
        .context("process path missing pid")?
        .parse::<u32>()
        .context("parsing pid")?;

    let stat = match fs::read_to_string(process_path.join("stat")) {
        Ok(content) => content,
        Err(error) if is_missing_or_permission(&error) => return Ok(None),
        Err(error) => return Err(error).context("reading process stat"),
    };
    let status = match fs::read_to_string(process_path.join("status")) {
        Ok(content) => content,
        Err(error) if is_missing_or_permission(&error) => return Ok(None),
        Err(error) => return Err(error).context("reading process status"),
    };
    let cmdline = match fs::read(process_path.join("cmdline")) {
        Ok(content) => content,
        Err(error) if is_missing_or_permission(&error) => Vec::new(),
        Err(error) => return Err(error).context("reading process cmdline"),
    };
    let comm = fs::read_to_string(process_path.join("comm")).unwrap_or_default();

    parse_process_sample(pid, &stat, &status, &cmdline, &comm, uid_map, page_size)
}

pub fn parse_process_sample(
    pid: u32,
    stat: &str,
    status: &str,
    cmdline: &[u8],
    comm_fallback: &str,
    uid_map: &HashMap<u32, String>,
    page_size: u64,
) -> Result<Option<RawProcessSample>> {
    let open = match stat.find('(') {
        Some(index) => index,
        None => return Ok(None),
    };
    let close = match stat.rfind(')') {
        Some(index) => index,
        None => return Ok(None),
    };
    let comm = stat[open + 1..close].to_string();
    let fields = stat[close + 2..].split_whitespace().collect::<Vec<_>>();
    if fields.len() < 22 {
        return Ok(None);
    }

    let state = fields[0].chars().next().unwrap_or('?');
    let utime = fields[11].parse::<u64>().ok();
    let stime = fields[12].parse::<u64>().ok();
    let start_time_ticks = fields[19].parse::<u64>().ok();
    let rss_pages = fields[21]
        .parse::<i64>()
        .ok()
        .map(|value| value.max(0) as u64);
    let (utime, stime, start_time_ticks, rss_pages) =
        match (utime, stime, start_time_ticks, rss_pages) {
            (Some(utime), Some(stime), Some(start_time_ticks), Some(rss_pages)) => {
                (utime, stime, start_time_ticks, rss_pages)
            }
            _ => return Ok(None),
        };

    let uid = parse_status_uid(status).unwrap_or(0);
    let command = parse_process_command(cmdline, comm_fallback)
        .or(Some(comm))
        .unwrap_or_else(|| pid.to_string());
    let user = uid_map
        .get(&uid)
        .cloned()
        .unwrap_or_else(|| uid.to_string());

    Ok(Some(RawProcessSample {
        key: ProcessKey {
            pid,
            start_time_ticks,
        },
        user,
        state,
        total_time_ticks: utime + stime,
        rss_bytes: rss_pages.saturating_mul(page_size),
        command,
    }))
}

fn parse_status_uid(status: &str) -> Option<u32> {
    for line in status.lines() {
        if let Some(value) = line.strip_prefix("Uid:") {
            return value
                .split_whitespace()
                .next()
                .and_then(|part| part.parse::<u32>().ok());
        }
    }
    None
}

fn parse_process_command(cmdline: &[u8], comm_fallback: &str) -> Option<String> {
    if !cmdline.is_empty() {
        let parts = cmdline
            .split(|byte| *byte == 0)
            .filter(|part| !part.is_empty())
            .map(|part| String::from_utf8_lossy(part).to_string())
            .collect::<Vec<_>>();
        if !parts.is_empty() {
            return Some(parts.join(" "));
        }
    }

    let trimmed = comm_fallback.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn read_passwd_users() -> Result<HashMap<u32, String>> {
    let content = fs::read_to_string("/etc/passwd").context("reading /etc/passwd")?;
    let mut users = HashMap::new();

    for line in content.lines() {
        let parts = line.split(':').collect::<Vec<_>>();
        if parts.len() < 3 {
            continue;
        }
        if let Ok(uid) = parts[2].parse::<u32>() {
            users.insert(uid, parts[0].to_string());
        }
    }

    Ok(users)
}

fn page_size() -> u64 {
    let size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if size > 0 { size as u64 } else { 4096 }
}

fn read_trimmed(path: impl AsRef<Path>) -> io::Result<String> {
    fs::read_to_string(path).map(|text| text.trim().to_string())
}

fn is_missing_or_permission(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::PermissionDenied
    )
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    #[test]
    fn parses_proc_stat() {
        let snapshot = parse_proc_stat(
            "cpu  10 0 10 80 0 0 0 0 0 0\ncpu0 5 0 5 40 0 0 0 0 0 0\ncpu1 5 0 5 40 0 0 0 0 0 0\n",
        )
        .unwrap();

        assert_eq!(snapshot.per_cpu.len(), 2);
        assert_eq!(snapshot.overall.total, 100);
    }

    #[test]
    fn parses_meminfo_with_swap_totals() {
        let info = parse_meminfo(
            "MemTotal:       32778300 kB\n\
             MemAvailable:   27295040 kB\n\
             SwapTotal:       4194300 kB\n\
             SwapFree:        3145725 kB\n",
        )
        .unwrap();

        assert_eq!(info.total_bytes, 32_778_300 * 1024);
        assert_eq!(info.available_bytes, 27_295_040 * 1024);
        assert_eq!(info.swap_total_bytes, 4_194_300 * 1024);
        assert_eq!(info.swap_free_bytes, 3_145_725 * 1024);
    }

    #[test]
    fn parses_process_sample_from_stat_and_status() {
        let mut uid_map = HashMap::new();
        uid_map.insert(1000, "clay".to_string());

        let sample = parse_process_sample(
            42,
            "42 (worker) R 1 1 1 1 1 0 0 0 0 0 10 5 0 0 20 0 1 0 99 1024 5 0 0 0 0 0 0 0 0 0 0 0",
            "Name:\tworker\nUid:\t1000\t1000\t1000\t1000\n",
            b"/usr/bin/worker\0--flag\0",
            "worker\n",
            &uid_map,
            4096,
        )
        .unwrap()
        .unwrap();

        assert_eq!(sample.key.pid, 42);
        assert_eq!(sample.user, "clay");
        assert_eq!(sample.command, "/usr/bin/worker --flag");
        assert_eq!(sample.rss_bytes, 5 * 4096);
    }

    #[test]
    fn parses_network_samples_from_proc_net_dev() {
        let samples = parse_net_dev(
            "Inter-|   Receive                                                |  Transmit\n\
             face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed\n\
              lo: 123 0 0 0 0 0 0 0 456 0 0 0 0 0 0 0\n\
            eth0: 2048 0 0 0 0 0 0 0 4096 0 0 0 0 0 0 0\n",
        )
        .unwrap();

        assert_eq!(samples.len(), 2);
        assert_eq!(samples[1].name, "eth0");
        assert_eq!(samples[1].rx_bytes, 2048);
        assert_eq!(samples[1].tx_bytes, 4096);
    }

    #[test]
    fn parses_diskstats_samples_as_bytes() {
        let samples = parse_diskstats(
            "259 0 nvme0n1 100 0 200 0 300 0 400 0 0 0 0 0 0 0 0 0 0\n\
             7 0 loop0 1 0 2 0 3 0 4 0 0 0 0 0 0 0 0 0 0\n",
        )
        .unwrap();

        assert_eq!(samples.len(), 2);
        assert_eq!(samples[0].name, "nvme0n1");
        assert_eq!(samples[0].read_bytes, 200 * 512);
        assert_eq!(samples[0].write_bytes, 400 * 512);
    }

    #[test]
    fn parses_swaps_and_uses_device_basename() {
        let swaps = parse_swaps(
            "Filename\t\t\t\tType\t\tSize\t\tUsed\t\tPriority\n\
             /dev/zram0                              partition\t4194300\t\t128\t\t100\n\
             /swapfile                               file\t\t2048\t\t1024\t\t-2\n",
        )
        .unwrap();

        assert_eq!(swaps.len(), 2);
        assert_eq!(swaps[0].name, "zram0");
        assert_eq!(swaps[0].size_bytes, 4_194_300 * 1024);
        assert_eq!(swaps[0].used_bytes, 128 * 1024);
        assert_eq!(swaps[1].name, "swapfile");
    }
}
