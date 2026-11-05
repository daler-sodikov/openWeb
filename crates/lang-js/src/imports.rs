//! In-process import analysis with `oxc` (`docs/ARCHITECTURE.md`, 4.8):
//! unused imports and sorting the import block, both without waiting for
//! the language server.

use std::path::Path;

use oxc_allocator::Allocator;
use oxc_ast::ast::{ImportDeclaration, ImportDeclarationSpecifier, Program, Statement};
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::{GetSpan, SourceType};

/// An unused import binding, or a whole unused import declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnusedImport {
    /// Start byte.
    pub start: usize,
    /// End byte.
    pub end: usize,
    /// The local names that are never used.
    pub names: Vec<String>,
}

/// A replacement of the import block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportEdit {
    /// Start byte.
    pub start: usize,
    /// End byte.
    pub end: usize,
    /// New text.
    pub text: String,
}

fn source_type(path: &Path) -> SourceType {
    SourceType::from_path(path).unwrap_or_else(|_| SourceType::tsx())
}

/// Parses `text` and calls `f` with the program; `None` when it does not
/// parse (unused-import results on broken code would be noise).
fn with_program<T>(text: &str, path: &Path, f: impl FnOnce(&Program<'_>) -> T) -> Option<T> {
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, text, source_type(path)).parse();
    if parsed.fatal_error || parsed.diagnostics.has_errors() {
        return None;
    }
    Some(f(&parsed.program))
}

/// Imports whose bindings are never referenced. A declaration whose
/// bindings are all unused is reported as a whole; side-effect imports
/// (`import "./a.css"`) are never unused.
pub fn unused_imports(text: &str, path: &Path) -> Vec<UnusedImport> {
    with_program(text, path, |program| {
        let semantic = SemanticBuilder::new().build(program).semantic;
        let scoping = semantic.scoping();
        let mut result = Vec::new();
        for statement in &program.body {
            let Statement::ImportDeclaration(decl) = statement else {
                continue;
            };
            let Some(specifiers) = &decl.specifiers else {
                continue;
            };
            let mut unused = Vec::new();
            for specifier in specifiers {
                let local = match specifier {
                    ImportDeclarationSpecifier::ImportSpecifier(s) => &s.local,
                    ImportDeclarationSpecifier::ImportDefaultSpecifier(s) => &s.local,
                    ImportDeclarationSpecifier::ImportNamespaceSpecifier(s) => &s.local,
                };
                let used = local.symbol_id.get().is_some_and(|id| {
                    scoping
                        .get_resolved_reference_ids(id)
                        .iter()
                        .next()
                        .is_some()
                });
                if !used {
                    unused.push((specifier.span(), local.name.to_string()));
                }
            }
            if unused.is_empty() {
                continue;
            }
            if unused.len() == specifiers.len() {
                result.push(UnusedImport {
                    start: decl.span.start as usize,
                    end: decl.span.end as usize,
                    names: unused.into_iter().map(|(_, name)| name).collect(),
                });
            } else {
                result.extend(unused.into_iter().map(|(span, name)| UnusedImport {
                    start: span.start as usize,
                    end: span.end as usize,
                    names: vec![name],
                }));
            }
        }
        result
    })
    .unwrap_or_default()
}

/// Sorts the leading import declarations by module specifier: packages
/// first, then relative paths, each alphabetically; stable for equal
/// specifiers. Side-effect imports stay where they are and split the block
/// into runs sorted separately. `None` if nothing changes, the file does
/// not parse, or comments sit between the imports.
pub fn sort_imports(text: &str, path: &Path) -> Option<ImportEdit> {
    with_program(text, path, |program| {
        let imports: Vec<&ImportDeclaration<'_>> = program
            .body
            .iter()
            .map_while(|s| match s {
                Statement::ImportDeclaration(decl) => Some(&**decl),
                _ => None,
            })
            .collect();
        let (first, last) = (imports.first()?, imports.last()?);
        let start = first.span.start as usize;
        let end = last.span.end as usize;
        // Only whitespace may separate the declarations (comments stay put
        // by bailing out rather than being moved with the wrong import).
        for pair in imports.windows(2) {
            let gap = &text[pair[0].span.end as usize..pair[1].span.start as usize];
            if !gap.trim().is_empty() {
                return None;
            }
        }
        let key = |decl: &ImportDeclaration<'_>| {
            let source = decl.source.value.as_str().unwrap_or_default();
            let relative = source.starts_with('.');
            (relative, source.to_ascii_lowercase(), source.to_owned())
        };
        // Spans of the declarations in their new order.
        let mut order: Vec<(usize, usize)> = Vec::with_capacity(imports.len());
        let mut run: Vec<&ImportDeclaration<'_>> = Vec::new();
        let flush = |run: &mut Vec<&ImportDeclaration<'_>>, order: &mut Vec<(usize, usize)>| {
            run.sort_by_key(|decl| key(decl));
            order.extend(
                run.drain(..)
                    .map(|d| (d.span.start as usize, d.span.end as usize)),
            );
        };
        for decl in &imports {
            if decl.specifiers.is_none() {
                flush(&mut run, &mut order);
                order.push((decl.span.start as usize, decl.span.end as usize));
            } else {
                run.push(decl);
            }
        }
        flush(&mut run, &mut order);
        let lines: Vec<&str> = order.iter().map(|&(a, b)| &text[a..b]).collect();
        let sorted = lines.join("\n");
        (sorted != text[start..end]).then_some(ImportEdit {
            start,
            end,
            text: sorted,
        })
    })
    .flatten()
}
