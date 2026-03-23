use std::collections::HashMap;

use super::model::{MachineInfo, SensorKind, SensorReading, SensorSummary};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BoardProfile {
    GenericLinux,
    RockchipRk3588,
}

impl BoardProfile {
    pub fn detect(machine: &MachineInfo, sensors: &[SensorReading]) -> Self {
        let machine_name = machine.machine_name.to_ascii_lowercase();
        if machine_name.contains("rk3588")
            || sensors.iter().any(|sensor| {
                matches!(
                    sensor.label.as_str(),
                    "littlecore-thermal"
                        | "bigcore0-thermal"
                        | "bigcore1-thermal"
                        | "gpu-thermal"
                        | "npu-thermal"
                        | "soc-thermal"
                )
            })
        {
            Self::RockchipRk3588
        } else {
            Self::GenericLinux
        }
    }

    pub fn sensor_summary(self, sensors: &[SensorReading]) -> Vec<SensorSummary> {
        match self {
            Self::GenericLinux => generic_summary(sensors),
            Self::RockchipRk3588 => rk3588_summary(sensors),
        }
    }

    pub fn per_cpu_temperature(self, cpu_id: usize, sensors: &[SensorReading]) -> Option<f64> {
        match self {
            Self::GenericLinux => None,
            Self::RockchipRk3588 => {
                let labels = if cpu_id < 4 {
                    ["littlecore-thermal", "soc-thermal"]
                } else if cpu_id < 6 {
                    ["bigcore0-thermal", "soc-thermal"]
                } else {
                    ["bigcore1-thermal", "soc-thermal"]
                };

                labels.into_iter().find_map(|label| {
                    sensors
                        .iter()
                        .find(|sensor| sensor.label == label)
                        .map(|sensor| sensor.temperature_c)
                })
            }
        }
    }
}

fn generic_summary(sensors: &[SensorReading]) -> Vec<SensorSummary> {
    let mut max_by_kind = HashMap::<SensorKind, f64>::new();
    for sensor in sensors {
        if matches!(
            sensor.kind,
            SensorKind::Unknown | SensorKind::Network | SensorKind::Wireless
        ) {
            continue;
        }

        max_by_kind
            .entry(sensor.kind)
            .and_modify(|current| *current = current.max(sensor.temperature_c))
            .or_insert(sensor.temperature_c);
    }

    [
        (SensorKind::Cpu, "cpu"),
        (SensorKind::Gpu, "gpu"),
        (SensorKind::Npu, "npu"),
        (SensorKind::Soc, "soc"),
        (SensorKind::Storage, "nvme"),
    ]
    .into_iter()
    .filter_map(|(kind, label)| {
        max_by_kind
            .get(&kind)
            .copied()
            .map(|temperature_c| SensorSummary {
                label: label.to_string(),
                temperature_c,
            })
    })
    .collect()
}

fn rk3588_summary(sensors: &[SensorReading]) -> Vec<SensorSummary> {
    let cpu = ["littlecore-thermal", "bigcore0-thermal", "bigcore1-thermal"]
        .into_iter()
        .filter_map(|label| lookup_sensor(sensors, label))
        .max_by(|left, right| left.total_cmp(right));
    let gpu = lookup_sensor(sensors, "gpu-thermal");
    let npu = lookup_sensor(sensors, "npu-thermal");
    let soc = lookup_sensor(sensors, "soc-thermal");

    [("cpu", cpu), ("gpu", gpu), ("npu", npu), ("soc", soc)]
        .into_iter()
        .filter_map(|(label, temperature_c)| {
            temperature_c.map(|temperature_c| SensorSummary {
                label: label.to_string(),
                temperature_c,
            })
        })
        .collect()
}

fn lookup_sensor(sensors: &[SensorReading], label: &str) -> Option<f64> {
    sensors
        .iter()
        .find(|sensor| sensor.label == label)
        .map(|sensor| sensor.temperature_c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::model::SensorSource;

    #[test]
    fn generic_summary_prefers_known_sensor_order() {
        let sensors = vec![
            SensorReading {
                source: SensorSource::Hwmon,
                device_name: "nvme".into(),
                label: "Composite".into(),
                kind: SensorKind::Storage,
                temperature_c: 40.0,
            },
            SensorReading {
                source: SensorSource::Hwmon,
                device_name: "k10temp".into(),
                label: "Tctl".into(),
                kind: SensorKind::Cpu,
                temperature_c: 55.0,
            },
        ];

        let summary = BoardProfile::GenericLinux.sensor_summary(&sensors);

        assert_eq!(summary[0].label, "cpu");
        assert_eq!(summary[1].label, "nvme");
    }
}
