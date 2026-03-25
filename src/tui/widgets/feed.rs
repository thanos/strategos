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
        let empty = ratatui::widgets::Paragraph::new("No items")
            .style(Style::default().add_modifier(Modifier::DIM));
        f.render_widget(empty, area);
        return;
    }

    let items: Vec<ListItem> = filtered
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let selected = i == state.chats_view.selected_feed_index;
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
    list_state.select(if filtered.is_empty() {
        None
    } else {
        Some(state.chats_view.selected_feed_index.min(filtered.len() - 1))
    });

    f.render_stateful_widget(list, area, &mut list_state);
}
