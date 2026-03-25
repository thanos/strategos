use crate::models::{ActionId, BackendId, EventId, ProjectId, TaskId};
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedItemKind {
    Update,
    Question,
    ReviewRequest,
    CommitRequest,
    Blocker,
    PlanProposal,
    Error,
    BudgetWarning,
    RoutingNotice,
    Completion,
    UserResponse,
}

impl FeedItemKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            FeedItemKind::Update => "UPDATE",
            FeedItemKind::Question => "QUESTION",
            FeedItemKind::ReviewRequest => "REVIEW",
            FeedItemKind::CommitRequest => "COMMIT",
            FeedItemKind::Blocker => "BLOCK",
            FeedItemKind::PlanProposal => "PLAN",
            FeedItemKind::Error => "ERROR",
            FeedItemKind::BudgetWarning => "BUDGET",
            FeedItemKind::RoutingNotice => "ROUTING",
            FeedItemKind::Completion => "DONE",
            FeedItemKind::UserResponse => "USER",
        }
    }
}

#[derive(Debug, Clone)]
pub struct FeedItem {
    pub id: FeedItemId,
    pub project_id: ProjectId,
    pub project_name: String,
    pub kind: FeedItemKind,
    pub summary: String,
    pub detail: String,
    pub source_backend: Option<BackendId>,
    pub created_at: DateTime<Utc>,
    pub requires_response: bool,
    pub resolved: bool,
    pub unread: bool,
    pub suggested_actions: Vec<SuggestedAction>,
    pub linked_action_id: Option<ActionId>,
    pub linked_event_ids: Vec<EventId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FeedItemId(pub uuid::Uuid);

impl FeedItemId {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }
}

impl Default for FeedItemId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for FeedItemId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", &self.0.to_string()[..8])
    }
}

#[derive(Debug, Clone)]
pub struct SuggestedAction {
    pub label: String,
    pub action: SuggestedActionType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuggestedActionType {
    Approve,
    Defer,
    Resolve,
    Retry,
    Review,
    Commit,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum FeedFilter {
    #[default]
    All,
    NeedsReply,
    Review,
    Commit,
    Blocked,
    Budget,
    Unread,
    Project(ProjectId),
}

impl FeedFilter {
    pub fn as_str(&self) -> &'static str {
        match self {
            FeedFilter::All => "all",
            FeedFilter::NeedsReply => "needs_reply",
            FeedFilter::Review => "review",
            FeedFilter::Commit => "commit",
            FeedFilter::Blocked => "blocked",
            FeedFilter::Budget => "budget",
            FeedFilter::Unread => "unread",
            FeedFilter::Project(id) => "project",
        }
    }

    pub fn matches(&self, item: &FeedItem) -> bool {
        match self {
            FeedFilter::All => true,
            FeedFilter::NeedsReply => item.requires_response && !item.resolved,
            FeedFilter::Review => item.kind == FeedItemKind::ReviewRequest,
            FeedFilter::Commit => item.kind == FeedItemKind::CommitRequest,
            FeedFilter::Blocked => item.kind == FeedItemKind::Blocker && !item.resolved,
            FeedFilter::Budget => {
                item.kind == FeedItemKind::BudgetWarning || item.kind == FeedItemKind::RoutingNotice
            }
            FeedFilter::Unread => item.unread,
            FeedFilter::Project(id) => item.project_id == *id,
        }
    }
}

pub fn feed_items_from_storage(storage: &crate::storage::sqlite::SqliteStorage) -> Vec<FeedItem> {
    let mut items = Vec::new();

    let Ok(projects) = storage.list_projects() else {
        return items;
    };

    let Ok(actions) = storage.list_pending_actions() else {
        return items;
    };

    for action in actions {
        let project = projects.iter().find(|p| p.id == action.project_id);
        let kind = match action.action_type {
            crate::models::policy::PendingActionType::ReviewRequest => FeedItemKind::ReviewRequest,
            crate::models::policy::PendingActionType::CommitSuggestion => {
                FeedItemKind::CommitRequest
            }
            crate::models::policy::PendingActionType::BudgetApproval => FeedItemKind::BudgetWarning,
            crate::models::policy::PendingActionType::BackendOverride => {
                FeedItemKind::RoutingNotice
            }
        };

        items.push(FeedItem {
            id: FeedItemId::new(),
            project_id: action.project_id.clone(),
            project_name: project.map(|p| p.name.clone()).unwrap_or_default(),
            kind,
            summary: action.description.clone(),
            detail: action.payload.to_string(),
            source_backend: None,
            created_at: action.created_at,
            requires_response: true,
            resolved: action.status != crate::models::policy::ActionStatus::Pending,
            unread: action.status == crate::models::policy::ActionStatus::Pending,
            suggested_actions: vec![
                SuggestedAction {
                    label: "Approve".into(),
                    action: SuggestedActionType::Approve,
                },
                SuggestedAction {
                    label: "Dismiss".into(),
                    action: SuggestedActionType::Defer,
                },
            ],
            linked_action_id: Some(action.id.clone()),
            linked_event_ids: Vec::new(),
        });
    }

    items.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    items
}
