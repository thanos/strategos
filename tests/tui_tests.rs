use strategos::tui::event::{Effect, UiEvent};
use strategos::tui::feed::FeedFilter;
use strategos::tui::state::AppState;
use strategos::tui::types::{FocusRegion, TopLevelTab, UiMode};
use strategos::tui::update::{update, FILTERS};

fn create_test_state() -> AppState {
    AppState::default()
}

#[test]
fn test_quit_key() {
    let mut state = create_test_state();
    let mut tick_count = 0;

    let key = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('q'),
        crossterm::event::KeyModifiers::NONE,
    );

    let effects = update(&mut state, UiEvent::Key(key), &mut tick_count);

    assert!(state.should_quit);
    assert!(effects.iter().any(|e| matches!(e, Effect::Quit)));
}

#[test]
fn test_help_toggle() {
    let mut state = create_test_state();
    let mut tick_count = 0;

    let key = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('?'),
        crossterm::event::KeyModifiers::NONE,
    );

    update(&mut state, UiEvent::Key(key), &mut tick_count);
    assert!(state.show_help);

    update(&mut state, UiEvent::Key(key), &mut tick_count);
    assert!(!state.show_help);
}

#[test]
fn test_help_closes_on_esc() {
    let mut state = create_test_state();
    let mut tick_count = 0;

    let question = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('?'),
        crossterm::event::KeyModifiers::NONE,
    );
    let esc = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Esc,
        crossterm::event::KeyModifiers::NONE,
    );

    update(&mut state, UiEvent::Key(question), &mut tick_count);
    assert!(state.show_help);

    update(&mut state, UiEvent::Key(esc), &mut tick_count);
    assert!(!state.show_help);
}

#[test]
fn test_help_ignores_other_keys() {
    let mut state = create_test_state();
    let mut tick_count = 0;

    let question = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('?'),
        crossterm::event::KeyModifiers::NONE,
    );
    let other_key = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('j'),
        crossterm::event::KeyModifiers::NONE,
    );

    update(&mut state, UiEvent::Key(question), &mut tick_count);
    assert!(state.show_help);

    // Other keys should be ignored while help is open
    update(&mut state, UiEvent::Key(other_key), &mut tick_count);
    assert!(state.show_help);

    // Selected feed ID should not change (should remain None since no feed items)
    assert!(state.chats_view.selected_feed_id.is_none());
}

#[test]
fn test_tab_switching() {
    let mut state = create_test_state();
    let mut tick_count = 0;

    let key = |c: char| {
        crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char(c),
            crossterm::event::KeyModifiers::NONE,
        )
    };

    update(&mut state, UiEvent::Key(key('2')), &mut tick_count);
    assert_eq!(state.current_tab, TopLevelTab::Projects);

    update(&mut state, UiEvent::Key(key('4')), &mut tick_count);
    assert_eq!(state.current_tab, TopLevelTab::Budget);

    update(&mut state, UiEvent::Key(key('1')), &mut tick_count);
    assert_eq!(state.current_tab, TopLevelTab::Chats);
}

#[test]
fn test_focus_cycling() {
    let mut state = create_test_state();
    assert_eq!(state.focused, FocusRegion::Feed);
    let mut tick_count = 0;

    let tab = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Tab,
        crossterm::event::KeyModifiers::NONE,
    );

    update(&mut state, UiEvent::Key(tab.clone()), &mut tick_count);
    assert_eq!(state.focused, FocusRegion::Composer);

    update(&mut state, UiEvent::Key(tab.clone()), &mut tick_count);
    assert_eq!(state.focused, FocusRegion::Tabs);

    update(&mut state, UiEvent::Key(tab.clone()), &mut tick_count);
    assert_eq!(state.focused, FocusRegion::Projects);

    update(&mut state, UiEvent::Key(tab.clone()), &mut tick_count);
    assert_eq!(state.focused, FocusRegion::Filters);

    update(&mut state, UiEvent::Key(tab.clone()), &mut tick_count);
    assert_eq!(state.focused, FocusRegion::Feed);
}

#[test]
fn test_backtab_cycling() {
    let mut state = create_test_state();
    assert_eq!(state.focused, FocusRegion::Feed);
    let mut tick_count = 0;

    let backtab = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::BackTab,
        crossterm::event::KeyModifiers::NONE,
    );

    update(&mut state, UiEvent::Key(backtab.clone()), &mut tick_count);
    assert_eq!(state.focused, FocusRegion::Filters);

    update(&mut state, UiEvent::Key(backtab.clone()), &mut tick_count);
    assert_eq!(state.focused, FocusRegion::Projects);
}

#[test]
fn test_input_mode_entry() {
    let mut state = create_test_state();
    let mut tick_count = 0;

    let key = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('i'),
        crossterm::event::KeyModifiers::NONE,
    );

    update(&mut state, UiEvent::Key(key), &mut tick_count);
    assert_eq!(state.mode, UiMode::Input);
    assert_eq!(state.focused, FocusRegion::Composer);
}

#[test]
fn test_composer_typing() {
    let mut state = create_test_state();
    state.mode = UiMode::Input;
    state.focused = FocusRegion::Composer;
    let mut tick_count = 0;

    for c in "hello".chars() {
        let key = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char(c),
            crossterm::event::KeyModifiers::NONE,
        );
        update(&mut state, UiEvent::Key(key), &mut tick_count);
    }

    assert_eq!(state.composer.input, "hello");
    assert_eq!(state.composer.cursor_position, 5);
}

#[test]
fn test_composer_backspace() {
    let mut state = create_test_state();
    state.mode = UiMode::Input;
    state.focused = FocusRegion::Composer;
    state.composer.input = "test".to_string();
    state.composer.cursor_position = 4;
    let mut tick_count = 0;

    let backspace = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Backspace,
        crossterm::event::KeyModifiers::NONE,
    );

    update(&mut state, UiEvent::Key(backspace.clone()), &mut tick_count);
    assert_eq!(state.composer.input, "tes");
    assert_eq!(state.composer.cursor_position, 3);

    update(&mut state, UiEvent::Key(backspace.clone()), &mut tick_count);
    assert_eq!(state.composer.input, "te");
    assert_eq!(state.composer.cursor_position, 2);
}

#[test]
fn test_composer_esc_clears() {
    let mut state = create_test_state();
    state.mode = UiMode::Input;
    state.composer.input = "test".to_string();
    state.composer.cursor_position = 4;
    state.composer.history_index = Some(0);
    let mut tick_count = 0;

    let esc = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Esc,
        crossterm::event::KeyModifiers::NONE,
    );

    update(&mut state, UiEvent::Key(esc), &mut tick_count);

    assert_eq!(state.mode, UiMode::Normal);
    assert!(state.composer.input.is_empty());
    assert_eq!(state.composer.cursor_position, 0);
    assert!(state.composer.history_index.is_none());
}

#[test]
fn test_filter_navigation_updates_active_filter() {
    let mut state = create_test_state();
    state.focused = FocusRegion::Filters;
    let mut tick_count = 0;

    let down = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Down,
        crossterm::event::KeyModifiers::NONE,
    );

    assert_eq!(state.chats_view.active_filter, FeedFilter::All);

    update(&mut state, UiEvent::Key(down.clone()), &mut tick_count);
    assert_eq!(state.chats_view.selected_filter_index, 1);
    assert_eq!(state.chats_view.active_filter, FeedFilter::NeedsReply);

    update(&mut state, UiEvent::Key(down.clone()), &mut tick_count);
    assert_eq!(state.chats_view.selected_filter_index, 2);
    assert_eq!(state.chats_view.active_filter, FeedFilter::Review);
}

#[test]
fn test_project_navigation_bounds() {
    use strategos::models::ProjectId;
    use strategos::tui::domain::{ProjectState, ProjectStatus};

    let mut state = create_test_state();
    state.focused = FocusRegion::Projects;
    state.projects = vec![
        ProjectState {
            id: ProjectId::new(),
            name: "p1".to_string(),
            status: ProjectStatus::Healthy,
            unread_count: 0,
            pending_actions: 0,
            default_backend: None,
            last_activity: None,
            budget_percent: 0.0,
        },
        ProjectState {
            id: ProjectId::new(),
            name: "p2".to_string(),
            status: ProjectStatus::Healthy,
            unread_count: 0,
            pending_actions: 0,
            default_backend: None,
            last_activity: None,
            budget_percent: 0.0,
        },
    ];
    let mut tick_count = 0;

    let down = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Down,
        crossterm::event::KeyModifiers::NONE,
    );
    let up = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Up,
        crossterm::event::KeyModifiers::NONE,
    );

    update(&mut state, UiEvent::Key(up.clone()), &mut tick_count);
    assert_eq!(state.chats_view.selected_project_index, 0);

    update(&mut state, UiEvent::Key(down.clone()), &mut tick_count);
    assert_eq!(state.chats_view.selected_project_index, 1);

    update(&mut state, UiEvent::Key(down.clone()), &mut tick_count);
    assert_eq!(state.chats_view.selected_project_index, 1);
}

#[test]
fn test_filter_count_matches_constant() {
    assert_eq!(FILTERS.len(), 7);
    assert!(FILTERS.contains(&FeedFilter::All));
    assert!(FILTERS.contains(&FeedFilter::NeedsReply));
    assert!(FILTERS.contains(&FeedFilter::Review));
    assert!(FILTERS.contains(&FeedFilter::Commit));
    assert!(FILTERS.contains(&FeedFilter::Blocked));
    assert!(FILTERS.contains(&FeedFilter::Budget));
    assert!(FILTERS.contains(&FeedFilter::Unread));
}

#[test]
fn test_throttled_refresh() {
    let mut state = create_test_state();
    let mut tick_count = 0;

    for _ in 0..39 {
        let effects = update(&mut state, UiEvent::Tick, &mut tick_count);
        assert!(!effects.iter().any(|e| matches!(e, Effect::RefreshState)));
    }

    let effects = update(&mut state, UiEvent::Tick, &mut tick_count);
    assert!(effects.iter().any(|e| matches!(e, Effect::RefreshState)));
    assert_eq!(tick_count, 0);
}

#[test]
fn test_feed_selection_persists_across_filter_change() {
    use strategos::models::ProjectId;
    use strategos::tui::feed::{FeedItem, FeedItemId, FeedItemKind};

    let mut state = create_test_state();
    state.focused = FocusRegion::Feed;
    let project_id = ProjectId::new();

    let review_id = FeedItemId::new();
    let update_id = FeedItemId::new();

    state.feed = vec![
        FeedItem {
            id: review_id,
            project_id: project_id.clone(),
            project_name: "test".to_string(),
            kind: FeedItemKind::ReviewRequest,
            summary: "review item".to_string(),
            detail: String::new(),
            source_backend: None,
            created_at: chrono::Utc::now(),
            requires_response: true,
            resolved: false,
            unread: true,
            suggested_actions: vec![],
            linked_action_id: None,
            linked_event_ids: vec![],
        },
        FeedItem {
            id: update_id,
            project_id: project_id.clone(),
            project_name: "test".to_string(),
            kind: FeedItemKind::Update,
            summary: "update item".to_string(),
            detail: String::new(),
            source_backend: None,
            created_at: chrono::Utc::now(),
            requires_response: false,
            resolved: false,
            unread: true,
            suggested_actions: vec![],
            linked_action_id: None,
            linked_event_ids: vec![],
        },
    ];

    let mut tick_count = 0;
    let down = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Down,
        crossterm::event::KeyModifiers::NONE,
    );

    // Filter is All by default, navigate to select the review item
    assert_eq!(state.chats_view.active_filter, FeedFilter::All);
    update(&mut state, UiEvent::Key(down.clone()), &mut tick_count);
    assert_eq!(state.chats_view.selected_feed_id, Some(review_id));

    // Change filter to Review through the UI (navigate to Filters, press Down to select Review)
    state.focused = FocusRegion::Filters;
    let filter_down = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Down,
        crossterm::event::KeyModifiers::NONE,
    );
    // All(0) -> NeedsReply(1) -> Review(2)
    update(
        &mut state,
        UiEvent::Key(filter_down.clone()),
        &mut tick_count,
    );
    update(
        &mut state,
        UiEvent::Key(filter_down.clone()),
        &mut tick_count,
    );
    assert_eq!(state.chats_view.active_filter, FeedFilter::Review);

    // Selection should still point to the review item (WITHOUT navigating)
    // The review item is still visible with the Review filter
    assert_eq!(state.chats_view.selected_feed_id, Some(review_id));

    // Change filter back to All - update item should now be visible
    let filter_up = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Up,
        crossterm::event::KeyModifiers::NONE,
    );
    update(&mut state, UiEvent::Key(filter_up.clone()), &mut tick_count);
    update(&mut state, UiEvent::Key(filter_up.clone()), &mut tick_count);
    assert_eq!(state.chats_view.active_filter, FeedFilter::All);

    // Selection should STILL be on review item (WITHOUT navigating)
    assert_eq!(state.chats_view.selected_feed_id, Some(review_id));
}

#[test]
fn test_feed_navigation_respects_active_filter() {
    use strategos::models::ProjectId;
    use strategos::tui::feed::{FeedItem, FeedItemId, FeedItemKind};

    let mut state = create_test_state();
    state.focused = FocusRegion::Feed;
    let project_id = ProjectId::new();

    let id1 = FeedItemId::new();
    let id2 = FeedItemId::new();
    let id3 = FeedItemId::new();

    state.feed = vec![
        FeedItem {
            id: id1,
            project_id: project_id.clone(),
            project_name: "test".to_string(),
            kind: FeedItemKind::ReviewRequest,
            summary: "review 1".to_string(),
            detail: String::new(),
            source_backend: None,
            created_at: chrono::Utc::now(),
            requires_response: true,
            resolved: false,
            unread: true,
            suggested_actions: vec![],
            linked_action_id: None,
            linked_event_ids: vec![],
        },
        FeedItem {
            id: id2,
            project_id: project_id.clone(),
            project_name: "test".to_string(),
            kind: FeedItemKind::Update,
            summary: "update 1".to_string(),
            detail: String::new(),
            source_backend: None,
            created_at: chrono::Utc::now(),
            requires_response: false,
            resolved: false,
            unread: true,
            suggested_actions: vec![],
            linked_action_id: None,
            linked_event_ids: vec![],
        },
        FeedItem {
            id: id3,
            project_id: project_id.clone(),
            project_name: "test".to_string(),
            kind: FeedItemKind::ReviewRequest,
            summary: "review 2".to_string(),
            detail: String::new(),
            source_backend: None,
            created_at: chrono::Utc::now(),
            requires_response: true,
            resolved: false,
            unread: true,
            suggested_actions: vec![],
            linked_action_id: None,
            linked_event_ids: vec![],
        },
    ];

    // Filter to only show Review items
    state.chats_view.active_filter = FeedFilter::Review;

    let mut tick_count = 0;
    let down = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Down,
        crossterm::event::KeyModifiers::NONE,
    );

    // Should select first review item
    update(&mut state, UiEvent::Key(down.clone()), &mut tick_count);
    assert_eq!(state.chats_view.selected_feed_id, Some(id1));

    // Should skip update item and go to second review item
    update(&mut state, UiEvent::Key(down.clone()), &mut tick_count);
    assert_eq!(state.chats_view.selected_feed_id, Some(id3));
}

#[test]
fn test_feed_selection_clamped_on_empty_filter() {
    let mut state = create_test_state();
    state.focused = FocusRegion::Feed;
    state.feed = vec![];
    state.chats_view.selected_feed_id = None;

    let mut tick_count = 0;
    let down = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Down,
        crossterm::event::KeyModifiers::NONE,
    );

    // Should handle gracefully without panic
    update(&mut state, UiEvent::Key(down), &mut tick_count);
    assert!(state.chats_view.selected_feed_id.is_none());
}

#[test]
fn test_single_word_routes_to_selected_feed_item() {
    use strategos::models::ProjectId;
    use strategos::tui::feed::{FeedItem, FeedItemId, FeedItemKind};

    let mut state = create_test_state();
    state.mode = UiMode::Input;
    state.focused = FocusRegion::Composer;

    let project_id = ProjectId::new();
    let feed_id = FeedItemId::new();

    state.feed = vec![FeedItem {
        id: feed_id,
        project_id: project_id.clone(),
        project_name: "test_project".to_string(),
        kind: FeedItemKind::Update,
        summary: "test".to_string(),
        detail: String::new(),
        source_backend: None,
        created_at: chrono::Utc::now(),
        requires_response: false,
        resolved: false,
        unread: true,
        suggested_actions: vec![],
        linked_action_id: None,
        linked_event_ids: vec![],
    }];
    state.chats_view.selected_feed_id = Some(feed_id);

    state.composer.input = "hello".to_string();
    state.composer.cursor_position = 5;

    let mut tick_count = 0;
    let enter = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Enter,
        crossterm::event::KeyModifiers::NONE,
    );

    let effects = update(&mut state, UiEvent::Key(enter), &mut tick_count);

    assert!(effects
        .iter()
        .any(|e| matches!(e, Effect::SubmitTask { .. })));

    let submit_task = effects.iter().find_map(|e| match e {
        Effect::SubmitTask {
            project,
            description,
        } => Some((project.clone(), description.clone())),
        _ => None,
    });

    if let Some((project, description)) = submit_task {
        assert_eq!(project, project_id);
        assert_eq!(description, "hello");
    }
}

#[test]
fn test_single_word_routes_to_selected_project() {
    use strategos::models::ProjectId;
    use strategos::tui::domain::{ProjectState, ProjectStatus};

    let mut state = create_test_state();
    state.mode = UiMode::Input;
    state.focused = FocusRegion::Composer;
    state.chats_view.selected_feed_id = None;

    let project_id = ProjectId::new();
    state.projects = vec![ProjectState {
        id: project_id.clone(),
        name: "my_project".to_string(),
        status: ProjectStatus::Healthy,
        unread_count: 0,
        pending_actions: 0,
        default_backend: None,
        last_activity: None,
        budget_percent: 0.0,
    }];
    state.chats_view.selected_project_index = 0;

    state.composer.input = "fix the bug".to_string();
    state.composer.cursor_position = 11;

    let mut tick_count = 0;
    let enter = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Enter,
        crossterm::event::KeyModifiers::NONE,
    );

    let effects = update(&mut state, UiEvent::Key(enter), &mut tick_count);

    assert!(effects
        .iter()
        .any(|e| matches!(e, Effect::SubmitTask { .. })));

    let submit_task = effects.iter().find_map(|e| match e {
        Effect::SubmitTask {
            project,
            description,
        } => Some((project.clone(), description.clone())),
        _ => None,
    });

    if let Some((project, description)) = submit_task {
        assert_eq!(project, project_id);
        assert_eq!(description, "fix the bug");
    }
}

#[test]
fn test_routing_fails_shows_error() {
    let mut state = create_test_state();
    state.mode = UiMode::Input;
    state.focused = FocusRegion::Composer;
    state.chats_view.selected_feed_id = None;
    state.projects = vec![];
    state.chats_view.selected_project_index = 0;

    state.composer.input = "hello".to_string();
    state.composer.cursor_position = 5;

    let mut tick_count = 0;
    let enter = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Enter,
        crossterm::event::KeyModifiers::NONE,
    );

    let effects = update(&mut state, UiEvent::Key(enter), &mut tick_count);

    assert!(effects.iter().any(|e| matches!(e, Effect::ShowError(_))));
}

#[test]
fn test_routing_failure_preserves_input() {
    let mut state = create_test_state();
    state.mode = UiMode::Input;
    state.focused = FocusRegion::Composer;
    state.chats_view.selected_feed_id = None;
    state.projects = vec![];

    state.composer.input = "my unsent message".to_string();
    state.composer.cursor_position = 17;

    let mut tick_count = 0;
    let enter = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Enter,
        crossterm::event::KeyModifiers::NONE,
    );

    let _ = update(&mut state, UiEvent::Key(enter), &mut tick_count);

    // Input should be preserved when routing fails
    assert_eq!(state.composer.input, "my unsent message");
    assert_eq!(state.composer.cursor_position, 17);
    // Should stay in Input mode
    assert_eq!(state.mode, UiMode::Input);
}

#[test]
fn test_explicit_project_prefix_routes_correctly() {
    use strategos::models::ProjectId;
    use strategos::tui::domain::{ProjectState, ProjectStatus};

    let mut state = create_test_state();
    state.mode = UiMode::Input;
    state.focused = FocusRegion::Composer;

    let project_id = ProjectId::new();
    state.projects = vec![ProjectState {
        id: project_id.clone(),
        name: "frontend".to_string(),
        status: ProjectStatus::Healthy,
        unread_count: 0,
        pending_actions: 0,
        default_backend: None,
        last_activity: None,
        budget_percent: 0.0,
    }];

    state.composer.input = "frontend fix the header".to_string();
    state.composer.cursor_position = 23;

    let mut tick_count = 0;
    let enter = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Enter,
        crossterm::event::KeyModifiers::NONE,
    );

    let effects = update(&mut state, UiEvent::Key(enter), &mut tick_count);

    assert!(effects
        .iter()
        .any(|e| matches!(e, Effect::SubmitTask { .. })));

    let submit_task = effects.iter().find_map(|e| match e {
        Effect::SubmitTask {
            project,
            description,
        } => Some((project.clone(), description.clone())),
        _ => None,
    });

    if let Some((project, description)) = submit_task {
        assert_eq!(project, project_id);
        assert_eq!(description, "fix the header");
    }
}
