use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use crate::tui::state::AppState;

pub fn render_budget_view(f: &mut Frame, area: Rect, state: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(6),
            Constraint::Length(8),
            Constraint::Min(10),
        ])
        .split(area);

    render_global_budget(f, chunks[0], state);
    render_backend_budget(f, chunks[1], state);
    render_project_budget(f, chunks[2], state);
}

fn render_global_budget(f: &mut Frame, area: Rect, state: &AppState) {
    let percent = state.budget.global_percent_used;
    let bar = progress_bar(percent, 30);

    let mode = format!("{:?}", state.budget.mode);
    let warning_count = state.budget.warnings.len();

    let mut lines = vec![
        format!("Mode: {}  Warnings: {}", mode, warning_count),
        format!("Global Budget: {:.0}%", percent * 100.0),
        format!("[{}]", bar),
    ];

    if let Some(rate) = &state.budget.daily_burn_rate {
        lines.push(format!(
            "Daily burn rate: ${:.2}",
            rate.cents as f64 / 100.0
        ));
    }

    if let Some(projected) = &state.budget.projected_eom {
        lines.push(format!(
            "Projected EOM: ${:.2}",
            projected.cents as f64 / 100.0
        ));
    }

    let text = lines.join("\n");
    let paragraph = Paragraph::new(text)
        .block(
            Block::default()
                .title("Global Budget")
                .borders(Borders::ALL),
        )
        .wrap(Wrap::default());
    f.render_widget(paragraph, area);
}

fn render_backend_budget(f: &mut Frame, area: Rect, state: &AppState) {
    let mut lines = Vec::new();

    if state.budget.backend_percent_used.is_empty() {
        lines.push("No backend data available".to_string());
    } else {
        lines.push("Backend Usage:".to_string());
        for (backend, percent) in &state.budget.backend_percent_used {
            let bar = progress_bar(*percent, 20);
            lines.push(format!("  {}: [{}] {:.0}%", backend, bar, percent * 100.0));
        }
    }

    let text = lines.join("\n");
    let paragraph = Paragraph::new(text)
        .block(
            Block::default()
                .title("Backend Budget")
                .borders(Borders::ALL),
        )
        .wrap(Wrap::default());
    f.render_widget(paragraph, area);
}

fn render_project_budget(f: &mut Frame, area: Rect, state: &AppState) {
    let mut lines = vec!["Project Budget Usage:".to_string()];

    let mut sorted: Vec<_> = state.projects.iter().collect();
    sorted.sort_by(|a, b| {
        b.budget_percent
            .partial_cmp(&a.budget_percent)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let top_projects: Vec<_> = sorted.iter().take(10).collect();

    if top_projects.is_empty() {
        lines.push("  (no projects)".to_string());
    } else {
        for p in top_projects {
            let bar = progress_bar(p.budget_percent, 15);
            lines.push(format!(
                "  {}: [{}] {:.0}%",
                p.name,
                bar,
                p.budget_percent * 100.0
            ));
        }
    }

    let text = lines.join("\n");
    let paragraph = Paragraph::new(text)
        .block(
            Block::default()
                .title("Project Budget")
                .borders(Borders::ALL),
        )
        .wrap(Wrap::default());
    f.render_widget(paragraph, area);
}

fn progress_bar(percent: f32, width: usize) -> String {
    let filled = ((percent.clamp(0.0, 1.0) * width as f32) as usize).min(width);
    let empty = width - filled;
    format!("{}{}", "█".repeat(filled), "░".repeat(empty))
}
