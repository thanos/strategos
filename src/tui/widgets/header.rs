use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    widgets::Paragraph,
    Frame,
};

use crate::tui::state::AppState;

pub fn render_header(f: &mut Frame, area: Rect, state: &AppState) {
    let project_ctx = state
        .chats_view
        .project_scope
        .as_ref()
        .and_then(|id| state.projects.iter().find(|p| p.id == *id))
        .map(|p| p.name.as_str())
        .unwrap_or("all");

    let text = format!(
        "Strategos | {} | Mode: {} | Project: {}",
        state.current_tab.as_str(),
        state.mode.as_str(),
        project_ctx
    );

    let paragraph = Paragraph::new(text).style(Style::default().add_modifier(Modifier::BOLD));

    f.render_widget(paragraph, area);
}
