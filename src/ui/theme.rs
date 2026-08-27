use ratatui::style::{Color, Modifier, Style};

pub(super) fn bar_style(percent: f64) -> Style {
    let color = if percent >= 85.0 {
        Color::LightRed
    } else if percent >= 60.0 {
        Color::Yellow
    } else {
        Color::LightGreen
    };
    Style::default().fg(color)
}

pub(super) fn cpu_usage_style(_percent: f64) -> Style {
    accent_value_style()
}

pub(super) fn temperature_style(_value: Option<f64>) -> Style {
    accent_value_style()
}

pub(super) fn memory_style(percent: f64) -> Style {
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

pub(super) fn state_style(state: char) -> Style {
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

pub(super) fn title_style() -> Style {
    Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD)
}

pub(super) fn overall_label_style() -> Style {
    Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD)
}

pub(super) fn overall_value_style() -> Style {
    Style::default()
        .fg(Color::LightGreen)
        .add_modifier(Modifier::BOLD)
}

pub(super) fn border_style() -> Style {
    Style::default().fg(Color::DarkGray)
}

pub(super) fn header_style() -> Style {
    Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD)
}

pub(super) fn label_style() -> Style {
    Style::default()
        .fg(Color::LightGreen)
        .add_modifier(Modifier::BOLD)
}

pub(super) fn value_style() -> Style {
    Style::default().fg(Color::White)
}

pub(super) fn accent_value_style() -> Style {
    Style::default().fg(Color::LightBlue)
}

pub(super) fn io_rate_style(bytes_per_sec: f64) -> Style {
    if bytes_per_sec <= f64::EPSILON {
        Style::default().fg(Color::Gray)
    } else {
        accent_value_style()
    }
}

pub(super) fn tag_style() -> Style {
    label_style()
}
