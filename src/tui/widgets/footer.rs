use std::cmp::Ordering;

use ratatui::{layout::Rect, style::Style, widgets::Paragraph, Frame};

use crate::tui::state::AppState;

pub fn render_footer(f: &mut Frame, area: Rect, state: &AppState) {
    let mut parts = Vec::new();

    let mut backends: Vec<_> = state.budget.backend_percent_used.iter().collect();
    backends.sort_by(
        |a, b| match (a.1.partial_cmp(b.1), a.0.as_str().cmp(b.0.as_str())) {
            (Some(Ordering::Equal), name_order) => name_order,
            (Some(order), _) => order.reverse(),
            (None, name_order) => name_order,
        },
    );

    for (backend, percent) in backends {
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
