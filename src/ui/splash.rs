//! Splash screen with the pu-get wordmark.

use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use ratatui::Frame;

use crate::app::App;
use crate::ui::{self, theme::Theme};

/// Block-letter wordmark. Kept ASCII so it renders in every terminal font.
pub const LOGO: &str = r" ____   _   _            ____   _____   _____
|  _ \ | | | |          / ___| | ____| |_   _|
| |_) | | | | |   ___   | |  _  |  _|     | |
|  __/  | |_| |  |___|  | |_| | | |___    | |
|_|      \___/           \____| |_____|   |_|";

/// Draw the splash screen.
pub fn draw(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let rect = ui::centered_rect(68, 14, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme.border_style())
        .title(Line::from(vec![
            Span::styled(" pu-get ", theme.brand_style()),
            Span::styled(
                format!("v{} ", env!("CARGO_PKG_VERSION")),
                theme.dim_style(),
            ),
        ]));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    let chunks = Layout::vertical([
        Constraint::Length(7),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .split(inner);

    // Centre the logo as one block. Centring each line independently would
    // shift the shorter rows and break the ASCII art.
    let logo_width = LOGO
        .lines()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(0);
    let logo_area = ui::centered_rect(u16::try_from(logo_width).unwrap_or(u16::MAX), 5, chunks[0]);
    frame.render_widget(
        Paragraph::new(LOGO)
            .alignment(Alignment::Left)
            .style(theme.brand_style()),
        logo_area,
    );

    frame.render_widget(
        Paragraph::new("first public pre-release")
            .alignment(Alignment::Center)
            .style(theme.dim_style()),
        chunks[1],
    );

    frame.render_widget(
        Paragraph::new(format!(
            "{} boards · {} catalogued PDFs",
            app.boards().len(),
            app.catalog.len()
        ))
        .alignment(Alignment::Center)
        .style(theme.text_style()),
        chunks[2],
    );

    frame.render_widget(
        Paragraph::new("Press any key to start")
            .alignment(Alignment::Center)
            .style(theme.key_style()),
        chunks[3],
    );

    frame.render_widget(
        Paragraph::new("Official study material, straight from the source.")
            .alignment(Alignment::Center)
            .style(theme.dim_style()),
        chunks[4],
    );
}
