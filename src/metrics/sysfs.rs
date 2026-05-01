use std::collections::HashSet;
use std::ffi::CStr;
use std::fs;
use std::io;
use std::os::raw::c_char;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::model::{
    AcceleratorMetrics, GpuMetrics, MachineInfo, NpuMetrics, RuntimeState, SensorKind,
    SensorReading, SensorSource, VpuMetrics,
};
use super::procfs::ProcfsReader;

pub struct SysfsReader {
    root: PathBuf,
    vpu_perf_monitor: VpuPerfMonitor,
}

#[derive(Debug, Default)]
struct VpuPerfMonitor {
    initialized: bool,
    enable_path: Option<PathBuf>,
    restore_value: Option<String>,
}

impl Drop for SysfsReader {
    fn drop(&mut self) {
        self.restore_vpu_perf_monitor();
    }
}

impl SysfsReader {
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
            vpu_perf_monitor: VpuPerfMonitor::default(),
        }
    }

    pub fn machine_info(&self, procfs: &ProcfsReader) -> Result<MachineInfo> {
        let kernel = procfs.read_kernel_release()?;
        let machine_name = self
            .read_device_tree_model()
            .or_else(|| self.read_dmi_system_name())
            .unwrap_or_else(uname_machine);

        Ok(MachineInfo {
            machine_name,
            kernel,
            arch: uname_machine(),
        })
    }

    pub fn read_cpu_frequencies(
        &self,
        cpu_count: usize,
    ) -> std::collections::HashMap<usize, Option<u64>> {
        let mut frequencies = std::collections::HashMap::with_capacity(cpu_count);
        for cpu_id in 0..cpu_count {
            let base = self
                .root
                .join("devices/system/cpu")
                .join(format!("cpu{cpu_id}/cpufreq"));
            let scaling = base.join("scaling_cur_freq");
            let cpuinfo = base.join("cpuinfo_cur_freq");
            let frequency = read_u64(&scaling).or_else(|_| read_u64(&cpuinfo)).ok();
            frequencies.insert(cpu_id, frequency);
        }
        frequencies
    }

    pub fn read_accelerators(&mut self) -> AcceleratorMetrics {
        let gpu_usage = self
            .read_gpu_devfreq_usage()
            .or_else(|| self.read_gpu_debugfs_usage());
        let gpu_frequency_hz = self.read_devfreq_frequency_hz(&["gpu", "mali", "panthor"]);
        let gpu = if gpu_usage.is_some() || gpu_frequency_hz.is_some() {
            Some(GpuMetrics {
                usage_percent: gpu_usage,
                frequency_hz: gpu_frequency_hz,
                runtime_state: None,
            })
        } else {
            None
        };

        let npu_core_usage_percent = self.read_rknpu_core_usage();
        let npu_usage_percent = average_percent(&npu_core_usage_percent);
        let npu_frequency_hz = self.read_devfreq_frequency_hz(&["npu", "rknpu"]);
        let npu = if npu_usage_percent.is_some()
            || npu_frequency_hz.is_some()
            || !npu_core_usage_percent.is_empty()
        {
            Some(NpuMetrics {
                usage_percent: npu_usage_percent,
                per_core_usage_percent: npu_core_usage_percent,
                frequency_hz: npu_frequency_hz,
                runtime_state: None,
            })
        } else {
            None
        };

        let generic = AcceleratorMetrics {
            gpu,
            npu,
            vpu: None,
        };
        let cix = self.read_cix_accelerators();

        AcceleratorMetrics {
            gpu: cix.gpu.or(generic.gpu),
            npu: cix.npu.or(generic.npu),
            vpu: cix.vpu,
        }
    }

    pub fn read_sensors(&self) -> Result<Vec<SensorReading>> {
        let mut sensors = Vec::new();
        sensors.extend(self.read_thermal_sensors()?);
        sensors.extend(self.read_hwmon_sensors()?);
        dedupe_sensors(sensors)
    }

    fn read_thermal_sensors(&self) -> Result<Vec<SensorReading>> {
        let base = self.root.join("class/thermal");
        let entries = match fs::read_dir(&base) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error).context("reading thermal zone directory"),
        };
        let mut sensors = Vec::new();

        for entry in entries {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.starts_with("thermal_zone") {
                continue;
            }

            let path = entry.path();
            let raw_label = read_trimmed(path.join("type")).unwrap_or_else(|_| name.clone());
            let label = self
                .read_acpi_thermal_zone_name(&name, &path)
                .as_deref()
                .and_then(cix_acpi_thermal_label)
                .unwrap_or(raw_label.as_str())
                .to_string();
            let raw_temp = read_u64(path.join("temp"))
                .or_else(|_| read_i64(path.join("temp")).map(|value| value.max(0) as u64));
            if let Ok(raw_temp) = raw_temp {
                sensors.push(SensorReading {
                    source: SensorSource::ThermalZone,
                    kind: classify_sensor(&label, &label),
                    device_name: name.clone(),
                    label,
                    temperature_c: raw_temp as f64 / 1000.0,
                });
            }
        }

        sensors.sort_by(|left, right| left.label.cmp(&right.label));
        Ok(sensors)
    }

    fn read_hwmon_sensors(&self) -> Result<Vec<SensorReading>> {
        let base = self.root.join("class/hwmon");
        let entries = match fs::read_dir(&base) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error).context("reading hwmon directory"),
        };
        let mut sensors = Vec::new();

        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let device_name = read_trimmed(path.join("name"))
                .unwrap_or_else(|_| entry.file_name().to_string_lossy().into_owned());

            for channel in
                fs::read_dir(&path).with_context(|| format!("reading hwmon {device_name}"))?
            {
                let channel = channel?;
                let file_name = channel.file_name().to_string_lossy().into_owned();
                if !file_name.starts_with("temp") || !file_name.ends_with("_input") {
                    continue;
                }

                let stem = file_name.trim_end_matches("_input");
                let value = match read_i64(channel.path()) {
                    Ok(value) => value.max(0) as u64,
                    Err(_) => continue,
                };
                let label = read_trimmed(path.join(format!("{stem}_label")))
                    .unwrap_or_else(|_| stem.to_string());
                sensors.push(SensorReading {
                    source: SensorSource::Hwmon,
                    kind: classify_sensor(&device_name, &label),
                    device_name: device_name.clone(),
                    label,
                    temperature_c: value as f64 / 1000.0,
                });
            }
        }

        sensors.sort_by(|left, right| {
            left.device_name
                .cmp(&right.device_name)
                .then_with(|| left.label.cmp(&right.label))
        });
        Ok(sensors)
    }

    fn read_devfreq_frequency_hz(&self, matchers: &[&str]) -> Option<u64> {
        let path = self.find_devfreq_path(matchers)?;
        read_u64(path.join("cur_freq")).ok()
    }

    fn read_gpu_devfreq_usage(&self) -> Option<f64> {
        let path = self.find_devfreq_path(&["gpu", "mali", "panthor"])?;
        parse_devfreq_load(&read_trimmed(path.join("load")).ok()?)
    }

    fn read_gpu_debugfs_usage(&self) -> Option<f64> {
        let base = self.root.join("kernel/debug");
        let entries = fs::read_dir(&base).ok()?;

        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            if !name.starts_with("mali") {
                continue;
            }

            let usage = read_trimmed(entry.path().join("dvfs_utilization"))
                .ok()
                .and_then(|content| parse_mali_debugfs_usage(&content));
            if usage.is_some() {
                return usage;
            }
        }

        None
    }

    fn read_rknpu_core_usage(&self) -> Vec<f64> {
        let path = self.root.join("kernel/debug/rknpu/load");
        let content = match read_trimmed(path) {
            Ok(content) => content,
            Err(_) => return Vec::new(),
        };
        parse_rknpu_debugfs_load(&content)
    }

    fn find_devfreq_path(&self, matchers: &[&str]) -> Option<PathBuf> {
        let base = self.root.join("class/devfreq");
        let entries = fs::read_dir(&base).ok()?;
        let mut candidates = Vec::new();

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let dir_name = entry.file_name().to_string_lossy().into_owned();
            let device_name = read_trimmed(path.join("name")).unwrap_or_else(|_| dir_name.clone());
            let haystack = format!("{dir_name} {device_name}").to_ascii_lowercase();
            if matchers.iter().any(|matcher| haystack.contains(matcher)) {
                candidates.push(path);
            }
        }

        candidates.sort();
        candidates.into_iter().next()
    }

    fn read_named_devfreq_frequency_hz(&self, names: &[&str]) -> Option<u64> {
        let path = self.find_named_devfreq_path(names)?;
        read_u64(path.join("cur_freq")).ok()
    }

    fn find_named_devfreq_path(&self, names: &[&str]) -> Option<PathBuf> {
        let base = self.root.join("class/devfreq");
        let entries = fs::read_dir(&base).ok()?;

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let dir_name = entry.file_name().to_string_lossy().into_owned();
            let device_name = read_trimmed(path.join("name")).unwrap_or_else(|_| dir_name.clone());
            if names
                .iter()
                .any(|name| dir_name == *name || device_name == *name)
            {
                return Some(path);
            }
        }

        None
    }

    fn read_cix_accelerators(&mut self) -> AcceleratorMetrics {
        if !self.is_cix_platform() {
            return AcceleratorMetrics::default();
        }

        AcceleratorMetrics {
            gpu: self.read_cix_gpu_metrics(),
            npu: self.read_cix_npu_metrics(),
            vpu: self.read_cix_vpu_metrics(),
        }
    }

    fn is_cix_platform(&self) -> bool {
        let machine_name = self
            .read_dmi_system_name()
            .unwrap_or_default()
            .to_ascii_lowercase();
        machine_name.contains("radxa orion o6")
            || self.find_named_devfreq_path(&["CIXH5000:00"]).is_some()
            || self.find_named_devfreq_path(&["CIXH4000:00"]).is_some()
            || self.find_named_devfreq_path(&["CIXH3010:00"]).is_some()
    }

    fn read_cix_gpu_metrics(&self) -> Option<GpuMetrics> {
        let usage_percent = self.read_gpu_debugfs_usage();
        let frequency_hz = self.read_named_devfreq_frequency_hz(&["CIXH5000:00"]);
        let runtime_state =
            self.read_runtime_state("devices/platform/CIXH5000:00/power/runtime_status");

        (usage_percent.is_some() || frequency_hz.is_some() || runtime_state.is_some()).then_some(
            GpuMetrics {
                usage_percent,
                frequency_hz,
                runtime_state,
            },
        )
    }

    fn read_cix_npu_metrics(&self) -> Option<NpuMetrics> {
        let frequency_hz = self.read_named_devfreq_frequency_hz(&["CIXH4000:00"]);
        let runtime_state =
            self.read_runtime_state("devices/platform/CIXH4000:00/power/runtime_status");

        (frequency_hz.is_some() || runtime_state.is_some()).then_some(NpuMetrics {
            usage_percent: None,
            per_core_usage_percent: Vec::new(),
            frequency_hz,
            runtime_state,
        })
    }

    fn read_cix_vpu_metrics(&mut self) -> Option<VpuMetrics> {
        self.ensure_vpu_perf_monitor();

        let usage_percent = self.read_vpu_debugfs_usage();
        let frequency_hz = self.read_named_devfreq_frequency_hz(&["CIXH3010:00"]);
        let runtime_state =
            self.read_runtime_state("devices/platform/CIXH3010:00/power/runtime_status");

        (usage_percent.is_some() || frequency_hz.is_some() || runtime_state.is_some()).then_some(
            VpuMetrics {
                usage_percent,
                frequency_hz,
                runtime_state,
            },
        )
    }

    fn read_vpu_debugfs_usage(&self) -> Option<f64> {
        let path = self
            .root
            .join("kernel/debug/amvx/log/group/perf/utilization");
        parse_vpu_debugfs_utilization(&read_trimmed(path).ok()?)
    }

    fn read_runtime_state(&self, relative_path: &str) -> Option<RuntimeState> {
        let path = self.root.join(relative_path);
        parse_runtime_state(&read_trimmed(path).ok()?)
    }

    fn read_acpi_thermal_zone_name(
        &self,
        thermal_zone_name: &str,
        thermal_zone_path: &Path,
    ) -> Option<String> {
        let index = thermal_zone_name
            .strip_prefix("thermal_zone")?
            .parse::<usize>()
            .ok()?;
        let acpi_path = read_trimmed(thermal_zone_path.join("device/path")).or_else(|_| {
            read_trimmed(
                self.root
                    .join(format!("bus/acpi/devices/LNXTHERM:{index:02}/path")),
            )
        });
        let acpi_path = acpi_path.ok()?;

        acpi_path
            .rsplit(['.', '\\'])
            .find(|part| !part.is_empty())
            .map(ToString::to_string)
    }

    fn ensure_vpu_perf_monitor(&mut self) {
        if self.vpu_perf_monitor.initialized {
            return;
        }
        self.vpu_perf_monitor.initialized = true;

        let enable_path = self.root.join("kernel/debug/amvx/log/group/perf/enable");
        let current_value = match read_trimmed(&enable_path) {
            Ok(value) => value,
            Err(_) => return,
        };

        self.vpu_perf_monitor.enable_path = Some(enable_path.clone());
        if current_value == "1" {
            return;
        }

        if fs::write(&enable_path, "1\n").is_ok() {
            self.vpu_perf_monitor.restore_value = Some(current_value);
        }
    }

    fn restore_vpu_perf_monitor(&mut self) {
        let Some(enable_path) = self.vpu_perf_monitor.enable_path.as_ref() else {
            return;
        };
        let Some(restore_value) = self.vpu_perf_monitor.restore_value.as_ref() else {
            return;
        };

        let _ = fs::write(enable_path, format!("{restore_value}\n"));
        self.vpu_perf_monitor.restore_value = None;
    }

    fn read_device_tree_model(&self) -> Option<String> {
        read_trimmed(Path::new("/proc/device-tree/model"))
            .ok()
            .map(|value| value.trim_end_matches('\0').to_string())
            .filter(|value| !value.is_empty())
    }

    fn read_dmi_system_name(&self) -> Option<String> {
        let vendor = read_trimmed(self.root.join("devices/virtual/dmi/id/sys_vendor")).ok();
        let product = read_trimmed(self.root.join("devices/virtual/dmi/id/product_name")).ok();

        match (vendor, product) {
            (Some(vendor), Some(product)) if !vendor.is_empty() && !product.is_empty() => {
                Some(format!("{vendor} {product}"))
            }
            (None, Some(product)) | (Some(product), None) if !product.is_empty() => Some(product),
            _ => None,
        }
    }
}

fn dedupe_sensors(mut sensors: Vec<SensorReading>) -> Result<Vec<SensorReading>> {
    let mut seen = HashSet::new();
    sensors.retain(|sensor| {
        let key = (
            sensor.kind,
            sensor.label.to_ascii_lowercase(),
            (sensor.temperature_c * 1000.0).round() as i64,
        );
        seen.insert(key)
    });
    Ok(sensors)
}

fn average_percent(values: &[f64]) -> Option<f64> {
    (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
}

fn classify_sensor(device_name: &str, label: &str) -> SensorKind {
    let haystack = format!("{device_name} {label}").to_ascii_lowercase();

    if haystack.contains("gpu") || haystack.contains("amdgpu") {
        SensorKind::Gpu
    } else if haystack.contains("npu") {
        SensorKind::Npu
    } else if haystack.contains("soc") {
        SensorKind::Soc
    } else if haystack.contains("package")
        || haystack.contains("tctl")
        || haystack.contains("tdie")
        || haystack.contains("cpu")
        || haystack.contains("coretemp")
        || haystack.contains("k10temp")
        || haystack.contains("littlecore")
        || haystack.contains("bigcore")
    {
        SensorKind::Cpu
    } else if haystack.contains("nvme") || haystack.contains("composite") {
        SensorKind::Storage
    } else if haystack.contains("wifi") || haystack.contains("phy") || haystack.contains("mt79") {
        SensorKind::Wireless
    } else if haystack.contains("r8169") || haystack.contains("eth") || haystack.contains("net") {
        SensorKind::Network
    } else {
        SensorKind::Unknown
    }
}

fn cix_acpi_thermal_label(acpi_name: &str) -> Option<&'static str> {
    match acpi_name {
        "TZB0" => Some("cpu-b0"),
        "TZB1" => Some("cpu-b1"),
        "TZL0" => Some("cpu-l0"),
        "TZM0" => Some("cpu-m0"),
        "TZM1" => Some("cpu-m1"),
        "TZGT" => Some("gpu"),
        "ECTZ" => Some("soc"),
        _ => None,
    }
}

fn read_trimmed(path: impl AsRef<Path>) -> io::Result<String> {
    fs::read_to_string(path).map(|text| text.trim().to_string())
}

fn read_u64(path: impl AsRef<Path>) -> io::Result<u64> {
    read_trimmed(path)?.parse::<u64>().map_err(io::Error::other)
}

fn read_i64(path: impl AsRef<Path>) -> io::Result<i64> {
    read_trimmed(path)?.parse::<i64>().map_err(io::Error::other)
}

fn parse_devfreq_load(content: &str) -> Option<f64> {
    let prefix: String = content
        .trim()
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect();
    (!prefix.is_empty())
        .then_some(prefix)
        .and_then(|value| value.parse::<f64>().ok())
}

fn parse_mali_debugfs_usage(content: &str) -> Option<f64> {
    let mut busy_time = None;
    let mut idle_time = None;
    let parts = content.split_whitespace().collect::<Vec<_>>();

    for index in (0..parts.len()).step_by(2) {
        let key = parts.get(index)?.trim_end_matches(':');
        let value = parts.get(index + 1)?.parse::<u64>().ok()?;
        match key {
            "busy_time" => busy_time = Some(value),
            "idle_time" => idle_time = Some(value),
            _ => {}
        }
    }

    let busy_time = busy_time?;
    let idle_time = idle_time?;
    let total_time = busy_time + idle_time;
    (total_time > 0).then(|| (busy_time as f64 / total_time as f64) * 100.0)
}

fn parse_rknpu_debugfs_load(content: &str) -> Vec<f64> {
    let mut loads = Vec::new();
    let mut remaining = content;

    while let Some(core_start) = remaining.find("Core") {
        remaining = &remaining[core_start..];
        let Some(colon) = remaining.find(':') else {
            break;
        };
        let after_colon = &remaining[colon + 1..];
        let Some(percent) = after_colon.find('%') else {
            break;
        };
        if let Ok(value) = after_colon[..percent].trim().parse::<f64>() {
            loads.push(value);
        }
        remaining = &after_colon[percent + 1..];
    }

    loads
}

fn parse_vpu_debugfs_utilization(content: &str) -> Option<f64> {
    let value = content
        .trim()
        .strip_prefix("VPU Utilization:")?
        .trim()
        .trim_end_matches('%');
    value.parse::<f64>().ok()
}

fn parse_runtime_state(content: &str) -> Option<RuntimeState> {
    let state = content.trim().to_ascii_lowercase();
    if state.is_empty() {
        None
    } else if state.contains("active") {
        Some(RuntimeState::Active)
    } else if state.contains("suspend") {
        Some(RuntimeState::Suspended)
    } else {
        Some(RuntimeState::Unknown)
    }
}

fn uname_machine() -> String {
    let mut utsname = std::mem::MaybeUninit::<libc::utsname>::uninit();
    let rc = unsafe { libc::uname(utsname.as_mut_ptr()) };
    if rc != 0 {
        return std::env::consts::ARCH.to_string();
    }

    let utsname = unsafe { utsname.assume_init() };
    let machine = unsafe { CStr::from_ptr(utsname.machine.as_ptr() as *const c_char) };
    machine.to_string_lossy().trim().to_string()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn reads_hwmon_sensors_on_generic_linux() {
        let temp = tempdir().unwrap();
        let hwmon = temp.path().join("class/hwmon/hwmon0");
        fs::create_dir_all(&hwmon).unwrap();
        fs::write(hwmon.join("name"), "k10temp\n").unwrap();
        fs::write(hwmon.join("temp1_input"), "38125\n").unwrap();
        fs::write(hwmon.join("temp1_label"), "Tctl\n").unwrap();
        fs::write(hwmon.join("temp2_input"), "41000\n").unwrap();
        fs::write(hwmon.join("temp2_label"), "Composite\n").unwrap();

        let reader = SysfsReader::new(temp.path());
        let sensors = reader.read_hwmon_sensors().unwrap();

        assert_eq!(sensors.len(), 2);
        assert_eq!(sensors[0].kind, SensorKind::Cpu);
    }

    #[test]
    fn names_cix_acpi_thermal_zones_from_acpi_paths() {
        let temp = tempdir().unwrap();
        let thermal0 = temp.path().join("class/thermal/thermal_zone0");
        let thermal4 = temp.path().join("class/thermal/thermal_zone4");
        let acpi0 = temp.path().join("bus/acpi/devices/LNXTHERM:00");
        let acpi4 = temp.path().join("bus/acpi/devices/LNXTHERM:04");
        fs::create_dir_all(&thermal0).unwrap();
        fs::create_dir_all(&thermal4).unwrap();
        fs::create_dir_all(&acpi0).unwrap();
        fs::create_dir_all(&acpi4).unwrap();
        fs::create_dir_all(thermal0.join("device")).unwrap();

        fs::write(thermal0.join("type"), "acpitz\n").unwrap();
        fs::write(thermal0.join("temp"), "38125\n").unwrap();
        fs::write(thermal0.join("device/path"), "\\_SB_.TZB0\n").unwrap();
        fs::write(thermal4.join("type"), "acpitz\n").unwrap();
        fs::write(thermal4.join("temp"), "41000\n").unwrap();
        fs::write(acpi4.join("path"), "\\_SB_.TZGT\n").unwrap();

        let reader = SysfsReader::new(temp.path());
        let sensors = reader.read_thermal_sensors().unwrap();

        assert_eq!(
            sensors
                .iter()
                .map(|sensor| (sensor.label.as_str(), sensor.kind))
                .collect::<Vec<_>>(),
            vec![("cpu-b0", SensorKind::Cpu), ("gpu", SensorKind::Gpu)]
        );
    }

    #[test]
    fn builds_dmi_machine_name() {
        let temp = tempdir().unwrap();
        let dmi = temp.path().join("devices/virtual/dmi/id");
        fs::create_dir_all(&dmi).unwrap();
        fs::write(dmi.join("sys_vendor"), "Example Corp\n").unwrap();
        fs::write(dmi.join("product_name"), "Workstation\n").unwrap();

        let reader = SysfsReader::new(temp.path());

        assert_eq!(
            reader.read_dmi_system_name().as_deref(),
            Some("Example Corp Workstation")
        );
    }

    #[test]
    fn reads_gpu_and_npu_metrics_from_devfreq_and_debugfs() {
        let temp = tempdir().unwrap();
        let gpu = temp.path().join("class/devfreq/fb000000.gpu");
        let npu = temp.path().join("class/devfreq/fdab0000.npu");
        let debugfs = temp.path().join("kernel/debug/rknpu");
        fs::create_dir_all(&gpu).unwrap();
        fs::create_dir_all(&npu).unwrap();
        fs::create_dir_all(&debugfs).unwrap();

        fs::write(gpu.join("name"), "fb000000.gpu\n").unwrap();
        fs::write(gpu.join("load"), "7@300000000Hz\n").unwrap();
        fs::write(gpu.join("cur_freq"), "300000000\n").unwrap();

        fs::write(npu.join("name"), "fdab0000.npu\n").unwrap();
        fs::write(npu.join("cur_freq"), "1000000000\n").unwrap();
        fs::write(
            debugfs.join("load"),
            "NPU load:  Core0:  5%, Core1:  8%, Core2:  0%,\n",
        )
        .unwrap();

        let mut reader = SysfsReader::new(temp.path());
        let accelerators = reader.read_accelerators();

        let gpu = accelerators.gpu.expect("gpu metrics");
        assert_eq!(gpu.frequency_hz, Some(300_000_000));
        assert_eq!(gpu.usage_percent, Some(7.0));

        let npu = accelerators.npu.expect("npu metrics");
        assert_eq!(npu.frequency_hz, Some(1_000_000_000));
        assert_eq!(npu.per_core_usage_percent, vec![5.0, 8.0, 0.0]);
        assert_eq!(npu.usage_percent, Some((5.0 + 8.0 + 0.0) / 3.0));
        assert_eq!(npu.runtime_state, None);
    }

    #[test]
    fn reads_cix_accelerators_and_restores_vpu_perf_monitor() {
        let temp = tempdir().unwrap();
        let gpu = temp.path().join("class/devfreq/CIXH5000:00");
        let npu = temp.path().join("class/devfreq/CIXH4000:00");
        let vpu = temp.path().join("class/devfreq/CIXH3010:00");
        let mali = temp.path().join("kernel/debug/mali0");
        let vpu_perf = temp.path().join("kernel/debug/amvx/log/group/perf");
        let gpu_power = temp.path().join("devices/platform/CIXH5000:00/power");
        let npu_power = temp.path().join("devices/platform/CIXH4000:00/power");
        let vpu_power = temp.path().join("devices/platform/CIXH3010:00/power");

        fs::create_dir_all(&gpu).unwrap();
        fs::create_dir_all(&npu).unwrap();
        fs::create_dir_all(&vpu).unwrap();
        fs::create_dir_all(&mali).unwrap();
        fs::create_dir_all(&vpu_perf).unwrap();
        fs::create_dir_all(&gpu_power).unwrap();
        fs::create_dir_all(&npu_power).unwrap();
        fs::create_dir_all(&vpu_power).unwrap();

        fs::write(gpu.join("name"), "CIXH5000:00\n").unwrap();
        fs::write(gpu.join("cur_freq"), "72000000\n").unwrap();
        fs::write(
            mali.join("dvfs_utilization"),
            "busy_time: 25 idle_time: 75 protm_time: 0\n",
        )
        .unwrap();
        fs::write(gpu_power.join("runtime_status"), "active\n").unwrap();

        fs::write(npu.join("name"), "CIXH4000:00\n").unwrap();
        fs::write(npu.join("cur_freq"), "1200000000\n").unwrap();
        fs::write(npu_power.join("runtime_status"), "suspended\n").unwrap();

        fs::write(vpu.join("name"), "CIXH3010:00\n").unwrap();
        fs::write(vpu.join("cur_freq"), "150000000\n").unwrap();
        fs::write(vpu_power.join("runtime_status"), "active\n").unwrap();
        fs::write(vpu_perf.join("enable"), "0\n").unwrap();
        fs::write(vpu_perf.join("utilization"), "VPU Utilization: 38.25%\n").unwrap();

        {
            let mut reader = SysfsReader::new(temp.path());
            let accelerators = reader.read_accelerators();

            let gpu = accelerators.gpu.expect("gpu metrics");
            assert_eq!(gpu.frequency_hz, Some(72_000_000));
            assert_eq!(gpu.usage_percent, Some(25.0));
            assert_eq!(gpu.runtime_state, Some(RuntimeState::Active));

            let npu = accelerators.npu.expect("npu metrics");
            assert_eq!(npu.frequency_hz, Some(1_200_000_000));
            assert_eq!(npu.runtime_state, Some(RuntimeState::Suspended));
            assert_eq!(npu.usage_percent, None);

            let vpu = accelerators.vpu.expect("vpu metrics");
            assert_eq!(vpu.frequency_hz, Some(150_000_000));
            assert_eq!(vpu.usage_percent, Some(38.25));
            assert_eq!(vpu.runtime_state, Some(RuntimeState::Active));

            assert_eq!(fs::read_to_string(vpu_perf.join("enable")).unwrap(), "1\n");
        }

        assert_eq!(fs::read_to_string(vpu_perf.join("enable")).unwrap(), "0\n");
    }

    #[test]
    fn parses_devfreq_load_prefix() {
        assert_eq!(parse_devfreq_load("1@300000000Hz"), Some(1.0));
        assert_eq!(parse_devfreq_load("100@1000000000Hz"), Some(100.0));
        assert_eq!(parse_devfreq_load(""), None);
    }

    #[test]
    fn parses_vpu_debugfs_utilization_text() {
        assert_eq!(
            parse_vpu_debugfs_utilization("VPU Utilization: 38.25%"),
            Some(38.25)
        );
        assert_eq!(
            parse_vpu_debugfs_utilization("VPU Performance Monitor is OFF"),
            None
        );
    }
}
