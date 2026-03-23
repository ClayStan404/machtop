# machtop

[![CI](https://github.com/ClayStan404/machtop/actions/workflows/ci.yml/badge.svg)](https://github.com/ClayStan404/machtop/actions/workflows/ci.yml)
[![Release Debian Package](https://github.com/ClayStan404/machtop/actions/workflows/release-deb.yml/badge.svg)](https://github.com/ClayStan404/machtop/actions/workflows/release-deb.yml)
[![License: GPLv3%2B](https://img.shields.io/badge/license-GPLv3%2B-blue.svg)](LICENSE)

`machtop` is a Linux terminal system monitor focused on machine-level telemetry.
It provides a compact `top`/`htop`-style view with better visibility into CPU
frequency, thermal sensors, memory pressure, storage/network I/O, and
accelerator data when the platform exposes it.

The project targets common `x86_64` and `aarch64` Linux systems, while also
including RK3588-specific enhancements for boards such as the Radxa ROCK 5T.

![machtop screenshot on Radxa ROCK 5T](docs/images/machtop-rock5t.png)

## Why machtop

Most terminal monitors are good at process and CPU usage, but weak on hardware
signals. `machtop` is built for developers who want one compact TUI that can
show:

- per-CPU usage and current frequency
- machine thermal data from standard Linux interfaces
- RAM, zram, and swap pressure in the summary view
- accelerator and sensor summaries in the overview panel
- per-adapter network throughput and per-device disk throughput
- best-effort GPU and NPU visibility
- top processes, memory, and uptime

It reads directly from kernel interfaces. There is no agent, daemon, or
background service to install.

## Features

- Dedicated overall CPU status row plus per-CPU usage bars with current frequency and optional per-CPU temperature
- Summary panel with RAM, zram, and swap percentage meters
- Summary panel with machine, accelerator, and sensor status
- Uptime and top processes
- Real-time network RX/TX and disk read/write rates
- Sensor collection from both `/sys/class/thermal` and `/sys/class/hwmon`
- Best-effort GPU and NPU frequency and utilization collection
- Graceful degradation when optional metrics are unavailable
- Support for generic Linux machines plus RK3588 profile-specific hints

## Installation

### Debian package

Prebuilt `.deb` packages are published for:

- `amd64`
- `arm64`
- `arm64_bookworm` compatibility build for Debian 12 based systems such as many RK3588 boards

Download the appropriate package from the
[Releases](https://github.com/ClayStan404/machtop/releases) page and install it:

```bash
sudo dpkg -i machtop_<version>_<arch>.deb
```

If your arm64 machine is on Debian 12 / Bookworm and the standard `arm64`
package asks for a newer `libc6`, use the `arm64_bookworm` compatibility asset.

### Build from source

Requirements:

- Linux
- Rust toolchain
- ANSI-capable terminal

Run directly:

```bash
cargo run
```

Build a release binary:

```bash
cargo build --release
./target/release/machtop
```

## Usage

Launch `machtop` in an interactive terminal:

```bash
machtop
```

Controls:

- `q`: quit
- `Ctrl+C`: quit

Current layout:

- top: `Summary`
- middle: `CPU`
- bottom-left: `Processes`
- bottom-right: `Network & Disk I/O`

## Platform Support

`machtop` is designed to work on:

- generic Linux on `x86_64`
- generic Linux on `aarch64`
- RK3588 boards with additional profile-specific sensor grouping

Support is intentionally capability-based rather than driven by a hardcoded
hardware whitelist. If a machine exposes a metric through supported kernel
interfaces, `machtop` will try to show it. If not, the UI hides that value
instead of failing.

## Data Sources and Permissions

### Metrics available to normal users

These usually come from standard kernel interfaces and do not require elevated
privileges:

- CPU usage from `/proc/stat`
- RAM and swap usage from `/proc/meminfo`
- zram usage from `/proc/swaps`
- uptime from `/proc/uptime`
- process information from `/proc/<pid>/...`
- network throughput from `/proc/net/dev`
- disk throughput from `/proc/diskstats`
- CPU frequency from `/sys/devices/system/cpu/cpu*/cpufreq`
- thermal sensors from `/sys/class/thermal`
- `hwmon` sensors from `/sys/class/hwmon`
- accelerator frequency from `/sys/class/devfreq`

### Optional privileged metrics

Some drivers expose extra accelerator telemetry only through `debugfs`. Those
signals are treated as optional:

- GPU utilization may fall back to driver-specific `debugfs` nodes
- NPU utilization may come from driver-specific `debugfs` nodes

If these files are not readable, `machtop` does not require `sudo` for the
entire application. The unavailable values are simply omitted.

### Notes on GPU and NPU telemetry

- RAM is always shown in the summary panel.
- `swap` is derived from system-wide `SwapTotal` and `SwapFree`.
- `zram` is derived from `zram*` entries in `/proc/swaps` and shown separately
  when present.
- Accelerator and sensor summaries are shown in the left side of `Summary` when
  available.
- Network and disk throughput are computed from consecutive samples rather than
  lifetime counters.
- Virtual interfaces and pseudo block devices are hidden by default to reduce
  noise.
- GPU frequency and NPU frequency are collected from `/sys/class/devfreq` when
  available.
- GPU utilization may come from `/sys/class/devfreq/*/load`, but that value is
  driver-defined and should be treated as best-effort rather than a universal
  Linux ABI.
- NPU utilization is shown only when the platform exposes a readable,
  driver-specific interface. Otherwise `machtop` shows frequency only.

## RK3588 Support

On RK3588 systems, `machtop` applies a profile layer on top of the generic
Linux collectors.

Current RK3588-specific behavior includes:

- improved grouping for `cpu`, `gpu`, `npu`, and `soc` thermal summaries
- per-CPU temperature mapping derived from known RK3588 thermal zone labels

If those labels are not present, `machtop` falls back to generic Linux
behavior.

## Project Layout

- `src/main.rs`: terminal lifecycle and event loop
- `src/app.rs`: refresh timing and application state
- `src/metrics/`: procfs/sysfs collection, derivation, and board profiles
- `src/ui.rs`: layout, formatting, and theme logic

## Development

Useful commands:

```bash
cargo fmt
cargo test
cargo clippy --all-targets -- -D warnings
```

CI runs on both `amd64` and `arm64` and checks:

- formatting
- tests
- clippy

## Releases

The repository includes GitHub Actions workflows for both regular CI and Debian
package releases.

### Create a tagged release

```bash
git tag v0.1.0
git push origin v0.1.0
```

### Trigger a release manually

From the GitHub Actions page, run `Release Debian Package` and provide a
`release_tag`, for example:

```text
v0.1.0
```

The release workflow will:

- build `.deb` packages for `amd64` and `arm64`
- generate `.sha256` checksum files
- install each package on the runner and run a smoke test
- upload the assets to the matching GitHub Release

## Contributing

Issues and pull requests are welcome.

If you plan to contribute code, please keep these design constraints in mind:

- `machtop` should remain useful on generic Linux machines, not only RK3588
- hardware-specific metrics must degrade cleanly when unsupported
- new telemetry sources should be explicit about permissions and ABI stability

## Status

`machtop` is usable today, but it is still a pre-1.0 project. The UI, platform
heuristics, and release process will continue to evolve as more hardware is
tested.

## License

Released under the GNU GPLv3 or later. See [LICENSE](LICENSE).
