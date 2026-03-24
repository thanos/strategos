use ratatui::{layout::Rect, Frame};

use crate::tui::state::AppState;

pub fn render_projects_view(f: &mut Frame, area: Rect, _state: &AppState) {
    let block = ratatui::widgets::Block::default()
        .title("Projects")
        .borders(ratatui::widgets::Borders::ALL);
    f.render_widget(block, area);
}
