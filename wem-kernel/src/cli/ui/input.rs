use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::cli::app::{App, AppPhase};

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let prompt = if app.phase == AppPhase::Idle {
        Span::styled(" > ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))
    } else {
        Span::styled(" ... ", Style::default().fg(Color::DarkGray))
    };

    let input_text = Span::styled(&app.input, Style::default().fg(Color::White));
    let line = Line::from(vec![prompt, input_text]);

    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(Color::DarkGray));

    let para = Paragraph::new(line).block(block);
    f.render_widget(para, area);

    if app.phase == AppPhase::Idle {
        f.set_cursor_position((area.x + 3 + app.cursor as u16, area.y + 1));
    }
}
