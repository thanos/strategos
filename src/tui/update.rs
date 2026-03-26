use crossterm::event::KeyCode;
use ratatui::Frame;

use crate::tui::event::{Effect, UiEvent};
use crate::tui::feed::{FeedFilter, FeedItem};
use crate::tui::state::AppState;
use crate::tui::types::{FocusRegion, TopLevelTab, UiMode};
use crate::tui::views;
use crate::tui::widgets::{self, sidebar::render_sidebar};

pub const FILTERS: &[FeedFilter] = &[
    FeedFilter::All,
    FeedFilter::NeedsReply,
    FeedFilter::Review,
    FeedFilter::Commit,
    FeedFilter::Blocked,
    FeedFilter::Budget,
    FeedFilter::Unread,
];

const REFRESH_INTERVAL_TICKS: u32 = 40;

pub fn update(state: &mut AppState, event: UiEvent, tick_count: &mut u32) -> Vec<Effect> {
    let mut effects = Vec::new();

    match event {
        UiEvent::Key(key) => {
            if state.show_help {
                match key.code {
                    KeyCode::Char('?') | KeyCode::Esc => {
                        state.show_help = false;
                    }
                    _ => {}
                }
                return effects;
            }

            match state.mode {
                UiMode::Normal => {
                    handle_normal_mode(state, key, &mut effects);
                }
                UiMode::Input => {
                    handle_input_mode(state, key, &mut effects);
                }
                UiMode::Detail => {
                    handle_detail_mode(state, key, &mut effects);
                }
                UiMode::Confirm => {
                    handle_confirm_mode(state, key, &mut effects);
                }
            }
        }
        UiEvent::Tick => {
            *tick_count += 1;
            if *tick_count >= REFRESH_INTERVAL_TICKS {
                *tick_count = 0;
                effects.push(Effect::RefreshState);
            }
        }
        UiEvent::Resize(_, _) => {}
        UiEvent::ErrorOccurred(msg) => {
            state.error_message = Some(msg);
        }
        UiEvent::ClearError => {
            state.error_message = None;
        }
        _ => {}
    }

    effects
}

fn get_filtered_feed<'a>(state: &'a AppState) -> Vec<&'a FeedItem> {
    state
        .feed
        .iter()
        .filter(|item| state.chats_view.active_filter.matches(item))
        .collect()
}

fn resolve_feed_index(state: &AppState, filtered: &[&FeedItem]) -> Option<usize> {
    state
        .chats_view
        .selected_feed_id
        .and_then(|id| filtered.iter().position(|item| item.id == id))
}

fn handle_normal_mode(
    state: &mut AppState,
    key: crossterm::event::KeyEvent,
    effects: &mut Vec<Effect>,
) {
    match key.code {
        KeyCode::Char('q') => {
            state.should_quit = true;
            effects.push(Effect::Quit);
        }
        KeyCode::Char('?') => {
            state.show_help = true;
        }
        KeyCode::Char('1') => state.current_tab = TopLevelTab::Chats,
        KeyCode::Char('2') => state.current_tab = TopLevelTab::Projects,
        KeyCode::Char('3') => state.current_tab = TopLevelTab::Queue,
        KeyCode::Char('4') => state.current_tab = TopLevelTab::Budget,
        KeyCode::Char('5') => state.current_tab = TopLevelTab::Events,
        KeyCode::Tab => {
            state.focused = match state.focused {
                FocusRegion::Tabs => FocusRegion::Projects,
                FocusRegion::Projects => FocusRegion::Filters,
                FocusRegion::Filters => FocusRegion::Feed,
                FocusRegion::Feed => FocusRegion::Composer,
                FocusRegion::Composer => FocusRegion::Tabs,
            };
        }
        KeyCode::BackTab => {
            state.focused = match state.focused {
                FocusRegion::Tabs => FocusRegion::Composer,
                FocusRegion::Projects => FocusRegion::Tabs,
                FocusRegion::Filters => FocusRegion::Projects,
                FocusRegion::Feed => FocusRegion::Filters,
                FocusRegion::Composer => FocusRegion::Feed,
            };
        }
        KeyCode::Char('j') | KeyCode::Down => match state.focused {
            FocusRegion::Projects => {
                if state.chats_view.selected_project_index < state.projects.len().saturating_sub(1)
                {
                    state.chats_view.selected_project_index += 1;
                }
            }
            FocusRegion::Filters => {
                if state.chats_view.selected_filter_index < FILTERS.len() - 1 {
                    state.chats_view.selected_filter_index += 1;
                    update_active_filter(state);
                }
            }
            FocusRegion::Feed => {
                let filtered = get_filtered_feed(state);
                if filtered.is_empty() {
                    state.chats_view.selected_feed_id = None;
                } else {
                    let current_idx = resolve_feed_index(state, &filtered);
                    let new_idx = match current_idx {
                        Some(idx) => (idx + 1).min(filtered.len() - 1),
                        None => 0,
                    };
                    state.chats_view.selected_feed_id = Some(filtered[new_idx].id);
                }
            }
            _ => {}
        },
        KeyCode::Char('k') | KeyCode::Up => match state.focused {
            FocusRegion::Projects => {
                if state.chats_view.selected_project_index > 0 {
                    state.chats_view.selected_project_index -= 1;
                }
            }
            FocusRegion::Filters => {
                if state.chats_view.selected_filter_index > 0 {
                    state.chats_view.selected_filter_index -= 1;
                    update_active_filter(state);
                }
            }
            FocusRegion::Feed => {
                let filtered = get_filtered_feed(state);
                if filtered.is_empty() {
                    state.chats_view.selected_feed_id = None;
                } else {
                    let current_idx = resolve_feed_index(state, &filtered);
                    let new_idx = match current_idx {
                        Some(idx) => idx.saturating_sub(1),
                        None => 0,
                    };
                    state.chats_view.selected_feed_id = Some(filtered[new_idx].id);
                }
            }
            _ => {}
        },
        KeyCode::Char('i') => {
            state.mode = UiMode::Input;
            state.focused = FocusRegion::Composer;
        }
        KeyCode::Enter => {
            if state.focused == FocusRegion::Feed {
                state.mode = UiMode::Detail;
            }
        }
        KeyCode::Esc => {
            state.error_message = None;
        }
        _ => {}
    }
}

fn update_active_filter(state: &mut AppState) {
    if let Some(filter) = FILTERS.get(state.chats_view.selected_filter_index) {
        state.chats_view.active_filter = filter.clone();

        // Reconcile selected_feed_id against the new filter
        let filtered = get_filtered_feed(state);

        if filtered.is_empty() {
            state.chats_view.selected_feed_id = None;
        } else {
            // Check if current selection is still visible
            let current_visible = state
                .chats_view
                .selected_feed_id
                .map(|id| filtered.iter().any(|item| item.id == id))
                .unwrap_or(false);

            if !current_visible {
                // Select the first visible item
                state.chats_view.selected_feed_id = Some(filtered[0].id);
            }
        }
    }
}

fn handle_input_mode(
    state: &mut AppState,
    key: crossterm::event::KeyEvent,
    effects: &mut Vec<Effect>,
) {
    match key.code {
        KeyCode::Esc => {
            state.mode = UiMode::Normal;
            state.composer.input.clear();
            state.composer.cursor_position = 0;
            state.composer.history_index = None;
        }
        KeyCode::Enter => {
            if !state.composer.input.is_empty() {
                let input = state.composer.input.clone();

                match parse_composer_input(&input, state) {
                    Some((project, description)) => {
                        state.composer.history.push(input);
                        state.composer.input.clear();
                        state.composer.cursor_position = 0;
                        state.composer.history_index = None;
                        state.mode = UiMode::Normal;

                        effects.push(Effect::SubmitTask {
                            project,
                            description,
                        });
                    }
                    None => {
                        effects.push(Effect::ShowError(
                            "No project context available. Select a project or use 'project_name message' format.".to_string()
                        ));
                    }
                }
            }
        }
        KeyCode::Char(c) => {
            state.composer.input.push(c);
            state.composer.cursor_position = state.composer.input.chars().count();
        }
        KeyCode::Backspace => {
            if !state.composer.input.is_empty() {
                let char_count = state.composer.input.chars().count();
                if char_count > 0 {
                    let byte_index = state
                        .composer
                        .input
                        .char_indices()
                        .nth(char_count - 1)
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                    state.composer.input.truncate(byte_index);
                    state.composer.cursor_position = char_count - 1;
                }
            }
        }
        KeyCode::Up => {
            if !state.composer.history.is_empty() {
                let idx = state
                    .composer
                    .history_index
                    .unwrap_or(state.composer.history.len());
                if idx > 0 {
                    state.composer.history_index = Some(idx - 1);
                    state.composer.input = state.composer.history[idx - 1].clone();
                    state.composer.cursor_position = state.composer.input.chars().count();
                }
            }
        }
        KeyCode::Down => {
            if let Some(idx) = state.composer.history_index {
                if idx < state.composer.history.len() - 1 {
                    state.composer.history_index = Some(idx + 1);
                    state.composer.input = state.composer.history[idx + 1].clone();
                    state.composer.cursor_position = state.composer.input.chars().count();
                } else {
                    state.composer.history_index = None;
                    state.composer.input.clear();
                    state.composer.cursor_position = 0;
                }
            }
        }
        _ => {}
    }
}

fn handle_detail_mode(
    state: &mut AppState,
    key: crossterm::event::KeyEvent,
    _effects: &mut Vec<Effect>,
) {
    match key.code {
        KeyCode::Esc => {
            state.mode = UiMode::Normal;
        }
        KeyCode::Char('j') | KeyCode::Down => {}
        KeyCode::Char('k') | KeyCode::Up => {}
        _ => {}
    }
}

fn handle_confirm_mode(
    _state: &mut AppState,
    _key: crossterm::event::KeyEvent,
    _effects: &mut Vec<Effect>,
) {
}

fn parse_composer_input(
    input: &str,
    state: &AppState,
) -> Option<(crate::models::ProjectId, String)> {
    // Try to find an explicit project prefix (word followed by space)
    if let Some(space_pos) = input.find(' ') {
        let potential_project = &input[..space_pos];
        let description = input[space_pos + 1..].to_string();

        // Check if first word matches a project name
        if let Some(project) = state.projects.iter().find(|p| p.name == potential_project) {
            return Some((project.id.clone(), description));
        }
    }

    // Fallback 1: Route to selected feed item's project (or first visible if no selection)
    let filtered = get_filtered_feed(state);

    if let Some(selected_id) = state.chats_view.selected_feed_id {
        // Use the explicitly selected item
        if let Some(item) = filtered.iter().find(|i| i.id == selected_id) {
            return Some((item.project_id.clone(), input.to_string()));
        }
    }

    // If no selection, use the first visible item
    if let Some(first_item) = filtered.first() {
        return Some((first_item.project_id.clone(), input.to_string()));
    }

    // Fallback 2: Route to selected project in sidebar
    if let Some(project) = state.projects.get(state.chats_view.selected_project_index) {
        return Some((project.id.clone(), input.to_string()));
    }

    None
}

pub fn render_app(f: &mut Frame, state: &mut AppState) {
    use crate::tui::{COMPOSER_HEIGHT, FOOTER_HEIGHT, HEADER_HEIGHT, SIDEBAR_WIDTH};

    let size = f.area();

    let header_area = ratatui::layout::Rect::new(0, 0, size.width, HEADER_HEIGHT);
    let footer_y = size.height.saturating_sub(FOOTER_HEIGHT);
    let footer_area = ratatui::layout::Rect::new(0, footer_y, size.width, FOOTER_HEIGHT);

    let composer_y = footer_y.saturating_sub(COMPOSER_HEIGHT);
    let composer_area = ratatui::layout::Rect::new(0, composer_y, size.width, COMPOSER_HEIGHT);

    let body_height = composer_y.saturating_sub(HEADER_HEIGHT);
    let body_y = HEADER_HEIGHT;

    let sidebar_area =
        ratatui::layout::Rect::new(0, body_y, SIDEBAR_WIDTH.min(size.width), body_height);
    let main_x = SIDEBAR_WIDTH.min(size.width);
    let main_width = size.width.saturating_sub(main_x);
    let main_area = ratatui::layout::Rect::new(main_x, body_y, main_width, body_height);

    widgets::render_header(f, header_area, state);
    render_sidebar(f, sidebar_area, state);
    widgets::render_footer(f, footer_area, state);
    widgets::render_composer(f, composer_area, state);

    match state.current_tab {
        TopLevelTab::Chats => {
            widgets::render_feed(f, main_area, state);
        }
        TopLevelTab::Projects => {
            views::render_projects_view(f, main_area, state);
        }
        TopLevelTab::Queue => {
            views::render_queue_view(f, main_area, state);
        }
        TopLevelTab::Budget => {
            views::render_budget_view(f, main_area, state);
        }
        TopLevelTab::Events => {
            views::render_events_view(f, main_area, state);
        }
    }

    if state.show_help {
        widgets::render_help(f, size);
    }
}
