use crate::models::{ActionId, BackendId, ProjectId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TopLevelTab {
    #[default]
    Chats,
    Projects,
    Queue,
    Budget,
    Events,
}

impl TopLevelTab {
    pub fn as_str(&self) -> &'static str {
        match self {
            TopLevelTab::Chats => "Chats",
            TopLevelTab::Projects => "Projects",
            TopLevelTab::Queue => "Queue",
            TopLevelTab::Budget => "Budget",
            TopLevelTab::Events => "Events",
        }
    }

    pub fn from_index(n: usize) -> Option<Self> {
        match n {
            0 => Some(TopLevelTab::Chats),
            1 => Some(TopLevelTab::Projects),
            2 => Some(TopLevelTab::Queue),
            3 => Some(TopLevelTab::Budget),
            4 => Some(TopLevelTab::Events),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UiMode {
    #[default]
    Normal,
    Input,
    Detail,
    Confirm,
}

impl UiMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            UiMode::Normal => "NORMAL",
            UiMode::Input => "INPUT",
            UiMode::Detail => "DETAIL",
            UiMode::Confirm => "CONFIRM",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FocusRegion {
    #[default]
    Feed,
    Tabs,
    Projects,
    Filters,
    Composer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectStatus {
    Healthy,
    NeedsAttention,
    AwaitingReview,
    ReadyToCommit,
    BlockedOnUser,
    BudgetConstrained,
    BackendUnavailable,
}

impl ProjectStatus {
    pub fn marker(&self) -> &'static str {
        match self {
            ProjectStatus::Healthy => "·",
            ProjectStatus::NeedsAttention => "!",
            ProjectStatus::AwaitingReview => "R",
            ProjectStatus::ReadyToCommit => "C",
            ProjectStatus::BlockedOnUser => "!",
            ProjectStatus::BudgetConstrained => "$",
            ProjectStatus::BackendUnavailable => "X",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProjectState {
    pub id: ProjectId,
    pub name: String,
    pub status: ProjectStatus,
    pub unread_count: usize,
    pub pending_actions: usize,
    pub default_backend: Option<BackendId>,
    pub last_activity: Option<chrono::DateTime<chrono::Utc>>,
    pub budget_percent: f32,
}

#[derive(Debug, Clone)]
pub struct ActionItem {
    pub id: ActionId,
    pub project_id: ProjectId,
    pub project_name: String,
    pub kind: ActionKind,
    pub priority: crate::models::Priority,
    pub summary: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub requires_user_decision: bool,
    pub resolved: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionKind {
    ReviewRequest,
    CommitSuggestion,
    BudgetApproval,
    Blocker,
    Approval,
}

impl ActionKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ActionKind::ReviewRequest => "REVIEW",
            ActionKind::CommitSuggestion => "COMMIT",
            ActionKind::BudgetApproval => "BUDGET",
            ActionKind::Blocker => "BLOCK",
            ActionKind::Approval => "APPROVAL",
        }
    }
}

#[derive(Debug, Clone)]
pub struct BudgetState {
    pub global_percent_used: f32,
    pub backend_percent_used: std::collections::HashMap<BackendId, f32>,
    pub mode: crate::budget::governor::BudgetMode,
    pub warnings: Vec<String>,
    pub daily_burn_rate: Option<crate::models::MoneyAmount>,
    pub projected_eom: Option<crate::models::MoneyAmount>,
}

impl Default for BudgetState {
    fn default() -> Self {
        Self {
            global_percent_used: 0.0,
            backend_percent_used: std::collections::HashMap::new(),
            mode: crate::budget::governor::BudgetMode::Govern,
            warnings: Vec::new(),
            daily_burn_rate: None,
            projected_eom: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RoutingState {
    pub recent_decisions: Vec<RoutingDecision>,
}

#[derive(Debug, Clone)]
pub struct RoutingDecision {
    pub task_id: crate::models::TaskId,
    pub project_id: ProjectId,
    pub selected_backend: BackendId,
    pub reason: String,
    pub fallback_applied: bool,
    pub budget_downgrade: bool,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

impl Default for RoutingState {
    fn default() -> Self {
        Self {
            recent_decisions: Vec::new(),
        }
    }
}
