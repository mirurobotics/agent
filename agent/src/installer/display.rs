// external crates
#[allow(unused_imports)]
use tracing::{debug, error, info, warn};

pub enum Colors {
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
}

pub fn color(text: &str, color: Colors) -> String {
    let color_code = match color {
        Colors::Red => "31",
        Colors::Green => "32",
        Colors::Yellow => "33",
        Colors::Blue => "34",
        Colors::Magenta => "35",
        Colors::Cyan => "36",
        Colors::White => "37",
    };
    format!("\x1b[{color_code}m{text}\x1b[0m")
}

pub fn format_info(text: &str) -> String {
    format!("{}{}", color("==> ", Colors::Green), text)
}

#[cfg(test)]
mod tests {
    use super::*;

    mod color {
        use super::*;

        #[test]
        fn all_variants() {
            let cases = vec![
                (Colors::Red, "31"),
                (Colors::Green, "32"),
                (Colors::Yellow, "33"),
                (Colors::Blue, "34"),
                (Colors::Magenta, "35"),
                (Colors::Cyan, "36"),
                (Colors::White, "37"),
            ];
            for (variant, expected_code) in cases {
                let result = color("hello", variant);
                assert_eq!(
                    result,
                    format!("\x1b[{expected_code}mhello\x1b[0m"),
                    "wrong ANSI code for color {expected_code}"
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
}
