//! Regenerate the README screenshots from the real renderer.
//!
//! Run from the repository root:
//!
//! ```sh
//! cargo run --example screenshots
//! ```
//!
//! The output is written as SVG under `docs/screenshots/`. No network access is
//! required: the embedded catalogs are rendered through a ratatui `TestBackend`.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use pu_get::app::{App, Screen};
use pu_get::catalog::{Catalog, MaterialType};
use pu_get::config::{AppPaths, Config};
use pu_get::download::{DownloadEvent, JobId, ProgressSnapshot};
use pu_get::error::DownloadError;
use pu_get::ui;
use ratatui::backend::TestBackend;
use ratatui::buffer::{Buffer, Cell};
use ratatui::style::{Color, Modifier};
use ratatui::Terminal;

const WIDTH: u16 = 100;
const HEIGHT: u16 = 30;
const CELL_WIDTH: f64 = 8.4;
const ROW_HEIGHT: f64 = 18.0;
const PADDING: f64 = 20.0;
const HEADER_HEIGHT: f64 = 42.0;
const DEFAULT_BG: &str = "#0f172a";
const DEFAULT_FG: &str = "#e2e8f0";
const FONT: &str = "'DejaVu Sans Mono', 'Liberation Mono', ui-monospace, SFMono-Regular, Menlo, \
                    Consolas, monospace";

fn main() -> Result<()> {
    let catalog = Catalog::load_embedded().context("could not load the embedded catalogs")?;
    fs::create_dir_all("docs/screenshots").context("could not create docs/screenshots")?;

    let paths = AppPaths::new(
        std::env::temp_dir().join("pu-get-screenshots/config.toml"),
        std::env::temp_dir().join("pu-get-screenshots/cache"),
    );

    let splash = App::new(catalog.clone(), Config::default(), paths.clone(), true);
    save(&splash, "splash", "splash")?;

    let mut browse = App::new(catalog.clone(), Config::default(), paths.clone(), true);
    browse.leave_splash();
    browse.selection.board = Some("cbse".to_owned());
    browse.selection.level = Some("12".to_owned());
    browse.selection.subject = Some("Chemistry".to_owned());
    browse.selection.kind = Some(MaterialType::SamplePaper);
    browse.latest_only = false;
    browse.search = "chemistry".to_owned();
    browse.screen = Screen::Browse;
    browse.browse_cursor = 2;

    let urls: Vec<String> = browse
        .filtered()
        .iter()
        .take(4)
        .map(|item| item.entry.url.clone())
        .collect();
    anyhow::ensure!(
        urls.len() >= 4,
        "the embedded CBSE catalog is too small for the browse screenshot"
    );
    for url in [urls[0].clone(), urls[3].clone()] {
        browse.selected.insert(url);
    }
    save(&browse, "browse", "browse")?;

    let mut board = App::new(catalog.clone(), Config::default(), paths.clone(), true);
    board.leave_splash();

    let mut settings = App::new(catalog.clone(), Config::default(), paths.clone(), true);
    settings.leave_splash();
    settings.screen = Screen::Settings;

    let mut help = App::new(catalog.clone(), Config::default(), paths.clone(), true);
    help.leave_splash();
    help.open_help();

    let mut queue = App::new(catalog, Config::default(), paths, true);
    queue.leave_splash();
    queue.screen = Screen::Queue;

    let jobs = [
        "NCERT Physics Part I - Chapter 1",
        "CBSE Chemistry Sample Paper 2026-27",
        "KSEEB Mathematics Question Paper 2022",
        "PUE Physics Textbook (English)",
        "CBSE Chemistry Marking Scheme 2025-26",
    ];
    let ids = [JobId(1), JobId(2), JobId(3), JobId(4), JobId(5)];
    for (id, title) in ids.iter().zip(jobs) {
        queue.handle_download_event(DownloadEvent::Queued {
            id: *id,
            title: title.to_owned(),
        });
    }

    queue.handle_download_event(DownloadEvent::Retrying {
        id: ids[1],
        attempt: 2,
        max_attempts: 4,
        delay: Duration::from_secs(1),
        error: "request timed out after 60 second(s)".to_owned(),
    });

    queue.handle_download_event(DownloadEvent::Started {
        id: ids[0],
        destination: PathBuf::from("/home/student/Downloads/physics-1.pdf"),
        resumed_from: 0,
        total: Some(8_900_000),
    });
    queue.handle_download_event(DownloadEvent::Progress {
        id: ids[0],
        progress: ProgressSnapshot {
            downloaded: 5_600_000,
            total: Some(8_900_000),
            bytes_per_second: 1_310_000.0,
            eta: Some(Duration::from_secs(3)),
        },
    });

    queue.handle_download_event(DownloadEvent::Started {
        id: ids[2],
        destination: PathBuf::from("/home/student/Downloads/maths-2022.pdf"),
        resumed_from: 2_200_000,
        total: Some(7_100_000),
    });
    queue.handle_download_event(DownloadEvent::Paused {
        id: ids[2],
        downloaded: 2_200_000,
    });

    queue.handle_download_event(DownloadEvent::Completed {
        id: ids[3],
        path: PathBuf::from("/home/student/Downloads/physics-textbook.pdf"),
        bytes: 4_100_000,
    });

    queue.handle_download_event(DownloadEvent::Failed {
        id: ids[4],
        error: DownloadError::Timeout { seconds: 60 },
    });

    // Do not let the simulated failure tint the status bar red; the queue row
    // already communicates it.
    queue.status.clear();
    queue.status_error = false;

    save(&queue, "queue", "downloads")?;
    save(&board, "board", "boards")?;
    save(&settings, "settings", "settings")?;
    save(&help, "help", "help")?;
    Ok(())
}

fn save(app: &App, slug: &str, title: &str) -> Result<()> {
    let svg = render(app, title)?;
    let path = Path::new("docs/screenshots").join(format!("{slug}.svg"));
    fs::write(&path, svg).with_context(|| format!("could not write {}", path.display()))?;
    println!("wrote {}", path.display());
    Ok(())
}

fn render(app: &App, title: &str) -> Result<String> {
    let backend = TestBackend::new(WIDTH, HEIGHT);
    let mut terminal = Terminal::new(backend).context("could not create the test terminal")?;
    terminal
        .draw(|frame| ui::draw(frame, app))
        .context("could not render the screenshot")?;
    Ok(svg_from_buffer(terminal.backend().buffer(), title))
}

fn svg_from_buffer(buffer: &Buffer, title: &str) -> String {
    let columns = usize::from(WIDTH);
    let rows = usize::from(HEIGHT);
    let width = f64::from(WIDTH) * CELL_WIDTH + PADDING * 2.0;
    let height = HEADER_HEIGHT + f64::from(HEIGHT) * ROW_HEIGHT + PADDING;

    let mut svg = String::new();
    let _ = writeln!(
        svg,
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width:.0}\" height=\"{height:.0}\" \
         viewBox=\"0 0 {width:.0} {height:.0}\" role=\"img\" aria-labelledby=\"title\" \
         xml:space=\"preserve\">"
    );
    let _ = writeln!(
        svg,
        "  <title id=\"title\">pu-get - {}</title>",
        escape(title)
    );
    let _ = writeln!(
        svg,
        "  <rect width=\"100%\" height=\"100%\" rx=\"12\" fill=\"#0b1020\"/>"
    );
    let _ = writeln!(
        svg,
        "  <circle cx=\"29\" cy=\"20\" r=\"5\" fill=\"#ff5f56\"/>\
         <circle cx=\"45\" cy=\"20\" r=\"5\" fill=\"#ffbd2e\"/>\
         <circle cx=\"61\" cy=\"20\" r=\"5\" fill=\"#27c93f\"/>"
    );
    let _ = writeln!(
        svg,
        "  <text x=\"78\" y=\"24\" fill=\"#94a3b8\" font-family=\"{FONT}\" font-size=\"12\">\
         pu-get - {}</text>",
        escape(title)
    );
    let _ = writeln!(
        svg,
        "  <text x=\"{:.0}\" y=\"24\" text-anchor=\"end\" fill=\"#475569\" \
         font-family=\"{FONT}\" font-size=\"11\">terminal</text>",
        width - PADDING
    );
    let _ = writeln!(
        svg,
        "  <rect x=\"{PADDING:.0}\" y=\"{HEADER_HEIGHT:.0}\" width=\"{:.1}\" height=\"{:.1}\" \
         rx=\"6\" fill=\"{DEFAULT_BG}\"/>",
        f64::from(WIDTH) * CELL_WIDTH,
        f64::from(HEIGHT) * ROW_HEIGHT
    );

    // Background runs, row by row.
    for row in 0..rows {
        let mut column = 0;
        while column < columns {
            let background = effective_colors(cell_at(buffer, column, row)).1;
            let start = column;
            column += 1;
            while column < columns && effective_colors(cell_at(buffer, column, row)).1 == background
            {
                column += 1;
            }

            if background != DEFAULT_BG {
                let _ = writeln!(
                    svg,
                    "  <rect x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{ROW_HEIGHT:.1}\" fill=\"{background}\"/>",
                    PADDING + start as f64 * CELL_WIDTH,
                    HEADER_HEIGHT + row as f64 * ROW_HEIGHT,
                    (column - start) as f64 * CELL_WIDTH
                );
            }
        }
    }

    // Text runs. Spaces are skipped entirely and every run carries an exact x
    // coordinate, so SVG whitespace collapsing cannot shift the layout.
    for row in 0..rows {
        let baseline = HEADER_HEIGHT + row as f64 * ROW_HEIGHT + 14.0;
        let mut column = 0;
        while column < columns {
            let cell = cell_at(buffer, column, row);
            if cell.symbol().trim().is_empty() {
                column += 1;
                continue;
            }

            let (foreground, _, bold) = effective_colors(cell);
            let start = column;
            let mut text = String::new();

            // Box-drawing glyphs are emitted one cell at a time: some SVG font
            // fallbacks give them a narrower advance than the cell, which would
            // leave visible gaps in a long border run.
            if is_box_drawing(cell.symbol()) {
                text.push_str(cell.symbol());
                column += 1;
            } else {
                while column < columns {
                    let cell = cell_at(buffer, column, row);
                    if cell.symbol().trim().is_empty() || is_box_drawing(cell.symbol()) {
                        break;
                    }
                    let (next_foreground, _, next_bold) = effective_colors(cell);
                    if next_foreground != foreground || next_bold != bold {
                        break;
                    }
                    text.push_str(cell.symbol());
                    column += 1;
                }
            }

            let _ = writeln!(
                svg,
                "  <text x=\"{:.1}\" y=\"{baseline:.1}\" fill=\"{foreground}\" \
                 font-family=\"{FONT}\" font-size=\"14\" font-weight=\"{}\">{}</text>",
                PADDING + start as f64 * CELL_WIDTH,
                if bold { "700" } else { "400" },
                escape(&text)
            );
        }
    }

    svg.push_str("</svg>\n");
    svg
}

fn cell_at(buffer: &Buffer, column: usize, row: usize) -> &Cell {
    &buffer.content[row * usize::from(WIDTH) + column]
}

fn effective_colors(cell: &Cell) -> (String, String, bool) {
    let mut foreground = css_color(cell.fg, DEFAULT_FG);
    let mut background = css_color(cell.bg, DEFAULT_BG);
    if cell.modifier.contains(Modifier::REVERSED) {
        std::mem::swap(&mut foreground, &mut background);
    }
    (
        foreground,
        background,
        cell.modifier.contains(Modifier::BOLD),
    )
}

fn css_color(color: Color, fallback: &'static str) -> String {
    let value = match color {
        Color::Reset => return fallback.to_owned(),
        Color::Black => "#020617",
        Color::Red => "#ef4444",
        Color::Green => "#22c55e",
        Color::Yellow => "#eab308",
        Color::Blue => "#3b82f6",
        Color::Magenta => "#d946ef",
        Color::Cyan => "#22d3ee",
        Color::Gray => "#94a3b8",
        Color::DarkGray => "#64748b",
        Color::LightRed => "#f87171",
        Color::LightGreen => "#4ade80",
        Color::LightYellow => "#fde047",
        Color::LightBlue => "#60a5fa",
        Color::LightMagenta => "#e879f9",
        Color::LightCyan => "#67e8f9",
        Color::White => "#f8fafc",
        Color::Rgb(red, green, blue) => return format!("#{red:02x}{green:02x}{blue:02x}"),
        Color::Indexed(_) => "#94a3b8",
    };
    value.to_owned()
}

fn is_box_drawing(symbol: &str) -> bool {
    symbol
        .chars()
        .next()
        .is_some_and(|character| ('\u{2500}'..='\u{257f}').contains(&character))
}

fn escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            _ => escaped.push(character),
        }
    }
    escaped
}
