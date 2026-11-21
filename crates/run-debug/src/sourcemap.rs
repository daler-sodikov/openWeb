//! Source Map v3 decoding, in both directions (`docs/ARCHITECTURE.md`,
//! 4.10.3). A pure function of the map's text; reading files is in `core`.
//! `names` and `sourcesContent` are ignored. Positions are zero-based.

use serde::Deserialize;

use crate::paths;

/// A zero-based line and column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourcePosition {
    /// Line.
    pub line: u32,
    /// Column.
    pub column: u32,
}

/// Why a map was not accepted. Never a panic, whatever the input.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SourceMapError {
    /// Not JSON, or not shaped like a source map.
    #[error("not a source map: {0}")]
    Json(String),
    /// A version other than 3.
    #[error("unsupported source map version {0}")]
    Version(u32),
    /// An index map (`sections`) is not supported.
    #[error("index source maps are not supported")]
    Sections,
    /// A bad `mappings` string.
    #[error("bad mappings: {0}")]
    Mappings(&'static str),
    /// A bad `data:` URL.
    #[error("bad data URL: {0}")]
    DataUrl(&'static str),
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Raw {
    version: u32,
    #[serde(default)]
    sources: Vec<Option<String>>,
    #[serde(default)]
    source_root: Option<String>,
    #[serde(default)]
    mappings: String,
    #[serde(default)]
    sections: Option<serde::de::IgnoredAny>,
}

/// One mapping: generated position to original position.
#[derive(Debug, Clone, Copy)]
struct Segment {
    gen_line: u32,
    gen_col: u32,
    source: u32,
    line: u32,
    col: u32,
}

/// An original position, from [`SourceMap::original_for`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Original<'a> {
    /// The source, as in `sources()` (joined with `sourceRoot`).
    pub source: &'a str,
    /// Where in it.
    pub position: SourcePosition,
}

/// A generated position, from [`SourceMap::generated_for`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Generated {
    /// Where in the generated file.
    pub position: SourcePosition,
    /// The original line that has this code: the asked line, or the next one
    /// with a mapping (a breakpoint on a blank line moves down).
    pub original_line: u32,
}

/// A decoded map.
#[derive(Debug, Clone)]
pub struct SourceMap {
    sources: Vec<String>,
    /// Sorted by generated position.
    by_generated: Vec<Segment>,
    /// Sorted by source, original position, then generated position.
    by_original: Vec<Segment>,
}

impl SourceMap {
    /// Decodes a map from its JSON text.
    ///
    /// # Errors
    /// A [`SourceMapError`] for anything that is not a version 3 map.
    pub fn parse(json: &str) -> Result<Self, SourceMapError> {
        let raw: Raw =
            serde_json::from_str(json).map_err(|e| SourceMapError::Json(e.to_string()))?;
        if raw.sections.is_some() {
            return Err(SourceMapError::Sections);
        }
        if raw.version != 3 {
            return Err(SourceMapError::Version(raw.version));
        }
        let root = raw.source_root.as_deref().unwrap_or("");
        let sources: Vec<String> = raw
            .sources
            .iter()
            .map(|s| join_root(root, s.as_deref().unwrap_or("")))
            .collect();
        let by_generated = decode_mappings(&raw.mappings, sources.len())?;
        let mut by_original = by_generated.clone();
        by_original.sort_by_key(|s| (s.source, s.line, s.col, s.gen_line, s.gen_col));
        Ok(Self {
            sources,
            by_generated,
            by_original,
        })
    }

    /// Decodes an inline map: `data:application/json;base64,…` (what `tsx`
    /// and `ts-node` write into `sourceMappingURL`).
    ///
    /// # Errors
    /// A bad URL, or what [`SourceMap::parse`] says.
    pub fn from_data_url(url: &str) -> Result<Self, SourceMapError> {
        let rest = url
            .strip_prefix("data:")
            .ok_or(SourceMapError::DataUrl("not a data: URL"))?;
        let (head, body) = rest
            .split_once(',')
            .ok_or(SourceMapError::DataUrl("no comma"))?;
        if !head.split(';').any(|p| p == "base64") {
            return Err(SourceMapError::DataUrl("only base64 is supported"));
        }
        let bytes = base64(body)?;
        let text = String::from_utf8(bytes).map_err(|_| SourceMapError::DataUrl("not UTF-8"))?;
        Self::parse(&text)
    }

    /// The sources, each joined with `sourceRoot`; resolve them against the
    /// map's own location.
    pub fn sources(&self) -> &[String] {
        &self.sources
    }

    /// The original position of generated `line:column`: the last mapping at
    /// or before it on that line. `None` if the line has none before it.
    pub fn original_for(&self, line: u32, column: u32) -> Option<Original<'_>> {
        let end = self
            .by_generated
            .partition_point(|s| (s.gen_line, s.gen_col) <= (line, column));
        let s = self.by_generated.get(end.checked_sub(1)?)?;
        if s.gen_line != line {
            return None;
        }
        Some(Original {
            source: self.sources.get(s.source as usize)?,
            position: SourcePosition {
                line: s.line,
                column: s.col,
            },
        })
    }

    /// The generated position for `source:line:column`: the first mapping at
    /// or after it, so a line with no code gives the next line that has some.
    /// `source` must be one of [`SourceMap::sources`].
    pub fn generated_for(&self, source: &str, line: u32, column: u32) -> Option<Generated> {
        let index = u32::try_from(self.sources.iter().position(|s| s == source)?).ok()?;
        let at = self
            .by_original
            .partition_point(|s| (s.source, s.line, s.col) < (index, line, column));
        let s = self.by_original.get(at).filter(|s| s.source == index)?;
        Some(Generated {
            position: SourcePosition {
                line: s.gen_line,
                column: s.gen_col,
            },
            original_line: s.line,
        })
    }
}

/// `sourceRoot` and a source, joined with `/` unless the source is absolute
/// or a URL.
fn join_root(root: &str, source: &str) -> String {
    if root.is_empty() || source.contains("://") || paths::is_absolute(&paths::normalize(source)) {
        source.to_owned()
    } else {
        format!("{}/{source}", root.trim_end_matches('/'))
    }
}

fn decode_mappings(text: &str, source_count: usize) -> Result<Vec<Segment>, SourceMapError> {
    let mut out = Vec::new();
    let (mut source, mut line, mut col) = (0_i64, 0_i64, 0_i64);
    for (gen_line, group) in text.split(';').enumerate() {
        let gen_line =
            u32::try_from(gen_line).map_err(|_| SourceMapError::Mappings("too many lines"))?;
        let mut gen_col = 0_i64;
        for segment in group.split(',').filter(|s| !s.is_empty()) {
            let mut fields = [0_i64; 5];
            let mut count = 0;
            let mut rest = segment.as_bytes();
            while !rest.is_empty() {
                let (value, used) = vlq(rest)?;
                *fields
                    .get_mut(count)
                    .ok_or(SourceMapError::Mappings("more than five fields"))? = value;
                count += 1;
                rest = &rest[used..];
            }
            if !matches!(count, 1 | 4 | 5) {
                return Err(SourceMapError::Mappings("a segment has 2, 3 or no fields"));
            }
            gen_col = add(gen_col, fields[0])?;
            if count == 1 {
                continue; // generated code with no source
            }
            source = add(source, fields[1])?;
            line = add(line, fields[2])?;
            col = add(col, fields[3])?;
            if usize::try_from(source).map_or(true, |s| s >= source_count) {
                return Err(SourceMapError::Mappings("source index out of range"));
            }
            out.push(Segment {
                gen_line,
                gen_col: to_u32(gen_col)?,
                source: to_u32(source)?,
                line: to_u32(line)?,
                col: to_u32(col)?,
            });
        }
    }
    // Each line's segments are in order in valid maps; sort anyway, stably.
    out.sort_by_key(|s| (s.gen_line, s.gen_col));
    Ok(out)
}

fn add(a: i64, b: i64) -> Result<i64, SourceMapError> {
    a.checked_add(b)
        .ok_or(SourceMapError::Mappings("number too large"))
}

fn to_u32(v: i64) -> Result<u32, SourceMapError> {
    u32::try_from(v).map_err(|_| SourceMapError::Mappings("negative or too large position"))
}

/// One base64 VLQ number: the value and the bytes used.
fn vlq(bytes: &[u8]) -> Result<(i64, usize), SourceMapError> {
    let (mut value, mut shift) = (0_i64, 0_u32);
    for (i, &b) in bytes.iter().enumerate() {
        let digit = i64::from(b64_value(b).ok_or(SourceMapError::Mappings("not a base64 digit"))?);
        if shift > 40 {
            return Err(SourceMapError::Mappings("number too large"));
        }
        value |= (digit & 31) << shift;
        shift += 5;
        if digit & 32 == 0 {
            let magnitude = value >> 1;
            return Ok((
                if value & 1 == 1 {
                    -magnitude
                } else {
                    magnitude
                },
                i + 1,
            ));
        }
    }
    Err(SourceMapError::Mappings("number cut off"))
}

fn b64_value(b: u8) -> Option<u8> {
    match b {
        b'A'..=b'Z' => Some(b - b'A'),
        b'a'..=b'z' => Some(b - b'a' + 26),
        b'0'..=b'9' => Some(b - b'0' + 52),
        b'+' | b'-' => Some(62),
        b'/' | b'_' => Some(63),
        _ => None,
    }
}

/// Standard (or URL-safe) base64, padding optional.
fn base64(text: &str) -> Result<Vec<u8>, SourceMapError> {
    let (mut out, mut acc, mut bits) = (Vec::new(), 0_u32, 0_u32);
    for b in text.trim_end_matches('=').bytes() {
        let v = b64_value(b).ok_or(SourceMapError::DataUrl("not base64"))?;
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(u8::try_from((acc >> bits) & 0xff).unwrap_or(0));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real `tsc` 6.0.3 output for fixtures/debug-app (see its README).
    const MAP: &str = include_str!("../../../fixtures/debug-app/dist/app.js.map");
    const SRC: &str = "../src/app.ts";

    fn at(line: u32, column: u32) -> SourcePosition {
        SourcePosition { line, column }
    }

    #[test]
    fn maps_generated_to_original_on_the_fixture() {
        let map = SourceMap::parse(MAP).unwrap();
        assert_eq!(map.sources(), [SRC]);
        // `sum += item.price;` is generated line 4, column 8, original line 9.
        let o = map.original_for(4, 8).unwrap();
        assert_eq!(o.source, SRC);
        assert_eq!(o.position, at(9, 4));
        // Inside the statement: the last mapping before the column.
        assert_eq!(map.original_for(4, 12).unwrap().position.line, 9);
        // `return sum;`
        assert_eq!(map.original_for(6, 4).unwrap().position.line, 12);
        // Line 0 is `"use strict"` with no mapping on it; nothing past the end.
        assert!(map.original_for(0, 0).is_none());
        assert!(map.original_for(99, 0).is_none());
    }

    #[test]
    fn maps_original_to_generated_on_the_fixture() {
        let map = SourceMap::parse(MAP).unwrap();
        let g = map.generated_for(SRC, 9, 0).unwrap();
        assert_eq!(g.position.line, 4);
        assert_eq!(g.original_line, 9);
        // The blank line 11 has no code: the breakpoint moves to line 12.
        let g = map.generated_for(SRC, 11, 0).unwrap();
        assert_eq!(g.original_line, 12);
        assert_eq!(g.position.line, 6);
        // The interface lines 1-4
        // have no code and move down to the function.
        assert_eq!(map.generated_for(SRC, 2, 0).unwrap().original_line, 6);
        // Past the last mapping, and an unknown source: nothing.
        assert!(map.generated_for(SRC, 500, 0).is_none());
        assert!(map.generated_for("other.ts", 8, 0).is_none());
    }

    #[test]
    fn both_directions_agree_for_every_mapping() {
        let map = SourceMap::parse(MAP).unwrap();
        for s in &map.by_generated {
            let o = map.original_for(s.gen_line, s.gen_col).unwrap();
            let g = map
                .generated_for(o.source, o.position.line, o.position.column)
                .unwrap();
            assert_eq!(g.original_line, o.position.line);
        }
    }

    #[test]
    fn vlq_examples_from_the_specification() {
        for (text, value) in [
            ("A", 0),
            ("C", 1),
            ("D", -1),
            ("gB", 16),
            ("hB", -16),
            ("2H", 123),
        ] {
            assert_eq!(vlq(text.as_bytes()).unwrap(), (value, text.len()), "{text}");
        }
        assert!(vlq(b"g").is_err(), "continuation with nothing after it");
        assert!(vlq(b"!").is_err());
    }

    #[test]
    fn the_specification_example_of_a_blank_line_and_source_root() {
        // Two generated lines; the second has its code at column 4, and the
        // original has a blank line between (line 0 and line 2 are mapped).
        let json = r#"{"version":3,"sourceRoot":"/root/","sources":["a.ts","/abs/b.ts","http://h/c.ts"],
            "mappings":"AAAA;IAEA,CCAA;ADAA"}"#;
        let map = SourceMap::parse(json).unwrap();
        assert_eq!(map.sources(), ["/root/a.ts", "/abs/b.ts", "http://h/c.ts"]);
        let g = map.generated_for("/root/a.ts", 1, 0).unwrap();
        assert_eq!((g.original_line, g.position), (2, at(1, 4)));
        assert_eq!(map.original_for(1, 5).unwrap().source, "/abs/b.ts");
    }

    #[test]
    fn an_inline_data_url_is_decoded() {
        use std::fmt::Write;
        let json = r#"{"version":3,"sources":["x.ts"],"mappings":"AAAA"}"#;
        let mut encoded = String::new();
        let table = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        for chunk in json.as_bytes().chunks(3) {
            let n = chunk
                .iter()
                .enumerate()
                .fold(0_u32, |a, (i, b)| a | (u32::from(*b) << (16 - 8 * i)));
            for i in 0..=chunk.len() {
                encoded.push(char::from(table[((n >> (18 - 6 * i)) & 63) as usize]));
            }
            for _ in chunk.len()..3 {
                encoded.push('=');
            }
        }
        let mut url = String::from("data:application/json;charset=utf-8;base64,");
        write!(url, "{encoded}").unwrap();
        let map = SourceMap::from_data_url(&url).unwrap();
        assert_eq!(map.original_for(0, 0).unwrap().source, "x.ts");
        assert!(SourceMap::from_data_url("data:application/json,{}").is_err());
        assert!(SourceMap::from_data_url("file:///a.map").is_err());
        assert!(SourceMap::from_data_url("data:application/json;base64,@@@").is_err());
    }

    #[test]
    fn malformed_maps_are_errors() {
        for json in [
            "",
            "null",
            "[]",
            r#"{"version":2,"mappings":""}"#,
            r#"{"version":3,"sections":[]}"#,
            r#"{"version":3,"sources":["a"],"mappings":"AAAA,AA"}"#,
            r#"{"version":3,"sources":["a"],"mappings":"AAAAAAAA"}"#,
            r#"{"version":3,"sources":["a"],"mappings":"AACA;AAAA,AAAD"}"#,
            r#"{"version":3,"sources":["a"],"mappings":"ACAA"}"#,
            r#"{"version":3,"sources":["a"],"mappings":"AAAA,g"}"#,
            r#"{"version":3,"sources":["a"],"mappings":"AAA?"}"#,
            r#"{"version":3,"sources":["a"],"mappings":"/////////////////////A"}"#,
        ] {
            assert!(SourceMap::parse(json).is_err(), "accepted: {json}");
        }
        // Empty and sparse maps are fine.
        let map = SourceMap::parse(r#"{"version":3,"sources":[],"mappings":""}"#).unwrap();
        assert!(map.original_for(0, 0).is_none());
        let map =
            SourceMap::parse(r#"{"version":3,"sources":["a"],"mappings":";;A,AAAA"}"#).unwrap();
        assert_eq!(map.original_for(2, 5).unwrap().position, at(0, 0));
    }

    #[test]
    fn damaged_copies_of_a_real_map_never_panic() {
        let bytes = MAP.as_bytes();
        let mut seed = 0x9E37_79B9_u32;
        let mut next = move || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as usize
        };
        for _ in 0..2000 {
            let mut copy = bytes.to_vec();
            match next() % 3 {
                0 => copy.truncate(next() % bytes.len()),
                1 => {
                    for _ in 0..=(next() % 4) {
                        let at = next() % copy.len();
                        copy[at] = b"AB;,/+=\"{}gZ9-"[next() % 14];
                    }
                }
                _ => {
                    let at = next() % copy.len();
                    copy.remove(at);
                }
            }
            if let Ok(text) = String::from_utf8(copy)
                && let Ok(map) = SourceMap::parse(&text)
            {
                let _ = map.original_for(4, 8);
                let _ = map.generated_for(SRC, 9, 0);
            }
        }
    }
}
