use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    widgets::{List, ListItem, ListState},
    Frame,
};

use crate::tui::state::AppState;

pub fn render_feed(f: &mut Frame, area: Rect, state: &mut AppState) {
    let filtered: Vec<_> = state
        .feed
        .iter()
        .filter(|item| state.chats_view.active_filter.matches(item))
        .collect();

    if filtered.is_empty() {
        state.chats_view.selected_feed_id = None;
        let empty = ratatui::widgets::Paragraph::new("No items")
            .style(Style::default().add_modifier(Modifier::DIM));
        f.render_widget(empty, area);
        return;
    }

    // Resolve ID to filtered index
    let selected_idx = state
        .chats_view
        .selected_feed_id
        .and_then(|id| filtered.iter().position(|item| item.id == id))
        .unwrap_or(0);

    // Clamp selection if ID not found or out of bounds
    let selected_idx = selected_idx.min(filtered.len() - 1);

    // Update state with resolved ID (in case it was clamped)
    state.chats_view.selected_feed_id = Some(filtered[selected_idx].id);

    let items: Vec<ListItem> = filtered
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let selected = i == selected_idx;
            let style = if selected {
                Style::default().add_modifier(Modifier::BOLD)
            } else if item.unread {
                Style::default()
            } else if item.resolved {
                Style::default().add_modifier(Modifier::DIM)
            } else {
                Style::default()
            };

            let text = format!(
                "[{}] {}: {}",
                item.kind.as_str(),
                item.project_name,
                item.summary
            );
            ListItem::new(text).style(style)
        })
        .collect();

    let list = List::new(items);
    let mut list_state = ListState::default();
    list_state.select(Some(selected_idx));

    f.render_stateful_widget(list, area, &mut list_state);
}
