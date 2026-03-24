use ratatui::{layout::Rect, style::Style, widgets::Paragraph, Frame};

use crate::tui::state::AppState;

pub fn render_footer(f: &mut Frame, area: Rect, state: &AppState) {
    let mut parts = Vec::new();

    for (backend, percent) in &state.budget.backend_percent_used {
        parts.push(format!("{} {:.0}%", backend, percent * 100.0));
    }

    parts.push(format!(
        "Global {:.0}%",
        state.budget.global_percent_used * 100.0
    ));
    parts.push(format!("Mode {:?}", state.budget.mode));
    parts.push(format!("Pending {}", state.actions.len()));

    let text = parts.join(" | ");

    let paragraph = Paragraph::new(text).style(Style::default());

    f.render_widget(paragraph, area);
}
