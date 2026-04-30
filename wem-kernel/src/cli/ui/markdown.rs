use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use pulldown_cmark::{Event, Tag, TagEnd, Options, Parser};

pub fn render_lines(text: &str) -> Vec<Line<'static>> {
    let mut lines: Vec<Line> = Vec::new();
    let mut spans: Vec<Span> = Vec::new();
    let mut current_style = Style::default();

    let parser = Parser::new_ext(text, Options::empty());

    for event in parser {
        match event {
            Event::Start(tag) => match tag {
                Tag::Heading { level, .. } => {
                    current_style = Style::default()
                        .add_modifier(Modifier::BOLD)
                        .fg(match level {
                            pulldown_cmark::HeadingLevel::H1 => Color::Cyan,
                            pulldown_cmark::HeadingLevel::H2 => Color::Cyan,
                            _ => Color::White,
                        });
                }
                Tag::Strong => { current_style = current_style.add_modifier(Modifier::BOLD); }
                Tag::Emphasis => { current_style = current_style.add_modifier(Modifier::ITALIC); }
                Tag::CodeBlock(_) => { current_style = Style::default().fg(Color::Yellow); }
                Tag::BlockQuote(_) => {
                    current_style = Style::default().fg(Color::DarkGray);
                    spans.push(Span::styled("│ ", Style::default().fg(Color::DarkGray)));
                }
                Tag::Item => { spans.push(Span::styled("  • ", Style::default().fg(Color::Cyan))); }
                Tag::Link { .. } => { current_style = Style::default().fg(Color::Cyan).add_modifier(Modifier::UNDERLINED); }
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Heading(_) | TagEnd::Paragraph | TagEnd::CodeBlock => {
                    if !spans.is_empty() { lines.push(Line::from(std::mem::take(&mut spans))); }
                    current_style = Style::default();
                }
                TagEnd::Strong => { current_style = current_style.remove_modifier(Modifier::BOLD); }
                TagEnd::Emphasis => { current_style = current_style.remove_modifier(Modifier::ITALIC); }
                TagEnd::BlockQuote | TagEnd::Link => { current_style = Style::default(); }
                TagEnd::Item => { if !spans.is_empty() { lines.push(Line::from(std::mem::take(&mut spans))); } }
                _ => {}
            },
            Event::Text(t) => {
                if t.contains('\n') {
                    let parts: Vec<&str> = t.split('\n').collect();
                    for (i, part) in parts.iter().enumerate() {
                        if !part.is_empty() { spans.push(Span::styled(part.to_string(), current_style)); }
                        if i < parts.len() - 1 { lines.push(Line::from(std::mem::take(&mut spans))); }
                    }
                } else {
                    spans.push(Span::styled(t.into_string(), current_style));
                }
            }
            Event::Code(c) => { spans.push(Span::styled(c.into_string(), Style::default().fg(Color::Yellow))); }
            Event::SoftBreak | Event::HardBreak => { if !spans.is_empty() { lines.push(Line::from(std::mem::take(&mut spans))); } }
            _ => {}
        }
    }
    if !spans.is_empty() { lines.push(Line::from(spans)); }
    lines
}
