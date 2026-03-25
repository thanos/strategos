use crossterm::event::{Event, EventStream, KeyEvent, MouseEvent};
use futures::StreamExt;

use crate::models::{ActionId, ProjectId};
use crate::tui::feed::FeedItemId;

use super::domain::TopLevelTab;

#[derive(Debug, Clone)]
pub enum UiEvent {
    Key(KeyEvent),
    Mouse(MouseEvent),
    Tick,
    Resize(u16, u16),
    FeedUpdated,
    ProjectUpdated(ProjectId),
    ActionQueued(ActionId),
    ActionResolved(ActionId),
    BudgetUpdated,
    RoutingDecisionLogged,
    ErrorOccurred(String),
    ClearError,
}

#[derive(Debug, Clone)]
pub enum Effect {
    SubmitTask { project: ProjectId, description: String },
    ApproveAction { action_id: ActionId },
    DeferAction { action_id: ActionId },
    ResolveFeedItem { item_id: FeedItemId },
    SwitchTab(TopLevelTab),
    FocusProject(ProjectId),
    SetFilter(crate::tui::feed::FeedFilter),
    RefreshState,
    ShowError(String),
    ClearError,
    Quit,
}

pub enum EventResult {
    Continue,
    Quit,
}

pub async fn collect_events(tx: tokio::sync::mpsc::Sender<UiEvent>, tick_rate: std::time::Duration) {
    let mut reader = EventStream::new();
    let mut interval = tokio::time::interval(tick_rate);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            _ = interval.tick() => {
                if tx.send(UiEvent::Tick).await.is_err() {
                    break;
                }
            }
            maybe_event = reader.next() => {
                match maybe_event {
                    Some(Ok(event)) => {
                        let ui_event = match event {
                            Event::Key(key) => UiEvent::Key(key),
                            Event::Mouse(mouse) => UiEvent::Mouse(mouse),
                            Event::Resize(cols, rows) => UiEvent::Resize(cols, rows),
                            _ => continue,
                        };
                        if tx.send(ui_event).await.is_err() {
                            break;
                        }
                    }
                    Some(Err(_)) | None => {
                        break;
                    }
                }
            }
        }
    }
}