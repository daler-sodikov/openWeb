//! EditorConfig (<https://editorconfig.org>) support.
//!
//! For a file, `.editorconfig` files are read from its directory upwards
//! until one says `root = true` (or the file system root). Files closer to
//! the edited file win; within a file, later sections win. Section globs
//! without `/` match the file name at any depth; globs with `/` are relative
//! to the `.editorconfig` file. Supported: `*`, `**`, `?`, `[set]`,
//! `[!set]`, `{a,b}`. Not supported yet: numeric ranges `{1..3}`.

use std::fs;
use std::path::{Path, PathBuf};

use globset::GlobBuilder;

/// `indent_style`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndentStyle {
    /// Indent with tabs.
    Tab,
    /// Indent with spaces.
    Space,
}

/// `end_of_line`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndOfLine {
    /// `\n`
    Lf,
    /// `\r\n`
    CrLf,
    /// `\r` (old Mac; not supported for saving, reported only).
    Cr,
}

/// Properties that apply to one file. `None` means "not set".
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EditorConfig {
    /// `indent_style`.
    pub indent_style: Option<IndentStyle>,
    /// `indent_size` as a number (`tab` resolves to `tab_width`).
    pub indent_size: Option<u32>,
    /// `tab_width` (defaults to `indent_size` when that is a number).
    pub tab_width: Option<u32>,
    /// `end_of_line`.
    pub end_of_line: Option<EndOfLine>,
    /// `charset`, lowercase (`utf-8`, `utf-8-bom`, `latin1`, ...).
    pub charset: Option<String>,
    /// `trim_trailing_whitespace`.
    pub trim_trailing_whitespace: Option<bool>,
    /// `insert_final_newline`.
    pub insert_final_newline: Option<bool>,
}

impl EditorConfig {
    /// The indentation unit implied by the settings, if any. `fallback` is
    /// the unit detected from the file.
    pub fn indent_unit(&self, fallback: &str) -> String {
        let width = self.indent_size.or(self.tab_width);
        match (self.indent_style, width) {
            (Some(IndentStyle::Tab), _) => "\t".to_owned(),
            (Some(IndentStyle::Space), Some(n)) => " ".repeat(n as usize),
            (Some(IndentStyle::Space), None) if fallback == "\t" => "    ".to_owned(),
            (None, Some(n)) if fallback != "\t" => " ".repeat(n as usize),
            _ => fallback.to_owned(),
        }
    }

    /// Display width of a tab character.
    pub fn display_tab_width(&self) -> u32 {
        self.tab_width.or(self.indent_size).unwrap_or(4)
    }
}

/// One parsed section: glob and `key = value` pairs (keys lowercase).
struct Section {
    glob: String,
    pairs: Vec<(String, String)>,
}

struct ConfigFile {
    dir: PathBuf,
    root: bool,
    sections: Vec<Section>,
}

fn parse(dir: &Path, text: &str) -> ConfigFile {
    let mut root = false;
    let mut sections: Vec<Section> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(glob) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            sections.push(Section {
                glob: glob.to_owned(),
                pairs: Vec::new(),
            });
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim().to_owned();
        match sections.last_mut() {
            Some(section) => section.pairs.push((key, value)),
            None if key == "root" => root = value.eq_ignore_ascii_case("true"),
            None => {}
        }
    }
    ConfigFile {
        dir: dir.to_path_buf(),
        root,
        sections,
    }
}

fn glob_matches(glob: &str, rel_path: &str) -> bool {
    let pattern = if let Some(stripped) = glob.strip_prefix('/') {
        stripped.to_owned()
    } else if glob.contains('/') {
        glob.to_owned()
    } else {
        format!("**/{glob}")
    };
    match GlobBuilder::new(&pattern).literal_separator(true).build() {
        Ok(g) => g.compile_matcher().is_match(rel_path),
        Err(err) => {
            tracing::debug!(%err, glob, "unsupported EditorConfig glob");
            false
        }
    }
}

fn apply(config: &mut EditorConfig, key: &str, value: &str) {
    let lower = value.to_ascii_lowercase();
    let unset = lower == "unset";
    let bool_value = match lower.as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    };
    match key {
        "indent_style" => {
            config.indent_style = match lower.as_str() {
                "tab" => Some(IndentStyle::Tab),
                "space" => Some(IndentStyle::Space),
                _ => None,
            }
        }
        "indent_size" => {
            config.indent_size = if lower == "tab" {
                config.tab_width
            } else {
                lower.parse().ok().filter(|n| *n > 0)
            }
        }
        "tab_width" => config.tab_width = lower.parse().ok().filter(|n| *n > 0),
        "end_of_line" => {
            config.end_of_line = match lower.as_str() {
                "lf" => Some(EndOfLine::Lf),
                "crlf" => Some(EndOfLine::CrLf),
                "cr" => Some(EndOfLine::Cr),
                _ => None,
            }
        }
        "charset" => config.charset = (!unset).then_some(lower.clone()),
        "trim_trailing_whitespace" => config.trim_trailing_whitespace = bool_value,
        "insert_final_newline" => config.insert_final_newline = bool_value,
        _ => {}
    }
}

/// Resolves the EditorConfig properties for `file` (an absolute path).
pub fn resolve(file: &Path) -> EditorConfig {
    let mut files = Vec::new();
    let mut dir = file.parent();
    while let Some(d) = dir {
        let candidate = d.join(".editorconfig");
        if let Ok(text) = fs::read_to_string(&candidate) {
            let parsed = parse(d, &text);
            let root = parsed.root;
            files.push(parsed);
            if root {
                break;
            }
        }
        dir = d.parent();
    }
    let mut config = EditorConfig::default();
    // Farthest first, so nearer files override.
    for cfg in files.iter().rev() {
        let Ok(rel) = file.strip_prefix(&cfg.dir) else {
            continue;
        };
        let rel = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        for section in &cfg.sections {
            if glob_matches(&section.glob, &rel) {
                for (key, value) in &section.pairs {
                    apply(&mut config, key, value);
                }
            }
        }
    }
    if config.tab_width.is_none() {
        config.tab_width = config.indent_size;
    }
    config
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_file_wins_and_root_stops() {
        let dir = tempfile::tempdir().unwrap();
        let top = dir.path();
        // Above the root: must be ignored.
        fs::write(
            top.join(".editorconfig"),
            "[*]\nindent_size = 8\ncharset = latin1\n",
        )
        .unwrap();
        let project = top.join("project");
        fs::create_dir_all(project.join("src/deep")).unwrap();
        fs::write(
            project.join(".editorconfig"),
            "root = true\n\n[*]\nindent_style = space\nindent_size = 2\nend_of_line = lf\n\
             trim_trailing_whitespace = true\ninsert_final_newline = true\n\n\
             [*.{md,markdown}]\ntrim_trailing_whitespace = false\n\n[Makefile]\nindent_style = tab\n",
        )
        .unwrap();
        fs::write(
            project.join("src/.editorconfig"),
            "[*.ts]\nindent_size = 4\n",
        )
        .unwrap();

        let ts = resolve(&project.join("src/deep/a.ts"));
        assert_eq!(ts.indent_style, Some(IndentStyle::Space));
        assert_eq!(ts.indent_size, Some(4), "nearer file wins");
        assert_eq!(ts.tab_width, Some(4));
        assert_eq!(ts.end_of_line, Some(EndOfLine::Lf));
        assert_eq!(ts.trim_trailing_whitespace, Some(true));
        assert_eq!(ts.charset, None, "file above root = true is ignored");
        assert_eq!(ts.indent_unit("\t"), "    ");

        let md = resolve(&project.join("README.md"));
        assert_eq!(
            md.trim_trailing_whitespace,
            Some(false),
            "later section wins"
        );
        assert_eq!(md.indent_size, Some(2));

        let make = resolve(&project.join("Makefile"));
        assert_eq!(make.indent_style, Some(IndentStyle::Tab));
        assert_eq!(make.indent_unit("  "), "\t");
    }

    #[test]
    fn globs_with_slashes_are_relative() {
        assert!(glob_matches("*.ts", "src/a.ts"));
        assert!(!glob_matches("src/*.ts", "lib/src/a.ts"));
        assert!(glob_matches("src/*.ts", "src/a.ts"));
        assert!(!glob_matches("src/*.ts", "src/x/a.ts"));
        assert!(
            glob_matches("src/**.ts", "src/x/a.ts") || glob_matches("src/**/*.ts", "src/x/a.ts")
        );
        assert!(glob_matches("[!a]*.js", "b.js"));
        assert!(glob_matches("/package.json", "package.json"));
    }

    #[test]
    fn no_files_means_nothing_set() {
        let dir = tempfile::tempdir().unwrap();
        let config = resolve(&dir.path().join("x.ts"));
        assert_eq!(config, EditorConfig::default());
        assert_eq!(config.indent_unit("  "), "  ");
        assert_eq!(config.display_tab_width(), 4);
    }
}
