use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::tui::state::AppState;

pub fn render_composer(f: &mut Frame, area: Rect, state: &AppState) {
    let is_focused = state.focused == crate::tui::types::FocusRegion::Composer;
    let is_input_mode = state.mode == crate::tui::types::UiMode::Input;

    let style = if is_focused || is_input_mode {
        Style::default()
    } else {
        Style::default().add_modifier(Modifier::DIM)
    };

    let input = &state.composer.input;
    let display = if input.is_empty() {
        "Type a message..."
    } else {
        input
    };

    let block = Block::default()
        .title("Composer")
        .borders(Borders::ALL)
        .border_style(style);

    let paragraph = Paragraph::new(display)
        .style(if input.is_empty() {
            Style::default().add_modifier(Modifier::DIM)
        } else {
            style
        })
        .block(block);

    f.render_widget(paragraph, area);
}
