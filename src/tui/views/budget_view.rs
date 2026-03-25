use ratatui::{
    layout::Rect,
    style::Style,
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::tui::state::AppState;

pub fn render_budget_view(f: &mut Frame, area: Rect, state: &AppState) {
    let mut lines = vec![
        format!(
            "Global Budget: {:.0}%",
            state.budget.global_percent_used * 100.0
        ),
        format!("Mode: {:?}", state.budget.mode),
    ];

    for (backend, percent) in &state.budget.backend_percent_used {
        lines.push(format!("{}: {:.0}%", backend, percent * 100.0));
    }

    if let Some(rate) = &state.budget.daily_burn_rate {
        lines.push(format!("Daily burn: {}", rate));
    }

    if let Some(projected) = &state.budget.projected_eom {
        lines.push(format!("Projected EOM: {}", projected));
    }

    let text = lines.join("\n");
    let paragraph =
        Paragraph::new(text).block(Block::default().title("Budget").borders(Borders::ALL));
    f.render_widget(paragraph, area);
}
