use crate::metrics::{RuntimeState, SensorSummary, SystemSnapshot};

pub(super) fn format_frequency(frequency_khz: Option<u64>) -> String {
    match frequency_khz {
        Some(value) => format!("[{:.1}G]", value as f64 / 1_000_000.0),
        None => "[ n/a]".to_string(),
    }
}

pub(super) fn format_cpu_usage(percent: f64) -> String {
    format!("{:>5.1}%", percent)
}

pub(super) fn format_temperature(value: f64) -> String {
    format!("[{:>4.1}C]", value)
}

pub(super) fn format_uptime(uptime: std::time::Duration) -> String {
    let total_seconds = uptime.as_secs();
    let days = total_seconds / 86_400;
    let hours = (total_seconds % 86_400) / 3_600;
    let minutes = (total_seconds % 3_600) / 60;

    if days > 0 {
        format!("{days}d {hours:02}h {minutes:02}m")
    } else {
        format!("{hours:02}h {minutes:02}m")
    }
}

pub(super) fn format_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;

    if bytes as f64 >= GIB {
        format!("{:.1}G", bytes as f64 / GIB)
    } else if bytes as f64 >= MIB {
        format!("{:.0}M", bytes as f64 / MIB)
    } else {
        format!("{:.0}K", bytes as f64 / KIB)
    }
}

pub(super) fn format_rate(bytes_per_sec: f64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;

    if bytes_per_sec >= GIB {
        format!("{:.1}G/s", bytes_per_sec / GIB)
    } else if bytes_per_sec >= MIB {
        format!("{:.1}M/s", bytes_per_sec / MIB)
    } else if bytes_per_sec >= KIB {
        format!("{:.0}K/s", bytes_per_sec / KIB)
    } else {
        format!("{:.0}B/s", bytes_per_sec.max(0.0))
    }
}

pub(super) fn format_sensor_summary(summary: &[SensorSummary]) -> Option<String> {
    if summary.is_empty() {
        None
    } else {
        Some(
            summary
                .iter()
                .map(|sensor| {
                    format!(
                        "{} {}",
                        sensor.label,
                        format_temperature(sensor.temperature_c)
                    )
                })
                .collect::<Vec<_>>()
                .join("  "),
        )
    }
}

pub(super) fn format_accelerator_summary(snapshot: &SystemSnapshot) -> Option<String> {
    let mut sections = Vec::new();

    if let Some(gpu) = snapshot.accelerators.gpu.as_ref() {
        sections.push(format_gpu_summary(
            gpu.usage_percent,
            gpu.frequency_hz,
            gpu.runtime_state,
        ));
    }
    if let Some(npu) = snapshot.accelerators.npu.as_ref() {
        sections.push(format_npu_summary(
            npu.usage_percent,
            &npu.per_core_usage_percent,
            npu.frequency_hz,
            npu.runtime_state,
        ));
    }
    if let Some(vpu) = snapshot.accelerators.vpu.as_ref() {
        sections.push(format_vpu_summary(
            vpu.usage_percent,
            vpu.frequency_hz,
            vpu.runtime_state,
        ));
    }

    (!sections.is_empty()).then(|| sections.join("  "))
}

pub(super) fn format_gpu_summary(
    usage_percent: Option<f64>,
    frequency_hz: Option<u64>,
    runtime_state: Option<RuntimeState>,
) -> String {
    format_accelerator_section("gpu", usage_percent, &[], frequency_hz, runtime_state)
}

pub(super) fn format_npu_summary(
    usage_percent: Option<f64>,
    per_core_usage_percent: &[f64],
    frequency_hz: Option<u64>,
    runtime_state: Option<RuntimeState>,
) -> String {
    format_accelerator_section(
        "npu",
        usage_percent,
        per_core_usage_percent,
        frequency_hz,
        runtime_state,
    )
}

pub(super) fn format_vpu_summary(
    usage_percent: Option<f64>,
    frequency_hz: Option<u64>,
    runtime_state: Option<RuntimeState>,
) -> String {
    format_accelerator_section("vpu", usage_percent, &[], frequency_hz, runtime_state)
}

fn format_accelerator_section(
    label: &str,
    usage_percent: Option<f64>,
    per_core_usage_percent: &[f64],
    frequency_hz: Option<u64>,
    runtime_state: Option<RuntimeState>,
) -> String {
    let mut parts = vec![label.to_string()];

    if !per_core_usage_percent.is_empty() {
        let core_text = per_core_usage_percent
            .iter()
            .map(|value| format!("{value:.0}"))
            .collect::<Vec<_>>()
            .join("/");
        parts.push(format!("{core_text}%"));
    } else if let Some(usage_percent) = usage_percent {
        parts.push(format!("{usage_percent:.0}%"));
    } else if let Some(runtime_state) = runtime_state {
        parts.push(runtime_state.label().to_string());
    }

    if let Some(frequency_hz) = frequency_hz {
        parts.push(format!("[{}]", format_engine_frequency(frequency_hz)));
    }

    parts.join(" ")
}

fn format_engine_frequency(frequency_hz: u64) -> String {
    const MHZ: f64 = 1_000_000.0;
    const GHZ: f64 = 1_000_000_000.0;

    if frequency_hz as f64 >= GHZ {
        format!("{:.1}G", frequency_hz as f64 / GHZ)
    } else {
        format!("{:.0}M", frequency_hz as f64 / MHZ)
    }
}
