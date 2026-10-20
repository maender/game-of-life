//! Keyboard shortcut table (logical key first, so AZERTY Mac works).

use winit::keyboard::{Key, KeyCode, NamedKey, PhysicalKey};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Action {
    TogglePlay,
    Step,
    Faster,
    Slower,
    Randomize,
    Clear,
    ToolDraw,
    ToolErase,
    Recenter,
    NextStyle,
    ToggleAge,
    ToggleTrails,
    ToggleAnim,
    ToggleDegraded,
    TogglePanel,
    ToggleFullscreen,
    Quit,
}

/// Maps a key press to its action: by produced character (case-insensitive),
/// with the physical digit row as a fallback for `1` `2` `3` (AZERTY produces
/// `&` `é` `"` there without Shift).
pub fn action_for_key(logical: &Key, physical: PhysicalKey) -> Option<Action> {
    let by_logical = match logical {
        Key::Named(NamedKey::Space) => Some(Action::TogglePlay),
        Key::Named(NamedKey::Tab) => Some(Action::TogglePanel),
        Key::Named(NamedKey::Escape) => Some(Action::Quit),
        Key::Character(c) => match c.to_lowercase().as_str() {
            " " => Some(Action::TogglePlay),
            "n" => Some(Action::Step),
            "+" | "=" => Some(Action::Faster),
            "-" => Some(Action::Slower),
            "r" => Some(Action::Randomize),
            "c" => Some(Action::Clear),
            "d" => Some(Action::ToolDraw),
            "e" => Some(Action::ToolErase),
            "h" => Some(Action::Recenter),
            "s" => Some(Action::NextStyle),
            "1" => Some(Action::ToggleAge),
            "2" => Some(Action::ToggleTrails),
            "3" => Some(Action::ToggleAnim),
            "l" => Some(Action::ToggleDegraded),
            "f" => Some(Action::ToggleFullscreen),
            _ => None,
        },
        _ => None,
    };
    by_logical.or(match physical {
        PhysicalKey::Code(KeyCode::Digit1) => Some(Action::ToggleAge),
        PhysicalKey::Code(KeyCode::Digit2) => Some(Action::ToggleTrails),
        PhysicalKey::Code(KeyCode::Digit3) => Some(Action::ToggleAnim),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(s: &str) -> Key {
        Key::Character(s.into())
    }

    const NO_CODE: PhysicalKey = PhysicalKey::Code(KeyCode::KeyZ);

    #[test]
    fn spec_shortcuts() {
        let named = [
            (NamedKey::Space, Action::TogglePlay),
            (NamedKey::Tab, Action::TogglePanel),
            (NamedKey::Escape, Action::Quit),
        ];
        for (k, a) in named {
            assert_eq!(action_for_key(&Key::Named(k), NO_CODE), Some(a), "{k:?}");
        }
        let chars = [
            ("n", Action::Step),
            ("N", Action::Step),
            ("+", Action::Faster),
            ("=", Action::Faster),
            ("-", Action::Slower),
            ("r", Action::Randomize),
            ("R", Action::Randomize),
            ("c", Action::Clear),
            ("C", Action::Clear),
            ("d", Action::ToolDraw),
            ("D", Action::ToolDraw),
            ("e", Action::ToolErase),
            ("E", Action::ToolErase),
            ("h", Action::Recenter),
            ("H", Action::Recenter),
            ("s", Action::NextStyle),
            ("S", Action::NextStyle),
            ("1", Action::ToggleAge),
            ("2", Action::ToggleTrails),
            ("3", Action::ToggleAnim),
            ("l", Action::ToggleDegraded),
            ("L", Action::ToggleDegraded),
            ("f", Action::ToggleFullscreen),
            ("F", Action::ToggleFullscreen),
        ];
        for (c, a) in chars {
            assert_eq!(action_for_key(&ch(c), NO_CODE), Some(a), "{c:?}");
        }
    }

    #[test]
    fn digit_keys_work_on_azerty() {
        let digit = |c| PhysicalKey::Code(c);
        assert_eq!(
            action_for_key(&ch("&"), digit(KeyCode::Digit1)),
            Some(Action::ToggleAge)
        );
        assert_eq!(
            action_for_key(&ch("é"), digit(KeyCode::Digit2)),
            Some(Action::ToggleTrails)
        );
        assert_eq!(
            action_for_key(&ch("\""), digit(KeyCode::Digit3)),
            Some(Action::ToggleAnim)
        );
    }

    #[test]
    fn unknown_key_is_none() {
        assert_eq!(action_for_key(&ch("z"), NO_CODE), None);
    }
}
