mod app;
mod domain;
mod event;
mod feed;
mod state;
mod terminal;
mod update;
mod views;
mod widgets;

pub use app::run_tui;

pub mod types {
    pub use super::domain::*;
    pub use super::feed::*;
}

const SIDEBAR_WIDTH: u16 = 30;
const MIN_TERMINAL_WIDTH: u16 = 100;
const MIN_TERMINAL_HEIGHT: u16 = 24;
const HEADER_HEIGHT: u16 = 1;
const COMPOSER_HEIGHT: u16 = 3;
const FOOTER_HEIGHT: u16 = 1;