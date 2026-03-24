use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;

use crate::tui::event::{Effect, UiEvent};
use crate::tui::state::AppState;
use crate::tui::types::{FocusRegion, TopLevelTab, UiMode};
use crate::tui::views;
use crate::tui::widgets::{self, sidebar::render_sidebar};

pub fn update(state: &mut AppState, event: UiEvent) -> Vec<Effect> {
    let mut effects = Vec::new();

    match event {
        UiEvent::Key(key) => {
            if state.show_help {
                state.show_help = false;
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
            effects.push(Effect::RefreshState);
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
                let filter_count = 7;
                if state.chats_view.selected_filter_index < filter_count - 1 {
                    state.chats_view.selected_filter_index += 1;
                }
            }
            FocusRegion::Feed => {
                let feed_len = state.feed.len();
                if feed_len > 0 && state.chats_view.selected_feed_index < feed_len - 1 {
                    state.chats_view.selected_feed_index += 1;
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
                }
            }
            FocusRegion::Feed => {
                if state.chats_view.selected_feed_index > 0 {
                    state.chats_view.selected_feed_index -= 1;
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

fn handle_input_mode(
    state: &mut AppState,
    key: crossterm::event::KeyEvent,
    effects: &mut Vec<Effect>,
) {
    match key.code {
        KeyCode::Esc => {
            state.mode = UiMode::Normal;
            state.composer.input.clear();
        }
        KeyCode::Enter => {
            if !state.composer.input.is_empty() {
                let input = state.composer.input.clone();
                state.composer.history.push(input.clone());
                state.composer.input.clear();
                state.mode = UiMode::Normal;

                if let Some((project, description)) = parse_composer_input(&input, state) {
                    effects.push(Effect::SubmitTask {
                        project,
                        description,
                    });
                }
            }
        }
        KeyCode::Char(c) => {
            state.composer.input.push(c);
            state.composer.cursor_position += 1;
        }
        KeyCode::Backspace => {
            if state.composer.cursor_position > 0 {
                state.composer.cursor_position -= 1;
                state.composer.input.remove(state.composer.cursor_position);
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
                }
            }
        }
        KeyCode::Down => {
            if let Some(idx) = state.composer.history_index {
                if idx < state.composer.history.len() - 1 {
                    state.composer.history_index = Some(idx + 1);
                    state.composer.input = state.composer.history[idx + 1].clone();
                } else {
                    state.composer.history_index = None;
                    state.composer.input.clear();
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
    let parts: Vec<&str> = input.splitn(2, ' ').collect();
    if parts.len() < 2 {
        return None;
    }

    let potential_project = parts[0];
    let description = parts[1].to_string();

    if let Some(project) = state.projects.iter().find(|p| p.name == potential_project) {
        return Some((project.id.clone(), description));
    }

    if let Some(item) = state.feed.get(state.chats_view.selected_feed_index) {
        return Some((item.project_id.clone(), input.to_string()));
    }

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
