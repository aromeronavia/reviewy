use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::Action;

/// Translates a key press into an [`Action`]. Unbound keys give `None`.
pub fn action_for(key: KeyEvent) -> Option<Action> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let action = match (key.code, ctrl) {
        (KeyCode::Char('q'), false) | (KeyCode::Char('c'), true) => Action::Quit,
        (KeyCode::Tab, _) | (KeyCode::Char('l'), false) => Action::NextPane,
        (KeyCode::BackTab, _) | (KeyCode::Char('h'), false) => Action::PrevPane,
        (KeyCode::Char('j'), false) | (KeyCode::Down, _) => Action::Down,
        (KeyCode::Char('k'), false) | (KeyCode::Up, _) => Action::Up,
        (KeyCode::Char('g'), false) | (KeyCode::Home, _) => Action::Top,
        (KeyCode::Char('G'), false) | (KeyCode::End, _) => Action::Bottom,
        (KeyCode::Char('d'), true) | (KeyCode::PageDown, _) => Action::PageDown,
        (KeyCode::Char('u'), true) | (KeyCode::PageUp, _) => Action::PageUp,
        (KeyCode::Enter, _) => Action::Open,
        (KeyCode::Char('r'), false) => Action::Refresh,
        _ => return None,
    };
    Some(action)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    #[test]
    fn maps_navigation_keys() {
        assert_eq!(action_for(key(KeyCode::Char('j'))), Some(Action::Down));
        assert_eq!(action_for(key(KeyCode::Down)), Some(Action::Down));
        assert_eq!(action_for(key(KeyCode::Char('k'))), Some(Action::Up));
        assert_eq!(action_for(key(KeyCode::Up)), Some(Action::Up));
        assert_eq!(action_for(key(KeyCode::Char('g'))), Some(Action::Top));
        assert_eq!(action_for(key(KeyCode::Home)), Some(Action::Top));
        let shift_g = KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT);
        assert_eq!(action_for(shift_g), Some(Action::Bottom));
        assert_eq!(action_for(key(KeyCode::End)), Some(Action::Bottom));
        assert_eq!(action_for(ctrl('d')), Some(Action::PageDown));
        assert_eq!(action_for(key(KeyCode::PageDown)), Some(Action::PageDown));
        assert_eq!(action_for(ctrl('u')), Some(Action::PageUp));
        assert_eq!(action_for(key(KeyCode::PageUp)), Some(Action::PageUp));
    }

    #[test]
    fn maps_pane_and_action_keys() {
        assert_eq!(action_for(key(KeyCode::Tab)), Some(Action::NextPane));
        assert_eq!(action_for(key(KeyCode::Char('l'))), Some(Action::NextPane));
        assert_eq!(action_for(key(KeyCode::BackTab)), Some(Action::PrevPane));
        assert_eq!(action_for(key(KeyCode::Char('h'))), Some(Action::PrevPane));
        assert_eq!(action_for(key(KeyCode::Enter)), Some(Action::Open));
        assert_eq!(action_for(key(KeyCode::Char('r'))), Some(Action::Refresh));
        assert_eq!(action_for(key(KeyCode::Char('q'))), Some(Action::Quit));
        assert_eq!(action_for(ctrl('c')), Some(Action::Quit));
    }

    #[test]
    fn unknown_key_is_none() {
        assert_eq!(action_for(key(KeyCode::Char('x'))), None);
    }
}
