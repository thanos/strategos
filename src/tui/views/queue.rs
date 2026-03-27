use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::tui::domain::ActionKind;
use crate::tui::state::{AppState, QueueFilter};

const QUEUE_FILTERS: &[QueueFilter] = &[
    QueueFilter::All,
    QueueFilter::Review,
    QueueFilter::Commit,
    QueueFilter::Blocker,
    QueueFilter::Budget,
];

pub fn render_queue_view(f: &mut Frame, area: Rect, state: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(12), Constraint::Min(40)])
        .split(area);

    render_filter_panel(f, chunks[0], state);
    render_action_list(f, chunks[1], state);
}

fn render_filter_panel(f: &mut Frame, area: Rect, state: &AppState) {
    let items: Vec<ListItem> = QUEUE_FILTERS
        .iter()
        .map(|filter| {
            let selected = *filter == state.queue_view.selected_filter;
            let style = if selected {
                Style::default().add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let label = match filter {
                QueueFilter::All => "all",
                QueueFilter::Review => "review",
                QueueFilter::Commit => "commit",
                QueueFilter::Blocker => "blocker",
                QueueFilter::Budget => "budget",
            };
            let prefix = if selected { "[" } else { " " };
            let suffix = if selected { "]" } else { "" };
            ListItem::new(format!("{}{}{}", prefix, label, suffix)).style(style)
        })
        .collect();

    let list = List::new(items).block(Block::default().title("Filter").borders(Borders::ALL));
    f.render_widget(list, area);
}

fn render_action_list(f: &mut Frame, area: Rect, state: &AppState) {
    let filtered: Vec<_> = state
        .actions
        .iter()
        .filter(|a| !a.resolved && action_matches_filter(a.kind, state.queue_view.selected_filter))
        .collect();

    if filtered.is_empty() {
        let empty = Paragraph::new("No pending actions")
            .style(Style::default().add_modifier(Modifier::DIM))
            .block(Block::default().title("Actions").borders(Borders::ALL));
        f.render_widget(empty, area);
        return;
    }

    let items: Vec<ListItem> = filtered
        .iter()
        .enumerate()
        .map(|(i, action)| {
            let selected = i == state.queue_view.selected_index;
            let style = if selected {
                Style::default().add_modifier(Modifier::BOLD)
            } else if action.requires_user_decision {
                Style::default()
            } else {
                Style::default().add_modifier(Modifier::DIM)
            };

            let text = format!(
                "[{}] {}: {}",
                action.kind.as_str(),
                action.project_name,
                action.summary
            );
            ListItem::new(text).style(style)
        })
        .collect();

    let list = List::new(items).block(Block::default().title("Actions").borders(Borders::ALL));

    let mut list_state = ListState::default();
    let select_idx = state
        .queue_view
        .selected_index
        .min(filtered.len().saturating_sub(1));
    list_state.select(Some(select_idx));
    f.render_stateful_widget(list, area, &mut list_state);
}

fn action_matches_filter(kind: ActionKind, filter: QueueFilter) -> bool {
    match filter {
        QueueFilter::All => true,
        QueueFilter::Review => kind == ActionKind::ReviewRequest,
        QueueFilter::Commit => kind == ActionKind::CommitSuggestion,
        QueueFilter::Blocker => kind == ActionKind::Blocker,
        QueueFilter::Budget => kind == ActionKind::BudgetApproval,
    }
}
