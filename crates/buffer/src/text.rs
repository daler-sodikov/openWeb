//! Line-ending detection and normalization.

use std::borrow::Cow;

/// Line ending style of a file on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineEnding {
    /// `\n` (Unix, macOS, most web projects).
    #[default]
    Lf,
    /// `\r\n` (Windows).
    CrLf,
}

impl LineEnding {
    /// Converts `\n`-normalized text back to this line ending.
    pub fn apply(self, text: &str) -> Cow<'_, str> {
        match self {
            Self::Lf => Cow::Borrowed(text),
            Self::CrLf => Cow::Owned(text.replace('\n', "\r\n")),
        }
    }
}

/// Normalizes all line breaks (`\r\n`, lone `\r`, `\n`) to `\n` and reports
/// the dominant original style. Lone `\r` (classic Mac) is counted as neither
/// style and will be written back using the detected style.
pub fn normalize_line_endings(text: &str) -> (String, LineEnding) {
    let crlf = text.matches("\r\n").count();
    let lf = text.matches('\n').count() - crlf;
    let ending = if crlf > lf {
        LineEnding::CrLf
    } else {
        LineEnding::Lf
    };
    if !text.contains('\r') {
        return (text.to_owned(), ending);
    }
    (text.replace("\r\n", "\n").replace('\r', "\n"), ending)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_and_normalizes() {
        assert_eq!(
            normalize_line_endings("a\nb\n"),
            ("a\nb\n".into(), LineEnding::Lf)
        );
        assert_eq!(
            normalize_line_endings("a\r\nb\r\n"),
            ("a\nb\n".into(), LineEnding::CrLf)
        );
        assert_eq!(
            normalize_line_endings("a\r\nb\nc\n"),
            ("a\nb\nc\n".into(), LineEnding::Lf)
        );
        assert_eq!(
            normalize_line_endings("a\rb"),
            ("a\nb".into(), LineEnding::Lf)
        );
    }

    #[test]
    fn round_trips_crlf() {
        let (text, ending) = normalize_line_endings("x\r\ny\r\n");
        assert_eq!(ending.apply(&text), "x\r\ny\r\n");
        assert_eq!(LineEnding::Lf.apply("x\ny"), "x\ny");
    }
}
