use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::tui::state::AppState;

pub fn render_events_view(f: &mut Frame, area: Rect, state: &AppState) {
    let filtered: Vec<_> = if let Some(filter) = &state.events_view.filter {
        state
            .events
            .iter()
            .filter(|e| e.event_type.contains(filter))
            .collect()
    } else {
        state.events.iter().collect()
    };

    if filtered.is_empty() {
        let empty = if state.events.is_empty() {
            "No events recorded"
        } else {
            "No events match filter"
        };
        let paragraph = Paragraph::new(empty)
            .style(Style::default().add_modifier(Modifier::DIM))
            .block(Block::default().title("Events").borders(Borders::ALL));
        f.render_widget(paragraph, area);
        return;
    }

    let items: Vec<ListItem> = filtered
        .iter()
        .enumerate()
        .map(|(i, event)| {
            let style = if i == state.events_view.scroll_offset {
                Style::default().add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };

            let time = event.timestamp.format("%H:%M:%S");
            let text = format!(
                "[{}] {} {}",
                time,
                event.event_type,
                event.payload.chars().take(60).collect::<String>()
            );
            ListItem::new(text).style(style)
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .title(format!("Events ({})", filtered.len()))
            .borders(Borders::ALL),
    );

    let mut list_state = ListState::default();
    list_state.select(Some(state.events_view.scroll_offset));
    f.render_stateful_widget(list, area, &mut list_state);
}
