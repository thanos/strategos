use crate::config::GlobalConfig;
use crate::tui::event::{collect_events, UiEvent};
use crate::tui::state::AppState;
use crate::tui::terminal;
use crate::tui::update::{render_app, update};

pub async fn run_tui(config: GlobalConfig) -> anyhow::Result<()> {
    let _guard = crate::tui::terminal::restore_on_panic();
    
    let mut terminal = terminal::init()?;
    let storage = crate::storage::sqlite::SqliteStorage::open(&config.storage_path())?;
    
    let mut app_state = AppState::load_from_storage(&storage);
    
    let (tx, mut rx) = tokio::sync::mpsc::channel(100);
    let tick_rate = std::time::Duration::from_millis(250);
    
    tokio::spawn(async move {
        collect_events(tx, tick_rate).await;
    });
    
    loop {
        if app_state.should_quit {
            break;
        }
        
        terminal.draw(|f| {
            render_app(f, &mut app_state);
        })?;
        
        if let Some(event) = rx.recv().await {
            let effects = update(&mut app_state, event);
            
            for effect in effects {
                match effect {
                    crate::tui::event::Effect::Quit => {
                        app_state.should_quit = true;
                    }
                    crate::tui::event::Effect::RefreshState => {
                        app_state = AppState::load_from_storage(&storage);
                    }
                    crate::tui::event::Effect::ShowError(msg) => {
                        app_state.error_message = Some(msg);
                    }
                    crate::tui::event::Effect::ClearError => {
                        app_state.error_message = None;
                    }
                    _ => {}
                }
            }
        }
    }
    
    terminal::restore()?;
    Ok(())
}