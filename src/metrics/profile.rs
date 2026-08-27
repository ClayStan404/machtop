use std::collections::HashMap;

use super::model::{MachineInfo, SensorKind, SensorReading, SensorSummary};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BoardProfile {
    GenericLinux,
    CixSky1,
    RockchipRk3588,
}

impl BoardProfile {
    pub fn detect(machine: &MachineInfo, sensors: &[SensorReading]) -> Self {
        let machine_name = machine.machine_name.to_ascii_lowercase();
        if machine_name.contains("radxa orion o6")
            || machine_name.contains("cix")
            || sensors.iter().any(|sensor| {
                matches!(
                    sensor.label.as_str(),
                    "cpu-b0" | "cpu-b1" | "cpu-l0" | "cpu-m0" | "cpu-m1"
                )
            })
        {
            Self::CixSky1
        } else if machine_name.contains("rk3588")
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
            Self::CixSky1 => cix_sky1_summary(sensors),
            Self::RockchipRk3588 => rk3588_summary(sensors),
        }
    }

    pub fn per_cpu_temperature(self, cpu_id: usize, sensors: &[SensorReading]) -> Option<f64> {
        match self {
            Self::GenericLinux => None,
            Self::CixSky1 => {
                let labels: &[&str] = match cpu_id {
                    0 | 1 => &["cpu-b1"],
                    2..=5 => &["cpu-l0", "soc"],
                    6 | 7 => &["cpu-m0"],
                    8 | 9 => &["cpu-m1"],
                    10 | 11 => &["cpu-b0"],
                    _ => return None,
                };

                labels
                    .iter()
                    .find_map(|label| lookup_sensor(sensors, label))
            }
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

fn cix_sky1_summary(sensors: &[SensorReading]) -> Vec<SensorSummary> {
    let cpu = ["cpu-b0", "cpu-b1", "cpu-l0", "cpu-m0", "cpu-m1"]
        .into_iter()
        .filter_map(|label| lookup_sensor(sensors, label))
        .max_by(|left, right| left.total_cmp(right));
    let gpu = lookup_sensor(sensors, "gpu");
    let soc = lookup_sensor(sensors, "soc");
    let storage = sensors
        .iter()
        .filter(|sensor| sensor.kind == SensorKind::Storage)
        .map(|sensor| sensor.temperature_c)
        .max_by(|left, right| left.total_cmp(right));

    [("cpu", cpu), ("gpu", gpu), ("soc", soc), ("nvme", storage)]
        .into_iter()
        .filter_map(|(label, temperature_c)| {
            temperature_c.map(|temperature_c| SensorSummary {
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

    #[test]
    fn cix_sky1_summary_uses_acpi_thermal_zone_labels() {
        let sensors = vec![
            SensorReading {
                source: SensorSource::ThermalZone,
                device_name: "thermal_zone0".into(),
                label: "cpu-b0".into(),
                kind: SensorKind::Cpu,
                temperature_c: 37.0,
            },
            SensorReading {
                source: SensorSource::ThermalZone,
                device_name: "thermal_zone4".into(),
                label: "gpu".into(),
                kind: SensorKind::Gpu,
                temperature_c: 35.0,
            },
            SensorReading {
                source: SensorSource::ThermalZone,
                device_name: "thermal_zone5".into(),
                label: "soc".into(),
                kind: SensorKind::Soc,
                temperature_c: 31.0,
            },
            SensorReading {
                source: SensorSource::Hwmon,
                device_name: "nvme".into(),
                label: "Composite".into(),
                kind: SensorKind::Storage,
                temperature_c: 43.0,
            },
        ];

        let summary = BoardProfile::CixSky1.sensor_summary(&sensors);

        assert_eq!(
            summary
                .iter()
                .map(|sensor| sensor.label.as_str())
                .collect::<Vec<_>>(),
            vec!["cpu", "gpu", "soc", "nvme"]
        );
        assert_eq!(
            BoardProfile::CixSky1.per_cpu_temperature(10, &sensors),
            Some(37.0)
        );
        assert_eq!(
            BoardProfile::CixSky1.per_cpu_temperature(2, &sensors),
            Some(31.0)
        );
        assert_eq!(
            BoardProfile::CixSky1.per_cpu_temperature(5, &sensors),
            Some(31.0)
        );
    }

    #[test]
    fn detects_rk3588_from_sensors_and_maps_big_cores() {
        let machine = MachineInfo {
            machine_name: "Generic Linux board".into(),
            kernel: "6.0".into(),
            arch: "aarch64".into(),
        };
        let sensors = vec![
            SensorReading {
                source: SensorSource::ThermalZone,
                device_name: "thermal_zone0".into(),
                label: "bigcore0-thermal".into(),
                kind: SensorKind::Cpu,
                temperature_c: 48.0,
            },
            SensorReading {
                source: SensorSource::ThermalZone,
                device_name: "thermal_zone1".into(),
                label: "bigcore1-thermal".into(),
                kind: SensorKind::Cpu,
                temperature_c: 52.0,
            },
        ];

        let profile = BoardProfile::detect(&machine, &sensors);

        assert_eq!(profile, BoardProfile::RockchipRk3588);
        assert_eq!(profile.per_cpu_temperature(4, &sensors), Some(48.0));
        assert_eq!(profile.per_cpu_temperature(7, &sensors), Some(52.0));
    }
}
