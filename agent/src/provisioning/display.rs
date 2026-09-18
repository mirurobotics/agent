// external crates
#[cfg(windows)]
use windows_sys::Win32::System::Console::{
    GetConsoleMode, GetStdHandle, SetConsoleMode, ENABLE_VIRTUAL_TERMINAL_PROCESSING,
    STD_OUTPUT_HANDLE,
};

pub enum Colors {
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
}

/// Maps each color to a full SGR parameter string. Green is Miru green
/// (#059669) as bold truecolor for CLI parity; the others stay basic ANSI.
pub fn color(text: &str, color: Colors) -> String {
    let params = match color {
        Colors::Red => "31",
        Colors::Green => "1;38;2;5;150;105",
        Colors::Yellow => "33",
        Colors::Blue => "34",
        Colors::Magenta => "35",
        Colors::Cyan => "36",
        Colors::White => "37",
    };
    format!("\x1b[{params}m{text}\x1b[0m")
}

pub fn format_info(text: &str) -> String {
    format!("{}{}", color("==> ", Colors::Green), text)
}

/// Enables ANSI virtual-terminal processing on the Windows console so the
/// provisioning SGR sequences render as color. Best-effort: a no-op when no
/// console is attached or the mode cannot be read/set. Never fails startup.
#[cfg(windows)]
pub fn enable_ansi() {
    // SAFETY: standard Win32 console FFI. An absent or invalid handle makes
    // GetConsoleMode return 0 (FALSE), so SetConsoleMode is never reached.
    unsafe {
        let handle = GetStdHandle(STD_OUTPUT_HANDLE);
        let mut mode: u32 = 0;
        if GetConsoleMode(handle, &mut mode) == 0 {
            return;
        }
        let _ = SetConsoleMode(handle, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING);
    }
}

/// Non-Windows platforms already honor ANSI SGR sequences.
#[cfg(not(windows))]
pub fn enable_ansi() {}

#[cfg(test)]
mod tests {
    use super::*;

    mod color {
        use super::*;

        #[test]
        fn all_variants() {
            let cases = vec![
                (Colors::Red, "31"),
                (Colors::Green, "1;38;2;5;150;105"),
                (Colors::Yellow, "33"),
                (Colors::Blue, "34"),
                (Colors::Magenta, "35"),
                (Colors::Cyan, "36"),
                (Colors::White, "37"),
            ];
            for (variant, expected_params) in cases {
                let result = color("hello", variant);
                assert_eq!(
                    result,
                    format!("\x1b[{expected_params}mhello\x1b[0m"),
                    "wrong SGR params for color {expected_params}"
                );
            }
        }

        #[test]
        fn empty_text() {
            let result = color("", Colors::Red);
            assert_eq!(result, "\x1b[31m\x1b[0m");
        }
    }

    mod format_info {
        use super::*;

        #[test]
        fn formats_with_green_arrow() {
            let result = format_info("test message");
            let expected = format!("{}test message", color("==> ", Colors::Green));
            assert_eq!(result, expected);
        }
    }

    mod enable_ansi {
        use super::*;

        #[test]
        fn is_callable() {
            enable_ansi();
        }
    }
}
