//! Keyboard handling for the interactive browser.
//!
//! Key events are translated into [`App`] operations here. Keeping this in one
//! module makes the key map easy to audit and keeps rendering free of input
//! side effects.

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::app::state::{App, Screen};
use crate::download::ConflictResolution;

/// Dispatch a terminal event.
pub fn handle_event(app: &mut App, event: Event) {
    match event {
        Event::Key(key) if key.kind != KeyEventKind::Release => handle_key(app, key),
        // The next loop iteration redraws at the new size; no state is needed.
        Event::Resize(_, _) => {}
        _ => {}
    }
}

/// Dispatch one key press.
pub fn handle_key(app: &mut App, key: KeyEvent) {
    if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c' | 'q'))
    {
        app.should_quit = true;
        return;
    }

    if app.screen == Screen::Help {
        if matches!(
            key.code,
            KeyCode::Esc | KeyCode::Backspace | KeyCode::Char('q' | '?')
        ) {
            app.close_help();
        }
        return;
    }

    if app.screen == Screen::Splash {
        match key.code {
            KeyCode::Char('q') => app.should_quit = true,
            _ => app.leave_splash(),
        }
        return;
    }

    if app.screen == Screen::Settings && app.settings.editing {
        handle_text_editor(app, key);
        return;
    }

    if app.screen == Screen::Browse && app.search_active {
        handle_search(app, key);
        return;
    }

    if app.screen == Screen::Browse && app.year_picker_open {
        handle_year_picker(app, key);
        return;
    }

    if app.screen == Screen::Queue && !app.queue.conflicts.is_empty() {
        handle_conflict(app, key);
        return;
    }

    match key.code {
        KeyCode::Char('q') => {
            app.should_quit = true;
            return;
        }
        KeyCode::Char('?') => {
            app.open_help();
            return;
        }
        KeyCode::Char('s') => {
            app.open_settings();
            return;
        }
        KeyCode::Char('D') => {
            app.open_queue();
            return;
        }
        _ => {}
    }

    match app.screen {
        Screen::Board | Screen::Level | Screen::Stream | Screen::Subject | Screen::Material => {
            handle_menu(app, key);
        }
        Screen::Browse => handle_browse(app, key),
        Screen::Queue => handle_queue(app, key),
        Screen::Settings => handle_settings(app, key),
        Screen::Splash | Screen::Help => {}
    }
}

fn handle_menu(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => app.move_selection(-1),
        KeyCode::Down | KeyCode::Char('j') => app.move_selection(1),
        KeyCode::PageUp => app.move_selection(-8),
        KeyCode::PageDown => app.move_selection(8),
        KeyCode::Home => app.move_selection_start(),
        KeyCode::End => app.move_selection_end(),
        KeyCode::Enter | KeyCode::Char(' ') => app.confirm_current(),
        KeyCode::Esc | KeyCode::Backspace => app.go_back(),
        _ => {}
    }
}

fn handle_browse(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => app.browse_move(-1),
        KeyCode::Down | KeyCode::Char('j') => app.browse_move(1),
        KeyCode::PageUp => app.browse_move(-10),
        KeyCode::PageDown => app.browse_move(10),
        KeyCode::Home => app.browse_cursor = 0,
        KeyCode::End => {
            let len = app.filtered().len();
            app.browse_cursor = len.saturating_sub(1);
        }
        KeyCode::Char(' ') => app.toggle_selected(),
        KeyCode::Char('a') => app.toggle_select_all(),
        KeyCode::Char('d') | KeyCode::Enter => app.queue_selection(),
        KeyCode::Char('/') => app.open_search(),
        KeyCode::Char('y') => app.open_year_picker(),
        KeyCode::Char('t') => app.toggle_latest(),
        KeyCode::Esc | KeyCode::Backspace => app.go_back(),
        _ => {}
    }
}

fn handle_search(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => app.cancel_search(),
        KeyCode::Enter => app.commit_search(),
        KeyCode::Backspace => app.pop_search_char(),
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => app.clear_search(),
        KeyCode::Char(character)
            if !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
        {
            app.push_search_char(character);
        }
        KeyCode::Up => app.browse_move(-1),
        KeyCode::Down => app.browse_move(1),
        KeyCode::PageUp => app.browse_move(-10),
        KeyCode::PageDown => app.browse_move(10),
        _ => {}
    }
}

fn handle_year_picker(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => app.year_move(-1),
        KeyCode::Down | KeyCode::Char('j') => app.year_move(1),
        KeyCode::PageUp => app.year_move(-5),
        KeyCode::PageDown => app.year_move(5),
        KeyCode::Home => app.year_cursor = 0,
        KeyCode::End => {
            let len = app.available_years().len();
            app.year_cursor = len;
        }
        KeyCode::Enter | KeyCode::Char(' ') => app.confirm_year(),
        KeyCode::Esc | KeyCode::Backspace => app.close_year_picker(),
        KeyCode::Char('q') => app.should_quit = true,
        _ => {}
    }
}

fn handle_queue(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => app.queue_move(-1),
        KeyCode::Down | KeyCode::Char('j') => app.queue_move(1),
        KeyCode::PageUp => app.queue_move(-8),
        KeyCode::PageDown => app.queue_move(8),
        KeyCode::Home => app.move_selection_start(),
        KeyCode::End => app.move_selection_end(),
        KeyCode::Char('p') | KeyCode::Enter => app.toggle_pause(),
        KeyCode::Char('r') => app.queue_retry(),
        KeyCode::Char('c') => app.queue_cancel(),
        KeyCode::Esc | KeyCode::Backspace => app.go_back(),
        _ => {}
    }
}

fn handle_conflict(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('s') => app.resolve_conflict(ConflictResolution::Skip),
        KeyCode::Char('r') => app.resolve_conflict(ConflictResolution::Rename),
        KeyCode::Char('o') => app.resolve_conflict(ConflictResolution::Overwrite),
        KeyCode::Char('c') | KeyCode::Esc => app.resolve_conflict(ConflictResolution::Cancel),
        KeyCode::Char('q') => app.should_quit = true,
        _ => {}
    }
}

fn handle_settings(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => app.settings_move(-1),
        KeyCode::Down | KeyCode::Char('j') => app.settings_move(1),
        KeyCode::Home => app.settings.cursor = 0,
        KeyCode::End => app.settings.cursor = crate::app::state::SettingField::ALL.len() - 1,
        KeyCode::Enter | KeyCode::Char(' ') => app.settings_activate(),
        KeyCode::Left | KeyCode::Char('-') => app.settings_adjust(-1),
        KeyCode::Right | KeyCode::Char('+') => app.settings_adjust(1),
        KeyCode::Esc | KeyCode::Backspace => app.close_settings(),
        _ => {}
    }
}

fn handle_text_editor(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Enter => app.commit_setting_editor(),
        KeyCode::Esc => app.cancel_setting_editor(),
        KeyCode::Backspace => app.pop_editor_char(),
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => app.clear_editor(),
        KeyCode::Char(character)
            if !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
        {
            app.push_editor_char(character);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::test_support::sample_app;

    fn press(app: &mut App, code: KeyCode) {
        handle_key(app, KeyEvent::new(code, KeyModifiers::NONE));
    }

    #[test]
    fn ctrl_c_always_quits() {
        let mut app = sample_app();
        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
        );
        assert!(app.should_quit);
    }

    #[test]
    fn help_opens_and_closes() {
        let mut app = sample_app();
        press(&mut app, KeyCode::Char('?'));
        assert_eq!(app.screen, Screen::Help);

        press(&mut app, KeyCode::Esc);
        assert_eq!(app.screen, Screen::Board);
    }

    #[test]
    fn enter_advances_the_board_screen() {
        let mut app = sample_app();
        assert_eq!(app.screen, Screen::Board);
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.screen, Screen::Level);
        assert_eq!(app.selection.board.as_deref(), Some("kseeb"));
    }

    #[test]
    fn search_typing_and_escape_restore_the_query() {
        let mut app = sample_app();
        app.selection.board = Some("kseeb".to_owned());
        app.selection.level = Some("SSLC".to_owned());
        app.selection.subject = Some("Mathematics".to_owned());
        app.selection.kind = Some(crate::catalog::MaterialType::PreviousYearPaper);
        app.screen = Screen::Browse;

        press(&mut app, KeyCode::Char('/'));
        assert!(app.search_active);
        press(&mut app, KeyCode::Char('m'));
        press(&mut app, KeyCode::Char('a'));
        assert_eq!(app.search, "ma");

        press(&mut app, KeyCode::Esc);
        assert!(!app.search_active);
        assert!(app.search.is_empty());
    }

    #[test]
    fn settings_text_editor_accepts_and_discards_input() {
        let mut app = sample_app();
        app.screen = Screen::Settings;
        app.settings.cursor = 0;

        press(&mut app, KeyCode::Enter);
        assert!(app.settings.editing);

        press(&mut app, KeyCode::Char('x'));
        assert!(app.settings.buffer.ends_with('x'));

        press(&mut app, KeyCode::Esc);
        assert!(!app.settings.editing);
        assert!(app.settings.buffer.is_empty());
    }

    #[test]
    fn q_is_text_while_searching_but_quits_on_menus() {
        let mut app = sample_app();
        app.selection.board = Some("kseeb".to_owned());
        app.selection.level = Some("SSLC".to_owned());
        app.selection.subject = Some("Mathematics".to_owned());
        app.selection.kind = Some(crate::catalog::MaterialType::PreviousYearPaper);
        app.screen = Screen::Browse;
        app.search_active = true;

        press(&mut app, KeyCode::Char('q'));
        assert!(!app.should_quit);
        assert_eq!(app.search, "q");

        app.search_active = false;
        press(&mut app, KeyCode::Char('q'));
        assert!(app.should_quit);
    }
}
