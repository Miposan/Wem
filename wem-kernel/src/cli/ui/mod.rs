pub mod markdown;
pub mod status_bar;
pub mod chat;
pub mod input;

use ratatui::{Frame, layout::Rect};
use super::app::App;

pub fn render(f: &mut Frame, app: &App) {
    let area = f.area();
    let status_area = Rect { x: area.x, y: area.y, width: area.width, height: 1 };
    let input_area = Rect {
        x: area.x,
        y: area.y + area.height.saturating_sub(3),
        width: area.width,
        height: 3,
    };

    status_bar::render(f, status_area, app);
    chat::render(f, app);
    input::render(f, input_area, app);
}
