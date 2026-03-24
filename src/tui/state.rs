use std::collections::HashMap;

use crate::models::BackendId;
use crate::tui::domain::ActionItem;
use crate::tui::domain::{BudgetState, ProjectState, RoutingState};
use crate::tui::feed::{FeedFilter, FeedItem};
use crate::tui::types::{FocusRegion, TopLevelTab, UiMode};

pub struct AppState {
    pub current_tab: TopLevelTab,
    pub mode: UiMode,
    pub focused: FocusRegion,
    pub projects: Vec<ProjectState>,
    pub feed: Vec<FeedItem>,
    pub actions: Vec<ActionItem>,
    pub events: Vec<EventRecord>,
    pub budget: BudgetState,
    pub routing: RoutingState,
    pub composer: ComposerState,
    pub chats_view: ChatsViewState,
    pub projects_view: ProjectsViewState,
    pub queue_view: QueueViewState,
    pub budget_view: BudgetViewState,
    pub events_view: EventsViewState,
    pub error_message: Option<String>,
    pub show_help: bool,
    pub should_quit: bool,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            current_tab: TopLevelTab::default(),
            mode: UiMode::default(),
            focused: FocusRegion::default(),
            projects: Vec::new(),
            feed: Vec::new(),
            actions: Vec::new(),
            events: Vec::new(),
            budget: BudgetState::default(),
            routing: RoutingState::default(),
            composer: ComposerState::default(),
            chats_view: ChatsViewState::default(),
            projects_view: ProjectsViewState::default(),
            queue_view: QueueViewState::default(),
            budget_view: BudgetViewState::default(),
            events_view: EventsViewState::default(),
            error_message: None,
            show_help: false,
            should_quit: false,
        }
    }
}

impl AppState {
    pub fn load_from_storage(storage: &crate::storage::sqlite::SqliteStorage) -> Self {
        let projects = Self::load_projects(storage);
        let feed = crate::tui::feed::feed_items_from_storage(storage);
        let actions = Self::load_actions(storage);
        let budget = Self::load_budget(storage);
        let events = Self::load_events(storage);

        Self {
            projects,
            feed,
            actions,
            events,
            budget,
            ..Self::default()
        }
    }

    fn load_projects(storage: &crate::storage::sqlite::SqliteStorage) -> Vec<ProjectState> {
        let Ok(projects) = storage.list_projects() else {
            return Vec::new();
        };

        projects
            .into_iter()
            .map(|p| {
                let pending = storage
                    .list_tasks_by_project(&p.id)
                    .map(|tasks| {
                        tasks
                            .iter()
                            .filter(|t| t.status == crate::models::task::TaskStatus::Pending)
                            .count()
                    })
                    .unwrap_or(0);

                ProjectState {
                    id: p.id.clone(),
                    name: p.name.clone(),
                    status: crate::tui::domain::ProjectStatus::Healthy,
                    unread_count: pending,
                    pending_actions: 0,
                    default_backend: p.default_backend.clone(),
                    last_activity: None,
                    budget_percent: 0.0,
                }
            })
            .collect()
    }

    fn load_actions(storage: &crate::storage::sqlite::SqliteStorage) -> Vec<ActionItem> {
        let Ok(actions) = storage.list_pending_actions() else {
            return Vec::new();
        };

        let projects: HashMap<_, _> = storage
            .list_projects()
            .unwrap_or_default()
            .into_iter()
            .map(|p| (p.id.clone(), p.name.clone()))
            .collect();

        actions
            .into_iter()
            .map(|a| ActionItem {
                id: a.id.clone(),
                project_id: a.project_id.clone(),
                project_name: projects.get(&a.project_id).cloned().unwrap_or_default(),
                kind: match a.action_type {
                    crate::models::policy::PendingActionType::ReviewRequest => {
                        crate::tui::domain::ActionKind::ReviewRequest
                    }
                    crate::models::policy::PendingActionType::CommitSuggestion => {
                        crate::tui::domain::ActionKind::CommitSuggestion
                    }
                    crate::models::policy::PendingActionType::BudgetApproval => {
                        crate::tui::domain::ActionKind::BudgetApproval
                    }
                    crate::models::policy::PendingActionType::BackendOverride => {
                        crate::tui::domain::ActionKind::Approval
                    }
                },
                priority: crate::models::Priority::Normal,
                summary: a.description,
                created_at: a.created_at,
                requires_user_decision: true,
                resolved: a.status != crate::models::policy::ActionStatus::Pending,
            })
            .collect()
    }

    fn load_budget(storage: &crate::storage::sqlite::SqliteStorage) -> BudgetState {
        let year_month = chrono::Utc::now().format("%Y-%m").to_string();

        let global_spent = storage
            .spend_by_month(1)
            .ok()
            .and_then(|v| v.into_iter().find(|(ym, _)| ym == &year_month))
            .map(|(_, m)| m.cents as f32)
            .unwrap_or(0.0);

        BudgetState {
            global_percent_used: global_spent / 100_000.0,
            backend_percent_used: HashMap::new(),
            mode: crate::budget::governor::BudgetMode::Govern,
            warnings: Vec::new(),
            daily_burn_rate: None,
            projected_eom: None,
        }
    }

    fn load_events(storage: &crate::storage::sqlite::SqliteStorage) -> Vec<EventRecord> {
        let Ok(events) = storage.list_events_recent(100) else {
            return Vec::new();
        };

        events
            .into_iter()
            .map(|e| EventRecord {
                id: e.id.0,
                event_type: format!("{:?}", e.event_type),
                timestamp: e.timestamp,
                payload: e.payload.to_string(),
            })
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct EventRecord {
    pub id: uuid::Uuid,
    pub event_type: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub payload: String,
}

#[derive(Debug, Clone, Default)]
pub struct ComposerState {
    pub input: String,
    pub cursor_position: usize,
    pub history: Vec<String>,
    pub history_index: Option<usize>,
}

#[derive(Debug, Clone, Default)]
pub struct ChatsViewState {
    pub selected_project_index: usize,
    pub selected_filter_index: usize,
    pub selected_feed_index: usize,
    pub feed_scroll_offset: usize,
    pub project_scope: Option<crate::models::ProjectId>,
    pub active_filter: FeedFilter,
}

#[derive(Debug, Clone, Default)]
pub struct ProjectsViewState {
    pub selected_index: usize,
    pub scroll_offset: usize,
}

#[derive(Debug, Clone, Default)]
pub struct QueueViewState {
    pub selected_index: usize,
    pub selected_filter: QueueFilter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum QueueFilter {
    #[default]
    All,
    Review,
    Commit,
    Blocker,
    Budget,
}

#[derive(Debug, Clone, Default)]
pub struct BudgetViewState {
    pub scroll_offset: usize,
}

#[derive(Debug, Clone, Default)]
pub struct EventsViewState {
    pub scroll_offset: usize,
    pub filter: Option<String>,
}
