use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

pub fn render_help(f: &mut Frame, _area: Rect) {
    let area = centered_rect(60, 70, f.area());

    let help_text = r#"
Global Keys:
  1-5      Switch tab (Chats, Projects, Queue, Budget, Events)
  Tab      Cycle focus forward
  Shift+Tab  Cycle focus backward
  q        Quit
  ?        Toggle this help

Navigation:
  j/k      Move down/up
  h/l      Move left/right

Actions:
  Enter    Open/expand item
  a        Approve
  d        Defer
  x        Resolve

Composer:
  i        Focus composer
  Enter    Send message
  Esc      Clear/close

Press ? to close help
"#;

    let block = Block::default()
        .title("Help")
        .borders(Borders::ALL)
        .style(Style::default().add_modifier(Modifier::BOLD));

    let paragraph = Paragraph::new(help_text).block(block);

    f.render_widget(Clear, area);
    f.render_widget(paragraph, area);
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
