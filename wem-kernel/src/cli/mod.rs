pub mod app;
pub mod ui;

use std::sync::Arc;

use crate::agent::runtime::AgentRuntime;

pub async fn run(runtime: Arc<AgentRuntime>, model: String) -> Result<(), Box<dyn std::error::Error>> {
    crossterm::terminal::enable_raw_mode()?;
    let stdout = std::io::stdout();
    let backend = ratatui::backend::CrosstermBackend::new(stdout);
    let mut terminal = ratatui::Terminal::new(backend)?;
    crossterm::execute!(std::io::stdout(), crossterm::terminal::EnterAlternateScreen)?;
    terminal.clear()?;

    let mut app = app::App::new(runtime, model);

    let (key_tx, mut key_rx) = tokio::sync::mpsc::channel::<crossterm::event::KeyEvent>(100);
    tokio::task::spawn_blocking(move || {
        loop {
            if crossterm::event::poll(std::time::Duration::from_millis(50)).unwrap() {
                if let crossterm::event::Event::Key(key) = crossterm::event::read().unwrap() {
                    if key.kind == crossterm::event::KeyEventKind::Press {
                        let _ = key_tx.blocking_send(key);
                    }
                }
            }
        }
    });

    let tick = tokio::time::interval(std::time::Duration::from_millis(200));
    tokio::pin!(tick);

    while app.running {
        terminal.draw(|f| ui::render(f, &app))?;

        tokio::select! {
            Some(key) = key_rx.recv() => {
                app.handle_key(key);
            }
            _ = tick.tick() => {
                app.poll_agent_events();
                if app.pending_message.is_some() {
                    app.send_pending_message().await;
                }
            }
        }
    }

    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(std::io::stdout(), crossterm::terminal::LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}
