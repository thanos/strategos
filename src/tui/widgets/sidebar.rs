use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    widgets::{List, ListItem, ListState},
    Frame,
};

use crate::tui::state::AppState;
use crate::tui::{types::FocusRegion, SIDEBAR_WIDTH};

pub fn render_sidebar(f: &mut Frame, area: Rect, state: &mut AppState) {
    let tabs = ["Chats", "Projects", "Queue", "Budget", "Events"];
    let selected_tab = match state.current_tab {
        crate::tui::types::TopLevelTab::Chats => 0,
        crate::tui::types::TopLevelTab::Projects => 1,
        crate::tui::types::TopLevelTab::Queue => 2,
        crate::tui::types::TopLevelTab::Budget => 3,
        crate::tui::types::TopLevelTab::Events => 4,
    };

    let is_focused = state.focused == FocusRegion::Tabs;

    let tab_items: Vec<ListItem> = tabs
        .iter()
        .enumerate()
        .map(|(i, &name)| {
            let style = if i == selected_tab {
                Style::default().add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let prefix = if i == selected_tab { "[" } else { " " };
            let suffix = if i == selected_tab { "]" } else { "" };
            ListItem::new(format!("{}{}{}", prefix, name, suffix)).style(style)
        })
        .collect();

    let tabs_list = List::new(tab_items).style(if is_focused {
        Style::default()
    } else {
        Style::default().add_modifier(Modifier::DIM)
    });

    let tab_count = tabs.len() as u16 + 1;
    f.render_stateful_widget(
        tabs_list,
        Rect::new(area.x, area.y, SIDEBAR_WIDTH.min(area.width), tab_count),
        &mut ListState::default(),
    );

    let projects_start_y = area.y + tab_count + 1;
    let projects_height = (area.height.saturating_sub(tab_count + 8)).min(area.height);

    let project_items: Vec<ListItem> = state
        .projects
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let selected = i == state.chats_view.selected_project_index;
            let style = if selected {
                Style::default().add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let count = if p.unread_count > 0 {
                format!(" ({})", p.unread_count)
            } else {
                String::new()
            };
            let marker = p.status.marker();
            ListItem::new(format!("{} {}{}", marker, p.name, count)).style(style)
        })
        .collect();

    let projects_list = List::new(project_items).style(if state.focused == FocusRegion::Projects {
        Style::default()
    } else {
        Style::default().add_modifier(Modifier::DIM)
    });

    f.render_widget(
        projects_list,
        Rect::new(
            area.x,
            projects_start_y,
            SIDEBAR_WIDTH.min(area.width),
            projects_height,
        ),
    );

    let filters_start_y = projects_start_y + projects_height + 1;
    let filters = [
        "all",
        "needs_reply",
        "review",
        "commit",
        "blocked",
        "budget",
        "unread",
    ];

    let filter_items: Vec<ListItem> = filters
        .iter()
        .enumerate()
        .map(|(i, &name)| {
            let selected = i == state.chats_view.selected_filter_index;
            let style = if selected {
                Style::default().add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let prefix = if selected { "[" } else { " " };
            let suffix = if selected { "]" } else { "" };
            ListItem::new(format!("{}{}{}", prefix, name, suffix)).style(style)
        })
        .collect();

    let filters_list = List::new(filter_items).style(if state.focused == FocusRegion::Filters {
        Style::default()
    } else {
        Style::default().add_modifier(Modifier::DIM)
    });

    let filters_height = (filters.len() + 1) as u16;
    f.render_widget(
        filters_list,
        Rect::new(
            area.x,
            filters_start_y,
            SIDEBAR_WIDTH.min(area.width),
            filters_height,
        ),
    );
}
