use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    StepForward,
    StepBack,
    Continue,
    Pause,
    NextNode,
    PrevNode,
    EditSource,
    Reload,
    ScrollUp,
    ScrollDown,
    ScrollRegUp,
    ScrollRegDown,
    Quit,
    None,
}

pub fn key_to_action(key: KeyEvent) -> Action {
    match (key.code, key.modifiers) {
        (KeyCode::Char('q'), _) | (KeyCode::Esc, _) => Action::Quit,
        (KeyCode::Char('n'), _) | (KeyCode::Right, _) => Action::StepForward,
        (KeyCode::Char('b'), _) | (KeyCode::Left, _) => Action::StepBack,
        (KeyCode::Char('c'), _) => Action::Continue,
        (KeyCode::Char('p'), _) | (KeyCode::Char(' '), _) => Action::Pause,
        (KeyCode::Tab, KeyModifiers::SHIFT) => Action::PrevNode,
        (KeyCode::BackTab, _) => Action::PrevNode,
        (KeyCode::Tab, _) => Action::NextNode,
        (KeyCode::Char('e'), _) => Action::EditSource,
        (KeyCode::Char('r'), _) => Action::Reload,
        (KeyCode::Up, _) | (KeyCode::Char('k'), _) => Action::ScrollUp,
        (KeyCode::Down, _) | (KeyCode::Char('j'), _) => Action::ScrollDown,
        (KeyCode::Char('K'), _) => Action::ScrollRegUp,
        (KeyCode::Char('J'), _) => Action::ScrollRegDown,
        _ => Action::None,
    }
}
