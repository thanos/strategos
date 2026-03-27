use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::tui::state::AppState;

pub fn render_projects_view(f: &mut Frame, area: Rect, state: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
        .split(area);

    render_project_list(f, chunks[0], state);
    render_project_detail(f, chunks[1], state);
}

fn render_project_list(f: &mut Frame, area: Rect, state: &AppState) {
    let items: Vec<ListItem> = state
        .projects
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let selected = i == state.projects_view.selected_index;
            let style = if selected {
                Style::default().add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let marker = p.status.marker();
            let count = if p.unread_count > 0 {
                format!(" ({})", p.unread_count)
            } else {
                String::new()
            };
            ListItem::new(format!("{} {}{}", marker, p.name, count)).style(style)
        })
        .collect();

    let list = List::new(items).block(Block::default().title("Projects").borders(Borders::ALL));

    let mut list_state = ListState::default();
    list_state.select(Some(state.projects_view.selected_index));
    f.render_stateful_widget(list, area, &mut list_state);
}

fn render_project_detail(f: &mut Frame, area: Rect, state: &AppState) {
    let project = state.projects.get(state.projects_view.selected_index);

    let content = match project {
        Some(p) => build_project_detail(p, state),
        None => vec!["No project selected".to_string()],
    };

    let text = content.join("\n");
    let paragraph = Paragraph::new(text).block(
        Block::default()
            .title("Project Detail")
            .borders(Borders::ALL),
    );
    f.render_widget(paragraph, area);
}

fn build_project_detail(
    project: &crate::tui::domain::ProjectState,
    state: &AppState,
) -> Vec<String> {
    let mut lines = Vec::new();

    lines.push(format!("Name: {}", project.name));
    lines.push(format!("ID: {}", project.id));
    lines.push(format!("Status: {:?}", project.status));
    lines.push(String::new());

    lines.push(format!(
        "Backend: {}",
        project
            .default_backend
            .as_ref()
            .map(|b| b.to_string())
            .unwrap_or_else(|| "none".to_string())
    ));
    lines.push(format!("Budget: {:.1}%", project.budget_percent * 100.0));
    lines.push(String::new());

    lines.push(format!("Pending Tasks: {}", project.unread_count));
    lines.push(format!("Pending Actions: {}", project.pending_actions));

    if let Some(last) = &project.last_activity {
        lines.push(format!("Last Activity: {}", last.format("%Y-%m-%d %H:%M")));
    }

    lines.push(String::new());
    lines.push("Recent Feed Items:".to_string());

    let recent: Vec<_> = state
        .feed
        .iter()
        .filter(|item| item.project_id == project.id)
        .take(5)
        .collect();

    if recent.is_empty() {
        lines.push("  (no recent items)".to_string());
    } else {
        for item in recent {
            lines.push(format!(
                "  [{}] {}",
                item.kind.as_str(),
                item.summary.chars().take(50).collect::<String>()
            ));
        }
    }

    lines
}
