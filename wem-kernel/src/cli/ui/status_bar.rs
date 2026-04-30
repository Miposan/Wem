use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::cli::app::{App, AppPhase};

pub fn render(f: &mut Frame, area: Rect, app: &App) {
    let phase_style = match &app.phase {
        AppPhase::Idle => Style::default().fg(Color::DarkGray),
        AppPhase::Thinking => Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        AppPhase::Streaming => Style::default().fg(Color::Green),
        AppPhase::ExecutingTools => Style::default().fg(Color::Cyan),
        AppPhase::WaitingPermission => Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
    };

    let phase_label = if app.phase != AppPhase::Idle {
        let spinner = match app.phase {
            AppPhase::Thinking => "⠋",
            AppPhase::Streaming => "⠙",
            AppPhase::ExecutingTools => "⠹",
            AppPhase::WaitingPermission => "!",
            _ => "",
        };
        format!(" {} {} ", spinner, app.phase.label())
    } else {
        String::new()
    };

    let step_info = if app.max_steps > 0 && app.phase != AppPhase::Idle {
        format!(" {}/{} ", app.step + 1, app.max_steps)
    } else {
        String::new()
    };

    let token_info = if app.total_input_tokens > 0 || app.total_output_tokens > 0 {
        format!(" {}↓ {}↑ ", app.total_input_tokens, app.total_output_tokens)
    } else {
        String::new()
    };

    let line = Line::from(vec![
        Span::styled(" wem ", Style::default().fg(Color::White).bg(Color::DarkGray)),
        Span::styled(format!(" {} ", app.model), Style::default().fg(Color::Cyan)),
        Span::styled(phase_label, phase_style),
        Span::styled(step_info, Style::default().fg(Color::DarkGray)),
        Span::styled(token_info, Style::default().fg(Color::DarkGray)),
        Span::styled(" Shift↑↓ scroll  Ctrl+C cancel ", Style::default().fg(Color::DarkGray)),
    ]);

    f.render_widget(Paragraph::new(line), area);
}
