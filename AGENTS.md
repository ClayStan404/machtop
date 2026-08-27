# Repository Guidelines

## Project Structure & Module Organization

`machtop` is a Linux terminal system monitor written in Rust 2024. `src/main.rs` owns terminal setup and the event loop; `src/app.rs` manages refresh timing and application state; `src/ui.rs` contains Ratatui layout, with formatting and theme helpers under `src/ui/`. Metric models, `/proc` and `/sys` readers, derivation, and board-specific profiles live under `src/metrics/`. Keep documentation images in `docs/images/`. GitHub Actions workflows are in `.github/workflows/`; `target/` is generated and must not be committed.

## Build, Test, and Development Commands

- `cargo run` launches the TUI in an interactive terminal.
- `cargo build --release --locked` creates `target/release/machtop` using the committed lockfile.
- `cargo fmt --check` verifies Rust formatting without changing files.
- `cargo test --locked` runs the complete unit-test suite.
- `cargo clippy --all-targets --locked -- -D warnings` applies the same lint policy as CI.

Run formatting, tests, and Clippy before opening a pull request. CI repeats these checks on amd64 and arm64 Linux.

## Coding Style & Naming Conventions

Use standard `rustfmt` output and four-space indentation. Follow Rust naming conventions: `snake_case` for modules, functions, and tests; `PascalCase` for structs and enums; `SCREAMING_SNAKE_CASE` for constants. Keep collectors separate from derived metrics and presentation logic. Hardware-specific telemetry must remain capability-based: return `Option` or skip unavailable values instead of making the application fail. Add context to fallible operations with `anyhow` where it improves diagnostics.

## Testing Guidelines

Place unit tests beside the code in `#[cfg(test)] mod tests`. Name tests after observable behavior, such as `parses_proc_stat` or `reads_hwmon_sensors_on_generic_linux`. Use `tempfile::tempdir` to model procfs/sysfs trees without touching host state. Cover parsing, missing data, permission-sensitive fallbacks, and platform profiles. No coverage threshold is configured, so review should focus on meaningful regression cases.

## Commit & Pull Request Guidelines

Recent history commonly uses concise Conventional Commit prefixes such as `feat:` and `fix:`; follow that pattern with an imperative summary. Keep commits focused. Pull requests should explain the behavior change, list tested architectures or hardware, link relevant issues, and include a terminal screenshot for visible UI changes. Confirm all CI commands pass and document any new privileged or unstable kernel interface.
