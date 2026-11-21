//! Log points: a message with `{expressions}` turned into a breakpoint
//! condition that prints and never pauses (`docs/ARCHITECTURE.md`, 4.10.2).

/// Turns an IntelliJ-style message (`text {expr} text`) into a condition:
///
/// ```text
/// (()=>{try{console.log(`text ${(expr
/// )} text`)}catch(e){}return false})()
/// ```
///
/// The condition is always false, so the program never stops, and an
/// expression that throws prints nothing.
///
/// The message is data: its backslashes, backticks and `$` are escaped, so
/// it cannot end the template. The only way to evaluate anything is a
/// `{expr}`, and an expression is used only if it is plain enough to stay
/// inside `${( … )}`: its quotes, parentheses, brackets and braces must be
/// balanced, and it must have no backtick, comment or line break
/// introducing character. Anything else (an unclosed `{`, `{}`, `{a//b}`, a
/// stray `)`) is printed as text. `\{` writes a literal `{`.
pub fn logpoint_condition(message: &str) -> String {
    let mut body = String::new();
    let chars: Vec<char> = message.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' && chars.get(i + 1) == Some(&'{') {
            body.push('{');
            i += 2;
        } else if c == '{' {
            match expression_end(&chars, i + 1) {
                Some(end) => {
                    let expr: String = chars[i + 1..end].iter().collect();
                    // The newline ends a `//` that slipped through, so it
                    // can never swallow the closing parenthesis.
                    body.push_str("${(");
                    body.push_str(&expr);
                    body.push_str("\n)}");
                    i = end + 1;
                }
                None => {
                    body.push('{');
                    i += 1;
                }
            }
        } else {
            push_escaped(&mut body, c);
            i += 1;
        }
    }
    format!("(()=>{{try{{console.log(`{body}`)}}catch(e){{}}return false}})()")
}

/// Appends `c` so that it is literal text inside a template literal.
fn push_escaped(out: &mut String, c: char) {
    match c {
        '\\' => out.push_str("\\\\"),
        '`' => out.push_str("\\`"),
        '$' => out.push_str("\\$"),
        '\r' => out.push_str("\\r"),
        other => out.push(other),
    }
}

/// The index of the `}` that closes the expression starting at `start`, if
/// what lies between is a safe, non-empty expression.
fn expression_end(chars: &[char], start: usize) -> Option<usize> {
    let mut stack: Vec<char> = Vec::new();
    let mut quote: Option<char> = None;
    let mut i = start;
    while i < chars.len() {
        let c = chars[i];
        if let Some(q) = quote {
            if c == '\\' {
                i += 1;
            } else if c == q {
                quote = None;
            } else if c == '\n' {
                return None;
            }
        } else {
            match c {
                '`' => return None,
                '/' if matches!(chars.get(i + 1), Some('/' | '*')) => return None,
                '\'' | '"' => quote = Some(c),
                '(' => stack.push(')'),
                '[' => stack.push(']'),
                '{' => stack.push('}'),
                ')' | ']' | '}' => match stack.pop() {
                    Some(expected) if expected == c => {}
                    None if c == '}' => {
                        let blank = chars[start..i].iter().all(|ch| ch.is_whitespace());
                        return if blank { None } else { Some(i) };
                    }
                    _ => return None,
                },
                _ => {}
            }
        }
        i += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEAD: &str = "(()=>{try{console.log(`";
    const TAIL: &str = "`)}catch(e){}return false})()";

    fn body(message: &str) -> String {
        let out = logpoint_condition(message);
        out.strip_prefix(HEAD)
            .and_then(|r| r.strip_suffix(TAIL))
            .map(str::to_owned)
            .unwrap_or_else(|| panic!("unexpected shape: {out}"))
    }

    #[test]
    fn plain_text_is_the_message() {
        assert_eq!(body("hello"), "hello");
        assert_eq!(body(""), "");
    }

    #[test]
    fn an_expression_is_evaluated_inside_the_template() {
        assert_eq!(body("i={i}"), "i=${(i\n)}");
        assert_eq!(body("{a} and {b.c}"), "${(a\n)} and ${(b.c\n)}");
        assert_eq!(body("{ {x:1}.x }"), "${( {x:1}.x \n)}");
        assert_eq!(body("{f(1, ')')}"), "${(f(1, ')')\n)}");
        assert_eq!(body("{xs[0]}"), "${(xs[0]\n)}");
    }

    #[test]
    fn the_condition_never_pauses_and_swallows_errors() {
        let out = logpoint_condition("{a.b.c}");
        assert!(out.ends_with("catch(e){}return false})()"));
        assert!(out.starts_with("(()=>{try{"));
    }

    #[test]
    fn backticks_dollars_and_backslashes_in_the_text_are_escaped() {
        assert_eq!(body("a`b"), "a\\`b");
        assert_eq!(body("cost $5"), "cost \\$5");
        assert_eq!(body(r"c:\dir"), r"c:\\dir");
        assert_eq!(body("line\r"), "line\\r");
    }

    #[test]
    fn dollar_brace_in_the_text_is_not_an_injection_point() {
        // The `$` is escaped; `{process.exit()}` is then the user's own
        // expression, written as a log point expression, and is handled as one.
        assert_eq!(body("${x}"), "\\$${(x\n)}");
        // A message that tries to close the template and the call.
        let hostile = "`);process.exit(1);//";
        assert_eq!(body(hostile), "\\`);process.exit(1);//");
        let whole = logpoint_condition(hostile);
        assert_eq!(
            whole.matches('`').count(),
            3,
            "two delimiters and one escaped: {whole}"
        );
    }

    #[test]
    fn hostile_expressions_stay_text_or_inside_the_substitution() {
        // Each of these cannot be a safe expression, so it is printed as text.
        for message in [
            "{`}",
            "{a`b}",
            "{a//b}",
            "{a/*b}",
            "{a)}",
            "{(a}",
            "{a]}",
            "{'unclosed}",
            "{\"x}",
            "{",
            "{}",
            "{   }",
            "{a",
            "}",
        ] {
            let out = logpoint_condition(message);
            assert!(!out.contains("${("), "{message:?} became code: {out}");
            assert!(
                out.starts_with(HEAD) && out.ends_with(TAIL),
                "{message:?}: {out}"
            );
        }
        // An attempt to leave the substitution early: the first brace stays
        // text, and only the well-formed `{2}` is an expression.
        assert_eq!(
            body("{1)}); process.exit(); (({2}"),
            "{1)}); process.exit(); ((${(2\n)}"
        );
    }

    #[test]
    fn a_hostile_message_never_adds_code_outside_the_substitutions() {
        let out = logpoint_condition("x`+(process.exit(),`{ok}`}{a}");
        // Remove every `${( … \n)}` and every escaped character; what is
        // left of the template must not contain an unescaped backtick or `$`.
        let inner = out
            .strip_prefix(HEAD)
            .and_then(|r| r.strip_suffix(TAIL))
            .unwrap_or_default();
        let mut text = String::new();
        let mut rest = inner;
        while let Some(at) = rest.find("${(") {
            text.push_str(&rest[..at]);
            let Some(end) = rest[at..].find("\n)}") else {
                panic!("unterminated substitution in {out}");
            };
            rest = &rest[at + end + 3..];
        }
        text.push_str(rest);
        let mut escaped = false;
        for c in text.chars() {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else {
                assert!(c != '`' && c != '$', "unescaped {c:?} in {out}");
            }
        }
        assert!(!escaped);
    }

    #[test]
    fn an_escaped_brace_is_a_literal_brace() {
        assert_eq!(body(r"\{not an expression}"), "{not an expression}");
        assert_eq!(body("{a}{b}"), "${(a\n)}${(b\n)}");
    }

    #[test]
    fn non_ascii_text_is_kept() {
        assert_eq!(body("значение {x} ✓"), "значение ${(x\n)} ✓");
    }
}
