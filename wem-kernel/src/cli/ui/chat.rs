use ratatui::{
    Frame,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph, Wrap},
};
use ratatui::layout::Rect;

use crate::cli::app::{App, ChatEntry, ToolCallStatus};
use super::markdown;

pub fn render(f: &mut Frame, app: &App) {
    let area = f.area();
    let chat_area = Rect {
        x: area.x,
        y: area.y + 1,
        width: area.width,
        height: area.height.saturating_sub(4),
    };

    let mut lines: Vec<Line> = Vec::new();

    for entry in &app.entries {
        match entry {
            ChatEntry::UserMessage { text } => {
                lines.push(Line::from(vec![
                    Span::styled(" you", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                    Span::styled("> ", Style::default().fg(Color::Green)),
                ]));
                for line in text.lines() {
                    lines.push(Line::from(Span::styled(format!("   {}", line), Style::default().fg(Color::White))));
                }
                lines.push(Line::raw(""));
            }
            ChatEntry::AssistantText { text } => {
                let md_lines = markdown::render_lines(text);
                for line in md_lines { lines.push(line); }
                lines.push(Line::raw(""));
            }
            ChatEntry::SystemInfo { text } => {
                lines.push(Line::from(vec![
                    Span::styled(" ── ", Style::default().fg(Color::DarkGray)),
                    Span::styled(text.clone(), Style::default().fg(Color::DarkGray)),
                ]));
                lines.push(Line::raw(""));
            }
            ChatEntry::Error { text } => {
                lines.push(Line::from(vec![
                    Span::styled(" ! ", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
                    Span::styled(text.clone(), Style::default().fg(Color::Red)),
                ]));
                lines.push(Line::raw(""));
            }
            ChatEntry::ToolCard { name, args_summary, status, result } => {
                let (icon, fg) = match status {
                    ToolCallStatus::Running => ("...", Color::Yellow),
                    ToolCallStatus::Done => (" ok", Color::Green),
                    ToolCallStatus::Error => (" err", Color::Red),
                };
                lines.push(Line::from(vec![
                    Span::styled("  ", Style::default()),
                    Span::styled(format!("[{}] {} ", icon, name), Style::default().fg(fg).add_modifier(Modifier::BOLD)),
                    Span::styled(truncate_str(args_summary, 80), Style::default().fg(Color::DarkGray)),
                ]));
                if let Some(res) = result {
                    let res_style = match status {
                        ToolCallStatus::Error => Style::default().fg(Color::Red),
                        _ => Style::default().fg(Color::DarkGray),
                    };
                    for res_line in truncate_str(res, 200).lines() {
                        lines.push(Line::from(vec![
                            Span::styled("    │ ", Style::default().fg(Color::DarkGray)),
                            Span::styled(res_line.to_string(), res_style),
                        ]));
                    }
                }
            }
        }
    }

    if !app.streaming_text.is_empty() {
        for line in markdown::render_lines(&app.streaming_text) { lines.push(line); }
        lines.push(Line::from(Span::styled(" ▍", Style::default().fg(Color::Cyan))));
    }

    if app.entries.is_empty() && app.streaming_text.is_empty() {
        lines.push(Line::raw(""));
        lines.push(Line::from(Span::styled("  wem agent", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))));
        lines.push(Line::from(Span::styled("  Type a message or /help for commands.", Style::default().fg(Color::DarkGray))));
    }

    let para = Paragraph::new(lines).block(Block::default()).wrap(Wrap { trim: false });
    let scroll = if app.auto_scroll {
        0
    } else {
        app.scroll_offset
    };
    f.render_widget(para.scroll((scroll, 0)), chat_area);
}

fn truncate_str(s: &str, max: usize) -> String {
    if s.len() > max {
        format!("{}...", &s[..s.floor_char_boundary(max)])
    } else {
        s.to_string()
    }
}
