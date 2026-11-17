use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::{Action, Mode};
use crate::github::ReviewEvent;

/// Translates a key press into an [`Action`] for the current mode. Unbound
/// keys give `None`. `Ctrl-c` quits from anywhere.
pub fn action_for(key: KeyEvent, mode: &Mode) -> Option<Action> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    if ctrl && key.code == KeyCode::Char('c') {
        return Some(Action::Quit);
    }
    match mode {
        Mode::Browse => browse_action(key.code, ctrl),
        Mode::PickReviewEvent => pick_action(key.code),
        Mode::Compose(_) => compose_action(key.code, ctrl),
    }
}

fn browse_action(code: KeyCode, ctrl: bool) -> Option<Action> {
    let action = match (code, ctrl) {
        (KeyCode::Char('q'), false) => Action::Quit,
        (KeyCode::Esc, _) => Action::Escape,
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
        (KeyCode::Char('v'), false) => Action::ToggleTab,
        (KeyCode::Char('c'), false) => Action::Comment,
        (KeyCode::Char('R'), false) => Action::Review,
        _ => return None,
    };
    Some(action)
}

fn pick_action(code: KeyCode) -> Option<Action> {
    let action = match code {
        KeyCode::Char('a') => Action::Pick(ReviewEvent::Approve),
        KeyCode::Char('r') => Action::Pick(ReviewEvent::RequestChanges),
        KeyCode::Char('c') => Action::Pick(ReviewEvent::Comment),
        KeyCode::Esc => Action::Cancel,
        _ => return None,
    };
    Some(action)
}

fn compose_action(code: KeyCode, ctrl: bool) -> Option<Action> {
    let action = match (code, ctrl) {
        (KeyCode::Esc, _) => Action::Cancel,
        (KeyCode::Char('s'), true) => Action::Submit,
        (KeyCode::Enter, _) => Action::Newline,
        (KeyCode::Backspace, _) => Action::Backspace,
        (KeyCode::Char(c), false) => Action::Type(c),
        _ => return None,
    };
    Some(action)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use crate::app::{Compose, ComposeTarget, Mode};
    use crate::github::ReviewEvent;

    fn browse(key: KeyEvent) -> Option<Action> {
        action_for(key, &Mode::Browse)
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    #[test]
    fn maps_navigation_keys() {
        assert_eq!(browse(key(KeyCode::Char('j'))), Some(Action::Down));
        assert_eq!(browse(key(KeyCode::Down)), Some(Action::Down));
        assert_eq!(browse(key(KeyCode::Char('k'))), Some(Action::Up));
        assert_eq!(browse(key(KeyCode::Up)), Some(Action::Up));
        assert_eq!(browse(key(KeyCode::Char('g'))), Some(Action::Top));
        assert_eq!(browse(key(KeyCode::Home)), Some(Action::Top));
        let shift_g = KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT);
        assert_eq!(browse(shift_g), Some(Action::Bottom));
        assert_eq!(browse(key(KeyCode::End)), Some(Action::Bottom));
        assert_eq!(browse(ctrl('d')), Some(Action::PageDown));
        assert_eq!(browse(key(KeyCode::PageDown)), Some(Action::PageDown));
        assert_eq!(browse(ctrl('u')), Some(Action::PageUp));
        assert_eq!(browse(key(KeyCode::PageUp)), Some(Action::PageUp));
    }

    #[test]
    fn maps_pane_and_action_keys() {
        assert_eq!(browse(key(KeyCode::Tab)), Some(Action::NextPane));
        assert_eq!(browse(key(KeyCode::Char('l'))), Some(Action::NextPane));
        assert_eq!(browse(key(KeyCode::BackTab)), Some(Action::PrevPane));
        assert_eq!(browse(key(KeyCode::Char('h'))), Some(Action::PrevPane));
        assert_eq!(browse(key(KeyCode::Enter)), Some(Action::Open));
        assert_eq!(browse(key(KeyCode::Char('r'))), Some(Action::Refresh));
        assert_eq!(browse(key(KeyCode::Char('q'))), Some(Action::Quit));
        assert_eq!(browse(ctrl('c')), Some(Action::Quit));
    }

    #[test]
    fn unknown_key_is_none() {
        assert_eq!(browse(key(KeyCode::Char('x'))), None);
    }

    #[test]
    fn maps_new_browse_keys() {
        assert_eq!(browse(key(KeyCode::Esc)), Some(Action::Escape));
        assert_eq!(browse(key(KeyCode::Char('v'))), Some(Action::ToggleTab));
        assert_eq!(browse(key(KeyCode::Char('c'))), Some(Action::Comment));
        let shift_r = KeyEvent::new(KeyCode::Char('R'), KeyModifiers::SHIFT);
        assert_eq!(browse(shift_r), Some(Action::Review));
    }

    #[test]
    fn maps_pick_keys() {
        let mode = Mode::PickReviewEvent;
        assert_eq!(action_for(key(KeyCode::Char('a')), &mode), Some(Action::Pick(ReviewEvent::Approve)));
        assert_eq!(action_for(key(KeyCode::Char('r')), &mode), Some(Action::Pick(ReviewEvent::RequestChanges)));
        assert_eq!(action_for(key(KeyCode::Char('c')), &mode), Some(Action::Pick(ReviewEvent::Comment)));
        assert_eq!(action_for(key(KeyCode::Esc), &mode), Some(Action::Cancel));
        assert_eq!(action_for(ctrl('c'), &mode), Some(Action::Quit));
        assert_eq!(action_for(key(KeyCode::Char('j')), &mode), None);
    }

    #[test]
    fn maps_compose_keys() {
        let mode = Mode::Compose(Compose { target: ComposeTarget::PullRequestComment, text: String::new() });
        assert_eq!(action_for(key(KeyCode::Esc), &mode), Some(Action::Cancel));
        assert_eq!(action_for(ctrl('s'), &mode), Some(Action::Submit));
        assert_eq!(action_for(key(KeyCode::Enter), &mode), Some(Action::Newline));
        assert_eq!(action_for(key(KeyCode::Backspace), &mode), Some(Action::Backspace));
        assert_eq!(action_for(key(KeyCode::Char('j')), &mode), Some(Action::Type('j')));
        assert_eq!(action_for(key(KeyCode::Char('q')), &mode), Some(Action::Type('q')));
        assert_eq!(action_for(key(KeyCode::Char(' ')), &mode), Some(Action::Type(' ')));
        let shift_j = KeyEvent::new(KeyCode::Char('J'), KeyModifiers::SHIFT);
        assert_eq!(action_for(shift_j, &mode), Some(Action::Type('J')));
        assert_eq!(action_for(ctrl('c'), &mode), Some(Action::Quit));
        assert_eq!(action_for(key(KeyCode::Tab), &mode), None);
    }
}
