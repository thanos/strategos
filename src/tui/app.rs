use crate::config::GlobalConfig;
use crate::tui::state::AppState;
use crate::tui::terminal::{self, PanicGuard};
use crate::tui::update::{render_app, update};

pub async fn run_tui(config: GlobalConfig) -> anyhow::Result<()> {
    let _guard = PanicGuard::new();

    let mut terminal = terminal::init()?;
    let storage = crate::storage::sqlite::SqliteStorage::open(&config.storage_path())?;

    let mut app_state = AppState::load_from_storage(&storage);
    let mut tick_count: u32 = 0;

    let (tx, mut rx) = tokio::sync::mpsc::channel(100);
    let tick_rate = std::time::Duration::from_millis(250);

    tokio::spawn(async move {
        crate::tui::event::collect_events(tx, tick_rate).await;
    });

    loop {
        if app_state.should_quit {
            break;
        }

        terminal.draw(|f| {
            render_app(f, &mut app_state);
        })?;

        match rx.recv().await {
            Some(event) => {
                let effects = update(&mut app_state, event, &mut tick_count);

                for effect in effects {
                    match effect {
                        crate::tui::event::Effect::Quit => {
                            app_state.should_quit = true;
                        }
                        crate::tui::event::Effect::RefreshState => {
                            app_state.refresh_from_storage(&storage);
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
            None => {
                break;
            }
        }
    }

    terminal::restore()?;
    Ok(())
}