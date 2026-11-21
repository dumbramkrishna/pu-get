//! Settings screen and text-field editor popup.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::state::{SettingField, SettingKind};
use crate::app::App;
use crate::ui::{self, theme::Theme};

/// Draw the settings list.
pub fn draw(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let block = ui::block("Settings", theme);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height < 4 {
        return;
    }

    let layout = Layout::vertical([
        Constraint::Length(8),
        Constraint::Length(1),
        Constraint::Min(2),
        Constraint::Length(1),
    ])
    .split(inner);

    draw_fields(frame, layout[0], app, theme);
    ui::rule(frame, layout[1], theme);

    let field = app.settings_field();
    let controls = match field.kind() {
        SettingKind::Text => "Enter edits this value.",
        SettingKind::Cycle => "Enter cycles forward; Left/Right cycles either way.",
        SettingKind::Number => "Left/Right or Enter changes the value.",
        SettingKind::Toggle => "Enter or Space toggles this value.",
    };
    let help = format!("{}\n{controls}", field.help());
    frame.render_widget(
        Paragraph::new(help)
            .wrap(Wrap { trim: true })
            .style(theme.dim_style()),
        layout[2],
    );

    frame.render_widget(
        Paragraph::new(format!("Config  {}", app.paths.config_file.display()))
            .style(theme.dim_style()),
        layout[3],
    );

    if app.settings.editing {
        draw_editor(frame, area, app, field, theme);
    }
}

fn draw_fields(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    enum Row {
        Header(&'static str),
        Space,
        Field(usize),
    }

    let rows = [
        Row::Header("DOWNLOADS"),
        Row::Field(0),
        Row::Field(1),
        Row::Field(2),
        Row::Space,
        Row::Header("PREFERENCES"),
        Row::Field(3),
        Row::Field(4),
    ];

    let width = area.width as usize;
    let items: Vec<ListItem<'_>> = rows
        .into_iter()
        .map(|row| match row {
            Row::Header(label) => {
                ListItem::new(Line::from(Span::styled(label, theme.header_style())))
            }
            Row::Space => ListItem::new(Line::from("")),
            Row::Field(index) => {
                let field = SettingField::ALL[index];
                let selected = index == app.settings.cursor;
                let marker_style = if selected {
                    theme.selected_text_style()
                } else {
                    theme.accent_style()
                };
                let label_style = if selected {
                    theme.selected_text_style()
                } else {
                    theme.text_style()
                };
                let value_style = if selected {
                    theme.selected_secondary_style()
                } else {
                    theme.dim_style()
                };

                let marker = ui::selected_marker(selected);
                let value = field.value(&app.config);
                let gap = width
                    .saturating_sub(
                        marker.chars().count() + field.label().len() + value.chars().count(),
                    )
                    .max(1);

                ListItem::new(Line::from(vec![
                    Span::styled(marker, marker_style),
                    Span::styled(field.label(), label_style),
                    Span::raw(" ".repeat(gap)),
                    Span::styled(value, value_style),
                ]))
                .style(if selected {
                    theme.selected_style()
                } else {
                    ui::plain_style()
                })
            }
        })
        .collect();

    frame.render_widget(List::new(items), area);
}

fn draw_editor(frame: &mut Frame, area: Rect, app: &App, field: SettingField, theme: &Theme) {
    let rect = ui::centered_rect(72, 9, area);
    frame.render_widget(Clear, rect);

    let block = ui::block(&format!("Edit {}", field.label()), theme);
    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    let input = Line::from(vec![
        Span::styled(app.settings.buffer.clone(), theme.text_style()),
        Span::styled("_", theme.accent_style()),
    ]);
    let hint = Line::from(vec![
        Span::styled("Enter", theme.key_style()),
        Span::styled(" save   ", theme.dim_style()),
        Span::styled("Esc", theme.key_style()),
        Span::styled(" cancel   ", theme.dim_style()),
        Span::styled("Ctrl-U", theme.key_style()),
        Span::styled(" clear", theme.dim_style()),
    ]);

    frame.render_widget(
        Paragraph::new(vec![input, Line::from(""), hint])
            .wrap(Wrap { trim: false })
            .style(theme.text_style()),
        inner,
    );
}
