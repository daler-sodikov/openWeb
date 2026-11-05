//! Keys and pastes to bytes, as xterm-compatible terminals send them.

use crate::model::Modes;

/// A key the view could not send as plain text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Key {
    /// `KeyboardEvent.key` (`ArrowUp`, `Enter`, `F5`, `c`, …).
    pub key: String,
    /// Ctrl held.
    pub ctrl: bool,
    /// Alt (Option) held.
    pub alt: bool,
    /// Shift held.
    pub shift: bool,
}

/// xterm modifier parameter: 1 + shift + 2·alt + 4·ctrl.
fn modifier(key: &Key) -> u8 {
    1 + u8::from(key.shift) + 2 * u8::from(key.alt) + 4 * u8::from(key.ctrl)
}

/// `CSI 1;m X` with modifiers, else `CSI X` (or `SS3 X` in app mode).
fn cursor_like(key: &Key, final_byte: char, app: bool) -> Vec<u8> {
    let m = modifier(key);
    if m > 1 {
        format!("\x1b[1;{m}{final_byte}").into_bytes()
    } else if app {
        format!("\x1bO{final_byte}").into_bytes()
    } else {
        format!("\x1b[{final_byte}").into_bytes()
    }
}

/// `CSI n ~` or `CSI n;m ~`.
fn tilde(key: &Key, number: u8) -> Vec<u8> {
    let m = modifier(key);
    if m > 1 {
        format!("\x1b[{number};{m}~").into_bytes()
    } else {
        format!("\x1b[{number}~").into_bytes()
    }
}

/// The bytes for `key` in `modes`, or `None` for keys a terminal ignores
/// (lone modifiers, unknown names).
pub fn encode_key(key: &Key, modes: Modes) -> Option<Vec<u8>> {
    let alt_prefix = |mut bytes: Vec<u8>| {
        if key.alt {
            bytes.insert(0, 0x1b);
        }
        bytes
    };
    let bytes = match key.key.as_str() {
        "Enter" => alt_prefix(vec![b'\r']),
        "Backspace" => alt_prefix(vec![if key.ctrl { 0x08 } else { 0x7f }]),
        "Tab" if key.shift => b"\x1b[Z".to_vec(),
        "Tab" => alt_prefix(vec![b'\t']),
        "Escape" => alt_prefix(vec![0x1b]),
        "ArrowUp" => cursor_like(key, 'A', modes.app_cursor),
        "ArrowDown" => cursor_like(key, 'B', modes.app_cursor),
        "ArrowRight" => cursor_like(key, 'C', modes.app_cursor),
        "ArrowLeft" => cursor_like(key, 'D', modes.app_cursor),
        "Home" => cursor_like(key, 'H', modes.app_cursor),
        "End" => cursor_like(key, 'F', modes.app_cursor),
        "Insert" => tilde(key, 2),
        "Delete" => tilde(key, 3),
        "PageUp" => tilde(key, 5),
        "PageDown" => tilde(key, 6),
        "F1" => cursor_like(key, 'P', true),
        "F2" => cursor_like(key, 'Q', true),
        "F3" => cursor_like(key, 'R', true),
        "F4" => cursor_like(key, 'S', true),
        "F5" => tilde(key, 15),
        "F6" => tilde(key, 17),
        "F7" => tilde(key, 18),
        "F8" => tilde(key, 19),
        "F9" => tilde(key, 20),
        "F10" => tilde(key, 21),
        "F11" => tilde(key, 23),
        "F12" => tilde(key, 24),
        other => {
            let mut chars = other.chars();
            let (Some(c), None) = (chars.next(), chars.next()) else {
                return None;
            };
            if key.ctrl {
                alt_prefix(vec![control_byte(c)?])
            } else {
                let mut buf = [0; 4];
                alt_prefix(c.encode_utf8(&mut buf).as_bytes().to_vec())
            }
        }
    };
    Some(bytes)
}

/// Ctrl+character: the C0 control code (`Ctrl+C` = 0x03).
fn control_byte(c: char) -> Option<u8> {
    match c {
        'a'..='z' => Some(c as u8 - b'a' + 1),
        'A'..='Z' => Some(c as u8 - b'A' + 1),
        '@' | ' ' | '2' => Some(0),
        '[' | '3' => Some(0x1b),
        '\\' | '4' => Some(0x1c),
        ']' | '5' => Some(0x1d),
        '^' | '6' => Some(0x1e),
        '_' | '-' | '7' => Some(0x1f),
        '?' | '8' => Some(0x7f),
        _ => None,
    }
}

/// Pasted text as bytes: line breaks become CR (what Enter sends); with
/// bracketed paste the text is wrapped and any ESC inside it is dropped so
/// the paste cannot end the bracket early.
pub fn encode_paste(text: &str, modes: Modes) -> Vec<u8> {
    let normalized = text.replace("\r\n", "\r").replace('\n', "\r");
    if modes.bracketed_paste {
        let inner: String = normalized.chars().filter(|&c| c != '\x1b').collect();
        format!("\x1b[200~{inner}\x1b[201~").into_bytes()
    } else {
        normalized.into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(name: &str) -> Key {
        Key {
            key: name.into(),
            ctrl: false,
            alt: false,
            shift: false,
        }
    }

    fn enc(k: &Key) -> String {
        String::from_utf8(encode_key(k, Modes::default()).unwrap()).unwrap()
    }

    #[test]
    fn cursor_keys_follow_application_mode_and_modifiers() {
        assert_eq!(enc(&key("ArrowUp")), "\x1b[A");
        let app = Modes {
            app_cursor: true,
            ..Modes::default()
        };
        assert_eq!(encode_key(&key("ArrowUp"), app).unwrap(), b"\x1bOA");
        let ctrl_right = Key {
            ctrl: true,
            ..key("ArrowRight")
        };
        assert_eq!(enc(&ctrl_right), "\x1b[1;5C");
        let shift_alt_home = Key {
            shift: true,
            alt: true,
            ..key("Home")
        };
        assert_eq!(enc(&shift_alt_home), "\x1b[1;4H");
    }

    #[test]
    fn function_and_editing_keys() {
        assert_eq!(enc(&key("F1")), "\x1bOP");
        assert_eq!(enc(&key("F5")), "\x1b[15~");
        assert_eq!(enc(&key("F12")), "\x1b[24~");
        assert_eq!(enc(&key("Delete")), "\x1b[3~");
        let ctrl_page_up = Key {
            ctrl: true,
            ..key("PageUp")
        };
        assert_eq!(enc(&ctrl_page_up), "\x1b[5;5~");
        let shift_tab = Key {
            shift: true,
            ..key("Tab")
        };
        assert_eq!(enc(&shift_tab), "\x1b[Z");
        assert_eq!(enc(&key("Backspace")), "\x7f");
        assert_eq!(enc(&key("Enter")), "\r");
    }

    #[test]
    fn control_and_alt_characters() {
        let ctrl = |c: &str| Key {
            ctrl: true,
            ..key(c)
        };
        assert_eq!(encode_key(&ctrl("c"), Modes::default()).unwrap(), [3]);
        assert_eq!(encode_key(&ctrl("D"), Modes::default()).unwrap(), [4]);
        assert_eq!(encode_key(&ctrl("["), Modes::default()).unwrap(), [0x1b]);
        assert_eq!(encode_key(&ctrl(" "), Modes::default()).unwrap(), [0]);
        let alt_b = Key {
            alt: true,
            ..key("b")
        };
        assert_eq!(enc(&alt_b), "\x1bb");
        assert_eq!(encode_key(&key("Shift"), Modes::default()), None);
        assert_eq!(encode_key(&ctrl("é"), Modes::default()), None);
    }

    #[test]
    fn pastes_use_cr_and_brackets_without_escapes() {
        assert_eq!(encode_paste("a\nb\r\nc", Modes::default()), b"a\rb\rc");
        let bracketed = Modes {
            bracketed_paste: true,
            ..Modes::default()
        };
        assert_eq!(
            encode_paste("x\x1b[201~y", bracketed),
            b"\x1b[200~x[201~y\x1b[201~"
        );
    }
}
