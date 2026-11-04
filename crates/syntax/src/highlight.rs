//! Token classes and capture-name mapping.

use std::fmt::Write as _;

use ferrule_buffer::Rope;

/// Highlight category of a span. Deliberately coarse; themes color these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenClass {
    /// Keywords and at-rules.
    Keyword,
    /// String literals.
    String,
    /// Regexes, template strings and other special strings.
    StringSpecial,
    /// Escape sequences.
    Escape,
    /// Numbers.
    Number,
    /// Constants and built-in constants (`true`, `null`, ...).
    Constant,
    /// Comments.
    Comment,
    /// Function and method names.
    Function,
    /// Types, classes and constructors.
    Type,
    /// Built-in variables (`this`, `console`, ...).
    VariableBuiltin,
    /// Parameters.
    Parameter,
    /// Properties and object keys.
    Property,
    /// Operators.
    Operator,
    /// Brackets, delimiters.
    Punctuation,
    /// Markup tags and CSS selectors.
    Tag,
    /// Markup attributes.
    Attribute,
}

impl TokenClass {
    /// Stable lowercase name, used in golden files.
    pub fn name(self) -> &'static str {
        match self {
            Self::Keyword => "keyword",
            Self::String => "string",
            Self::StringSpecial => "string.special",
            Self::Escape => "escape",
            Self::Number => "number",
            Self::Constant => "constant",
            Self::Comment => "comment",
            Self::Function => "function",
            Self::Type => "type",
            Self::VariableBuiltin => "variable.builtin",
            Self::Parameter => "parameter",
            Self::Property => "property",
            Self::Operator => "operator",
            Self::Punctuation => "punctuation",
            Self::Tag => "tag",
            Self::Attribute => "attribute",
        }
    }

    pub(crate) const ALL: [Self; 16] = [
        Self::Keyword,
        Self::String,
        Self::StringSpecial,
        Self::Escape,
        Self::Number,
        Self::Constant,
        Self::Comment,
        Self::Function,
        Self::Type,
        Self::VariableBuiltin,
        Self::Parameter,
        Self::Property,
        Self::Operator,
        Self::Punctuation,
        Self::Tag,
        Self::Attribute,
    ];

    pub(crate) fn code(self) -> u8 {
        // Index + 1; 0 means "no class".
        Self::ALL
            .iter()
            .position(|c| *c == self)
            .map_or(0, |i| i as u8 + 1)
    }

    pub(crate) fn from_code(code: u8) -> Option<Self> {
        code.checked_sub(1)
            .and_then(|i| Self::ALL.get(usize::from(i)).copied())
    }
}

/// Maps a capture name to a token class. The most specific known prefix
/// wins (`function.method.builtin` → `function`). `None` means plain text.
pub(crate) fn class_for_capture(name: &str) -> Option<TokenClass> {
    let mut key = name;
    loop {
        if let Some(class) = exact(key) {
            return class;
        }
        let dot = key.rfind('.')?;
        key = &key[..dot];
    }
}

/// `Some(None)` = known capture that renders as plain text.
fn exact(name: &str) -> Option<Option<TokenClass>> {
    use TokenClass as T;
    Some(match name {
        "keyword" | "charset" | "import" | "keyframes" | "media" | "namespace" | "supports" => {
            Some(T::Keyword)
        }
        "string.special.key" => Some(T::Property),
        "string.special" | "string.regexp" => Some(T::StringSpecial),
        "string" => Some(T::String),
        "escape" => Some(T::Escape),
        "number" => Some(T::Number),
        "boolean" | "constant" => Some(T::Constant),
        "comment" => Some(T::Comment),
        "function" => Some(T::Function),
        "constructor" | "type" => Some(T::Type),
        "variable.builtin" => Some(T::VariableBuiltin),
        "variable.parameter" => Some(T::Parameter),
        "property" => Some(T::Property),
        "operator" => Some(T::Operator),
        "punctuation" => Some(T::Punctuation),
        "tag" => Some(T::Tag),
        "attribute" => Some(T::Attribute),
        "variable" | "embedded" => None,
        _ => return None,
    })
}

/// A highlighted byte range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HighlightSpan {
    /// Start byte.
    pub start: usize,
    /// End byte (exclusive).
    pub end: usize,
    /// Token class.
    pub class: TokenClass,
}

/// Renders spans as one readable line each (`L1:5 keyword "const"`), for
/// golden tests and debugging. Lines and columns are 1-based; columns count
/// chars.
pub fn render_spans(rope: &Rope, spans: &[HighlightSpan]) -> String {
    let mut out = String::new();
    for span in spans {
        let start_char = rope.byte_to_char(span.start);
        let line = rope.char_to_line(start_char);
        let col = start_char - rope.line_to_char(line);
        let text = rope
            .slice(start_char..rope.byte_to_char(span.end))
            .to_string();
        // Writing to a String cannot fail.
        let _ = writeln!(
            out,
            "L{}:{} {} {:?}",
            line + 1,
            col + 1,
            span.class.name(),
            text
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_mapping_uses_most_specific_prefix() {
        assert_eq!(
            class_for_capture("function.method.builtin"),
            Some(TokenClass::Function)
        );
        assert_eq!(
            class_for_capture("string.special.key"),
            Some(TokenClass::Property)
        );
        assert_eq!(
            class_for_capture("string.special.regex"),
            Some(TokenClass::StringSpecial)
        );
        assert_eq!(
            class_for_capture("punctuation.bracket"),
            Some(TokenClass::Punctuation)
        );
        assert_eq!(class_for_capture("variable"), None);
        assert_eq!(class_for_capture("variable.member"), None);
        assert_eq!(class_for_capture("unknown.thing"), None);
    }

    #[test]
    fn codes_round_trip() {
        for class in TokenClass::ALL {
            assert_eq!(TokenClass::from_code(class.code()), Some(class));
        }
        assert_eq!(TokenClass::from_code(0), None);
    }
}
