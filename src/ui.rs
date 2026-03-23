use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};

use crate::app::App;
use crate::metrics::{
    CpuMetrics, DiskIoEntry, NetworkIoEntry, ProcessEntry, SensorSummary, SystemSnapshot,
};

struct ViewModel {
    kernel: String,
    machine: String,
    arch: String,
    uptime: String,
    memory_rows: Vec<MemoryMeterView>,
    accelerators: Option<String>,
    sensors: Option<String>,
}

struct MemoryMeterView {
    label: &'static str,
    percent: f64,
    size_text: String,
}

struct CpuLayoutSpec {
    columns: usize,
    id_width: usize,
    fixed_width: usize,
    min_bar_width: usize,
    max_bar_width: usize,
    bar_width: usize,
    column_gap: u16,
}

pub fn render(frame: &mut Frame, app: &App) {
    let snapshot = app.snapshot();
    let view = build_view_model(snapshot);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(summary_panel_height(&view)),
            Constraint::Min(0),
        ])
        .split(frame.area());

    render_summary(frame, chunks[0], &view);

    let cpu_height = cpu_panel_height(chunks[1].width, &snapshot.cpus);
    let body = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(cpu_height), Constraint::Min(10)])
        .split(chunks[1]);
    let body_columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(body[1]);

    render_cpu_panel(frame, body[0], snapshot);
    render_process_table(frame, body_columns[0], snapshot);
    render_io_panel(frame, body_columns[1], snapshot);
}

fn build_view_model(snapshot: &SystemSnapshot) -> ViewModel {
    ViewModel {
        kernel: snapshot.machine.kernel.clone(),
        machine: snapshot.machine.machine_name.clone(),
        arch: snapshot.machine.arch.clone(),
        uptime: format_uptime(snapshot.uptime),
        memory_rows: build_memory_rows(&snapshot.memory),
        accelerators: format_accelerator_summary(snapshot),
        sensors: format_sensor_summary(&snapshot.sensor_summary),
    }
}

fn summary_panel_height(view: &ViewModel) -> u16 {
    let left_rows = 4;
    let right_rows = view.memory_rows.len().max(1);

    left_rows.max(right_rows) as u16 + 2
}

fn render_summary(frame: &mut Frame, area: Rect, view: &ViewModel) {
    let block = Block::default()
        .title(Span::styled("Summary", title_style()))
        .borders(Borders::ALL)
        .border_style(border_style());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let sections = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(inner);
    let right_block = Block::default()
        .borders(Borders::LEFT)
        .border_style(border_style());
    let right_inner = right_block.inner(sections[1]);
    frame.render_widget(right_block, sections[1]);

    let left_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(sections[0]);

    let pair_rows = [
        (
            ("kernel", view.kernel.as_str(), accent_value_style()),
            ("machine", view.machine.as_str(), accent_value_style()),
        ),
        (
            ("arch", view.arch.as_str(), accent_value_style()),
            ("uptime", view.uptime.as_str(), accent_value_style()),
        ),
    ];

    for (row_index, (left, right)) in pair_rows.iter().enumerate() {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(left_rows[row_index]);
        render_summary_cell(frame, cols[0], left.0, left.1, left.2);
        render_summary_cell(frame, cols[1], right.0, right.1, right.2);
    }

    if let Some(accelerators) = view.accelerators.as_deref() {
        render_summary_cell(
            frame,
            left_rows[2],
            "accel",
            accelerators,
            accent_value_style(),
        );
    } else {
        render_summary_cell(frame, left_rows[2], "", "", Style::default());
    }

    if let Some(sensors) = view.sensors.as_deref() {
        render_summary_cell(
            frame,
            left_rows[3],
            "sensors",
            sensors,
            accent_value_style(),
        );
    } else {
        render_summary_cell(frame, left_rows[3], "", "", Style::default());
    }

    if right_inner.width == 0 || right_inner.height == 0 {
        return;
    }

    let right_row_count = view.memory_rows.len().max(1);
    let right_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![Constraint::Length(1); right_row_count])
        .split(right_inner);

    if right_row_count == 1 && view.memory_rows.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                "No memory data",
                Style::default().fg(Color::Gray),
            )])),
            right_rows[0],
        );
    } else {
        for (index, memory_row) in view.memory_rows.iter().enumerate() {
            render_memory_line(frame, right_rows[index], memory_row);
        }
    }
}

fn render_summary_cell(
    frame: &mut Frame,
    area: Rect,
    label: &str,
    value: &str,
    value_style: Style,
) {
    if area.width == 0 {
        return;
    }

    if label.is_empty() && value.is_empty() {
        frame.render_widget(Paragraph::new(""), area);
        return;
    }

    let label_width = 11usize.min(area.width.saturating_sub(1) as usize);
    let value_width = area.width.saturating_sub(label_width as u16 + 1) as usize;
    let line = Line::from(vec![
        Span::styled(format!("{label:<label_width$}"), label_style()),
        Span::raw(" "),
        Span::styled(truncate_text(value, value_width), value_style),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn render_overall_line(frame: &mut Frame, area: Rect, percent: f64) {
    if area.width == 0 {
        return;
    }

    let percent_text = format!("{:>5.1}%", percent);
    let min_bar_width = 12usize;
    let max_bar_width = 28usize;
    let static_width = "overall".len() + 2 + percent_text.len() + 2 + 2;
    let bar_width = (area.width.saturating_sub(static_width as u16) as usize)
        .clamp(min_bar_width, max_bar_width);

    let mut spans = vec![
        Span::styled("overall", overall_label_style()),
        Span::raw("  "),
        Span::styled(percent_text, overall_value_style()),
        Span::raw("  "),
        Span::styled("[", Style::default().fg(Color::DarkGray)),
    ];
    spans.extend(meter_spans(percent, bar_width));
    spans.push(Span::styled("]", Style::default().fg(Color::DarkGray)));

    let content_width = spans
        .iter()
        .map(|span| span.content.chars().count())
        .sum::<usize>();
    let left_padding = area.width.saturating_sub(content_width as u16) as usize / 2;
    let mut line = Vec::with_capacity(spans.len() + usize::from(left_padding > 0));
    if left_padding > 0 {
        line.push(Span::raw(" ".repeat(left_padding)));
    }
    line.extend(spans);

    frame.render_widget(Paragraph::new(Line::from(line)), area);
}

fn render_memory_line(frame: &mut Frame, area: Rect, memory: &MemoryMeterView) {
    if area.width == 0 {
        return;
    }

    let label_width = 6usize.min(area.width.saturating_sub(1) as usize);
    let size_width = memory.size_text.len().min(area.width as usize);
    let meter_width = area.width.saturating_sub(label_width as u16 + 1) as usize;
    let bar_width = meter_width
        .saturating_sub(9 + usize::from(size_width > 0) + size_width)
        .clamp(4, 16);
    let visible_size_width = meter_width.saturating_sub(bar_width + 9 + 1);
    let size_text = truncate_text(&memory.size_text, visible_size_width);

    let mut line = vec![
        Span::styled(format!("{:<label_width$}", memory.label), label_style()),
        Span::raw(" "),
    ];
    line.push(Span::styled("[", Style::default().fg(Color::DarkGray)));
    line.extend(meter_spans(memory.percent, bar_width));
    line.push(Span::styled("]", Style::default().fg(Color::DarkGray)));
    line.push(Span::raw(" "));
    line.push(Span::styled(
        format!("{:>5.1}%", memory.percent),
        memory_style(memory.percent),
    ));
    if !size_text.is_empty() {
        line.push(Span::raw(" "));
        line.push(Span::styled(size_text, accent_value_style()));
    }

    frame.render_widget(Paragraph::new(Line::from(line)), area);
}

fn build_memory_rows(memory: &crate::metrics::MemoryMetrics) -> Vec<MemoryMeterView> {
    let mut rows = vec![MemoryMeterView {
        label: "ram",
        percent: memory.ram.used_percent,
        size_text: format!(
            "[{}/{}]",
            format_bytes(memory.ram.used_bytes),
            format_bytes(memory.ram.total_bytes)
        ),
    }];

    if memory.zram.total_bytes > 0 {
        rows.push(MemoryMeterView {
            label: "zram",
            percent: memory.zram.used_percent,
            size_text: format!(
                "[{}/{}]",
                format_bytes(memory.zram.used_bytes),
                format_bytes(memory.zram.total_bytes)
            ),
        });
    }
    if memory.swap.total_bytes > 0 {
        rows.push(MemoryMeterView {
            label: "swap",
            percent: memory.swap.used_percent,
            size_text: format!(
                "[{}/{}]",
                format_bytes(memory.swap.used_bytes),
                format_bytes(memory.swap.total_bytes)
            ),
        });
    }

    rows
}

fn truncate_text(text: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    if text.chars().count() <= max_width {
        return text.to_string();
    }
    if max_width <= 3 {
        return text.chars().take(max_width).collect();
    }

    let head: String = text.chars().take(max_width - 3).collect();
    format!("{head}...")
}

fn render_cpu_panel(frame: &mut Frame, area: Rect, snapshot: &SystemSnapshot) {
    let block = Block::default()
        .title(Span::styled("CPU", title_style()))
        .borders(Borders::ALL)
        .border_style(border_style());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(inner);
    render_overall_line(frame, sections[0], snapshot.overall_usage_percent);

    if sections[1].width == 0 || sections[1].height == 0 {
        return;
    }

    let layout = cpu_layout(sections[1].width, &snapshot.cpus);
    let mut constraints = Vec::with_capacity(layout.columns * 2 - 1);
    for index in 0..layout.columns {
        constraints.push(Constraint::Ratio(1, layout.columns as u32));
        if index + 1 != layout.columns {
            constraints.push(Constraint::Length(layout.column_gap));
        }
    }
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(sections[1]);

    let rows_per_column = snapshot.cpus.len().div_ceil(layout.columns);
    for (column_index, chunk) in chunks.iter().step_by(2).enumerate() {
        let start = column_index * rows_per_column;
        let end = usize::min(start + rows_per_column, snapshot.cpus.len());
        if start >= end {
            continue;
        }

        let column_layout = CpuLayoutSpec {
            bar_width: cpu_bar_width(chunk.width as usize, &layout),
            ..layout
        };
        let lines = snapshot.cpus[start..end]
            .iter()
            .map(|cpu| cpu_line(cpu, &column_layout))
            .collect::<Vec<_>>();
        frame.render_widget(Paragraph::new(lines), *chunk);
    }
}

fn cpu_layout(width: u16, cpus: &[CpuMetrics]) -> CpuLayoutSpec {
    let id_width = cpus
        .iter()
        .map(|cpu| cpu.id.to_string().len())
        .max()
        .unwrap_or(1)
        .max(2);
    let has_temperature = cpus.iter().any(|cpu| cpu.temperature_c.is_some());
    let fixed_width = cpu_fixed_width(id_width, cpus);
    let min_bar_width = 12;
    let max_bar_width = if has_temperature { 18 } else { 22 };
    let column_padding = 3;
    let column_gap = 4;
    let max_columns = cpus.len().clamp(1, 3);
    let mut columns = 1;

    for candidate in 1..=max_columns {
        let available_width =
            (width as usize).saturating_sub(column_gap * candidate.saturating_sub(1));
        let column_width = available_width / candidate;
        if column_width >= fixed_width + min_bar_width + column_padding {
            columns = candidate;
        } else {
            break;
        }
    }

    CpuLayoutSpec {
        columns,
        id_width,
        fixed_width,
        min_bar_width,
        max_bar_width,
        bar_width: min_bar_width,
        column_gap: column_gap as u16,
    }
}

fn cpu_fixed_width(id_width: usize, cpus: &[CpuMetrics]) -> usize {
    let usage_width = format_cpu_usage(100.0).len();
    let frequency_width = cpus
        .iter()
        .map(|cpu| format_frequency(cpu.frequency_khz).len())
        .max()
        .unwrap_or_else(|| format_frequency(None).len());
    let temperature_width = cpus
        .iter()
        .filter_map(|cpu| cpu.temperature_c)
        .map(format_temperature)
        .map(|text| text.len())
        .max()
        .unwrap_or(0);

    id_width
        + 1
        + 1
        + 1
        + 1
        + usage_width
        + 2
        + frequency_width
        + if temperature_width > 0 {
            1 + temperature_width
        } else {
            0
        }
}

fn cpu_panel_height(width: u16, cpus: &[CpuMetrics]) -> u16 {
    let layout = cpu_layout(width, cpus);
    let rows = cpus.len().div_ceil(layout.columns) as u16;
    rows + 3
}

fn cpu_bar_width(column_width: usize, layout: &CpuLayoutSpec) -> usize {
    column_width
        .saturating_sub(layout.fixed_width + 1)
        .clamp(layout.min_bar_width, layout.max_bar_width)
}

fn cpu_line(cpu: &CpuMetrics, layout: &CpuLayoutSpec) -> Line<'static> {
    let usage_style = cpu_usage_style(cpu.usage_percent);
    let mut spans = Vec::new();
    spans.push(Span::styled(
        format!("{:>width$}", cpu.id, width = layout.id_width),
        label_style(),
    ));
    spans.push(Span::raw(" "));
    spans.push(Span::styled("[", Style::default().fg(Color::DarkGray)));
    spans.extend(meter_spans(cpu.usage_percent, layout.bar_width));
    spans.push(Span::styled("]", Style::default().fg(Color::DarkGray)));
    spans.push(Span::raw("  "));
    spans.push(Span::styled(
        format_frequency(cpu.frequency_khz),
        accent_value_style(),
    ));
    spans.push(Span::raw(" "));
    spans.push(Span::styled(
        format_cpu_usage(cpu.usage_percent),
        usage_style,
    ));
    if let Some(temperature_c) = cpu.temperature_c {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            format_temperature(temperature_c),
            temperature_style(Some(temperature_c)),
        ));
    }

    Line::from(spans)
}

fn render_process_table(frame: &mut Frame, area: Rect, snapshot: &SystemSnapshot) {
    let compact = area.width < 96;
    let max_rows = area.height.saturating_sub(3) as usize;
    let rows = snapshot
        .process_list
        .iter()
        .take(max_rows)
        .map(|process| render_process_row(process, compact))
        .collect::<Vec<Row>>();

    let (widths, header) = if compact {
        (
            vec![
                Constraint::Length(6),
                Constraint::Length(2),
                Constraint::Length(6),
                Constraint::Length(6),
                Constraint::Length(7),
                Constraint::Min(12),
            ],
            Row::new([
                Cell::from("PID"),
                Cell::from("S"),
                Cell::from("CPU%"),
                Cell::from("MEM%"),
                Cell::from("RSS"),
                Cell::from("COMMAND"),
            ]),
        )
    } else {
        (
            vec![
                Constraint::Length(6),
                Constraint::Length(10),
                Constraint::Length(2),
                Constraint::Length(6),
                Constraint::Length(6),
                Constraint::Length(7),
                Constraint::Min(16),
            ],
            Row::new([
                Cell::from("PID"),
                Cell::from("USER"),
                Cell::from("S"),
                Cell::from("CPU%"),
                Cell::from("MEM%"),
                Cell::from("RSS"),
                Cell::from("COMMAND"),
            ]),
        )
    };

    let table = Table::new(rows, widths)
        .header(header.style(header_style()))
        .block(
            Block::default()
                .title(Span::styled("Processes", title_style()))
                .borders(Borders::ALL)
                .border_style(border_style()),
        )
        .column_spacing(1);

    frame.render_widget(table, area);
}

fn render_io_panel(frame: &mut Frame, area: Rect, snapshot: &SystemSnapshot) {
    let block = Block::default()
        .title(Span::styled("Network & Disk I/O", title_style()))
        .borders(Borders::ALL)
        .border_style(border_style());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(inner);

    render_network_table(frame, sections[0], &snapshot.network_io);
    render_disk_table(frame, sections[1], &snapshot.disk_io);
}

fn render_network_table(frame: &mut Frame, area: Rect, entries: &[NetworkIoEntry]) {
    render_io_section(
        frame,
        area,
        "Network",
        entries.is_empty(),
        "No active interfaces",
        || {
            Table::new(
                entries
                    .iter()
                    .map(|entry| {
                        Row::new([
                            Cell::from(Span::styled(entry.name.clone(), tag_style())),
                            Cell::from(Span::styled(
                                format_rate(entry.rx_bytes_per_sec),
                                io_rate_style(entry.rx_bytes_per_sec),
                            )),
                            Cell::from(Span::styled(
                                format_rate(entry.tx_bytes_per_sec),
                                io_rate_style(entry.tx_bytes_per_sec),
                            )),
                        ])
                    })
                    .collect::<Vec<_>>(),
                [
                    Constraint::Min(6),
                    Constraint::Length(8),
                    Constraint::Length(8),
                ],
            )
            .header(
                Row::new([Cell::from("IFACE"), Cell::from("RX/s"), Cell::from("TX/s")])
                    .style(header_style()),
            )
            .column_spacing(1)
        },
    );
}

fn render_disk_table(frame: &mut Frame, area: Rect, entries: &[DiskIoEntry]) {
    render_io_section(
        frame,
        area,
        "Disk",
        entries.is_empty(),
        "No active disks",
        || {
            Table::new(
                entries
                    .iter()
                    .map(|entry| {
                        Row::new([
                            Cell::from(Span::styled(entry.name.clone(), tag_style())),
                            Cell::from(Span::styled(
                                format_rate(entry.read_bytes_per_sec),
                                io_rate_style(entry.read_bytes_per_sec),
                            )),
                            Cell::from(Span::styled(
                                format_rate(entry.write_bytes_per_sec),
                                io_rate_style(entry.write_bytes_per_sec),
                            )),
                        ])
                    })
                    .collect::<Vec<_>>(),
                [
                    Constraint::Min(6),
                    Constraint::Length(8),
                    Constraint::Length(8),
                ],
            )
            .header(
                Row::new([
                    Cell::from("DEVICE"),
                    Cell::from("READ/s"),
                    Cell::from("WRITE/s"),
                ])
                .style(header_style()),
            )
            .column_spacing(1)
        },
    );
}

fn render_io_section<F>(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    is_empty: bool,
    empty_message: &str,
    build_table: F,
) where
    F: FnOnce() -> Table<'static>,
{
    if area.width == 0 || area.height == 0 {
        return;
    }

    let section = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(area);
    frame.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(title, label_style())])),
        section[0],
    );

    if is_empty {
        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                empty_message,
                Style::default().fg(Color::Gray),
            )])),
            section[1],
        );
    } else {
        frame.render_widget(build_table(), section[1]);
    }
}

fn render_process_row(process: &ProcessEntry, compact: bool) -> Row<'static> {
    let cpu_style = bar_style(process.cpu_percent);
    let mem_style = memory_style(process.mem_percent);
    let mut cells = vec![
        Cell::from(Span::styled(format!("{}", process.pid), value_style())),
        Cell::from(Span::styled(
            process.state.to_string(),
            state_style(process.state),
        )),
        Cell::from(Span::styled(
            format!("{:>4.1}", process.cpu_percent),
            cpu_style,
        )),
        Cell::from(Span::styled(
            format!("{:>4.1}", process.mem_percent),
            mem_style,
        )),
        Cell::from(Span::styled(
            format_bytes(process.rss_bytes),
            accent_value_style(),
        )),
    ];

    if compact {
        cells.push(Cell::from(Span::styled(
            process.command.clone(),
            accent_value_style(),
        )));
    } else {
        cells.insert(
            1,
            Cell::from(Span::styled(process.user.clone(), tag_style())),
        );
        cells.push(Cell::from(Span::styled(
            process.command.clone(),
            accent_value_style(),
        )));
    }

    Row::new(cells)
}

fn meter_spans(percent: f64, width: usize) -> Vec<Span<'static>> {
    let clamped = percent.clamp(0.0, 100.0);
    let filled = ((clamped / 100.0) * width as f64).round() as usize;
    let filled = filled.min(width);

    let green_cap = ((width as f64) * 0.50).round() as usize;
    let yellow_cap = ((width as f64) * 0.80).round() as usize;

    let green = filled.min(green_cap);
    let yellow = filled.min(yellow_cap).saturating_sub(green);
    let red = filled.saturating_sub(green + yellow);
    let empty = width.saturating_sub(filled);

    let mut spans = Vec::with_capacity(4);
    if green > 0 {
        spans.push(Span::styled(
            "|".repeat(green),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ));
    }
    if yellow > 0 {
        spans.push(Span::styled(
            "|".repeat(yellow),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
    }
    if red > 0 {
        spans.push(Span::styled(
            "|".repeat(red),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ));
    }
    if empty > 0 {
        spans.push(Span::styled(
            ".".repeat(empty),
            Style::default().fg(Color::DarkGray),
        ));
    }
    spans
}

fn format_frequency(frequency_khz: Option<u64>) -> String {
    match frequency_khz {
        Some(value) => format!("[{:.1}G]", value as f64 / 1_000_000.0),
        None => "[ n/a]".to_string(),
    }
}

fn format_cpu_usage(percent: f64) -> String {
    format!("{:>5.1}%", percent)
}

fn format_temperature(value: f64) -> String {
    format!("[{:>4.1}C]", value)
}

fn format_uptime(uptime: std::time::Duration) -> String {
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

fn format_bytes(bytes: u64) -> String {
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

fn format_rate(bytes_per_sec: f64) -> String {
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

fn format_sensor_summary(summary: &[SensorSummary]) -> Option<String> {
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

fn format_accelerator_summary(snapshot: &SystemSnapshot) -> Option<String> {
    let mut sections = Vec::new();

    if let Some(gpu) = snapshot.accelerators.gpu.as_ref() {
        sections.push(format_gpu_summary(gpu.usage_percent, gpu.frequency_hz));
    }
    if let Some(npu) = snapshot.accelerators.npu.as_ref() {
        sections.push(format_npu_summary(
            npu.usage_percent,
            &npu.per_core_usage_percent,
            npu.frequency_hz,
        ));
    }

    (!sections.is_empty()).then(|| sections.join("  "))
}

fn format_gpu_summary(usage_percent: Option<f64>, frequency_hz: Option<u64>) -> String {
    format_accelerator_section("gpu", usage_percent, &[], frequency_hz)
}

fn format_npu_summary(
    usage_percent: Option<f64>,
    per_core_usage_percent: &[f64],
    frequency_hz: Option<u64>,
) -> String {
    format_accelerator_section("npu", usage_percent, per_core_usage_percent, frequency_hz)
}

fn format_accelerator_section(
    label: &str,
    usage_percent: Option<f64>,
    per_core_usage_percent: &[f64],
    frequency_hz: Option<u64>,
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

fn bar_style(percent: f64) -> Style {
    let color = if percent >= 85.0 {
        Color::LightRed
    } else if percent >= 60.0 {
        Color::Yellow
    } else {
        Color::LightGreen
    };
    Style::default().fg(color)
}

fn cpu_usage_style(_percent: f64) -> Style {
    accent_value_style()
}

fn temperature_style(_value: Option<f64>) -> Style {
    accent_value_style()
}

fn memory_style(percent: f64) -> Style {
    if percent >= 85.0 {
        Style::default()
            .fg(Color::LightRed)
            .add_modifier(Modifier::BOLD)
    } else if percent >= 65.0 {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else {
        accent_value_style()
    }
}

fn state_style(state: char) -> Style {
    match state {
        'R' => Style::default()
            .fg(Color::LightGreen)
            .add_modifier(Modifier::BOLD),
        'D' => Style::default()
            .fg(Color::LightRed)
            .add_modifier(Modifier::BOLD),
        'S' => Style::default().fg(Color::Gray),
        'Z' => Style::default()
            .fg(Color::LightRed)
            .add_modifier(Modifier::BOLD),
        _ => Style::default().fg(Color::White),
    }
}

fn title_style() -> Style {
    Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD)
}

fn overall_label_style() -> Style {
    Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD)
}

fn overall_value_style() -> Style {
    Style::default()
        .fg(Color::LightGreen)
        .add_modifier(Modifier::BOLD)
}

fn border_style() -> Style {
    Style::default().fg(Color::DarkGray)
}

fn header_style() -> Style {
    Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD)
}

fn label_style() -> Style {
    Style::default()
        .fg(Color::LightGreen)
        .add_modifier(Modifier::BOLD)
}

fn value_style() -> Style {
    Style::default().fg(Color::White)
}

fn accent_value_style() -> Style {
    Style::default().fg(Color::LightBlue)
}

fn io_rate_style(bytes_per_sec: f64) -> Style {
    if bytes_per_sec <= f64::EPSILON {
        Style::default().fg(Color::Gray)
    } else {
        accent_value_style()
    }
}

fn tag_style() -> Style {
    label_style()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_bytes_adaptively() {
        assert_eq!(format_bytes(512 * 1024), "512K");
        assert_eq!(format_bytes(128 * 1024 * 1024), "128M");
        assert_eq!(format_bytes(3 * 1024 * 1024 * 1024), "3.0G");
    }

    #[test]
    fn formats_rates_compactly() {
        assert_eq!(format_rate(0.0), "0B/s");
        assert_eq!(format_rate(834.0 * 1024.0), "834K/s");
        assert_eq!(format_rate(12.4 * 1024.0 * 1024.0), "12.4M/s");
        assert_eq!(format_rate(1.2 * 1024.0 * 1024.0 * 1024.0), "1.2G/s");
    }

    #[test]
    fn builds_memory_rows_in_expected_order() {
        let rows = build_memory_rows(&crate::metrics::MemoryMetrics {
            ram: crate::metrics::UsageMetric::from_total_and_used(8, 4),
            zram: crate::metrics::UsageMetric::from_total_and_used(4, 1),
            swap: crate::metrics::UsageMetric::from_total_and_used(16, 2),
        });

        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].label, "ram");
        assert_eq!(rows[0].size_text, "[0K/0K]");
        assert_eq!(rows[1].label, "zram");
        assert_eq!(rows[2].label, "swap");
    }

    #[test]
    fn cpu_layout_uses_more_columns_on_wider_terminal() {
        let cpus = (0..16)
            .map(|id| CpuMetrics {
                id,
                usage_percent: 0.0,
                frequency_khz: Some(3_600_000),
                temperature_c: None,
            })
            .collect::<Vec<_>>();

        let compact_layout = cpu_layout(78, &cpus);
        let medium_layout = cpu_layout(96, &cpus);
        let wide_layout = cpu_layout(120, &cpus);
        let very_wide_layout = cpu_layout(160, &cpus);

        assert_eq!(compact_layout.columns, 2);
        assert_eq!(medium_layout.columns, 2);
        assert_eq!(wide_layout.columns, 3);
        assert_eq!(very_wide_layout.columns, 3);
    }

    #[test]
    fn formats_accelerator_sections_compactly() {
        assert_eq!(
            format_gpu_summary(Some(7.0), Some(300_000_000)),
            "gpu 7% [300M]"
        );
        assert_eq!(
            format_npu_summary(Some(4.0), &[12.0, 8.0, 0.0], Some(1_000_000_000)),
            "npu 12/8/0% [1.0G]"
        );
    }

    #[test]
    fn formats_sensor_summary_with_bracketed_temperatures() {
        let summary = [SensorSummary {
            label: "cpu".to_string(),
            temperature_c: 52.3,
        }];

        assert_eq!(
            format_sensor_summary(&summary),
            Some("cpu [52.3C]".to_string())
        );
    }
}
