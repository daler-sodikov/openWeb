//! Mapping between domain types and IPC DTOs.

use ferrule_buffer::{BufferError, EditKind, SelRange, Selection};
use ferrule_ipc as ipc;
use ferrule_ipc::{IpcError, IpcErrorKind};
use ferrule_project::ProjectError;
use ferrule_syntax::TokenClass;
use ferrule_vfs::VfsError;

pub(crate) fn to_usize(v: u32) -> usize {
    v as usize
}

/// Offsets are bounded by the 20 MB file limit, so they always fit in u32;
/// saturate defensively instead of panicking.
pub(crate) fn to_u32(v: usize) -> u32 {
    u32::try_from(v).unwrap_or(u32::MAX)
}

pub(crate) fn selection_from_ipc(sel: &ipc::EditorSelection) -> Selection {
    Selection {
        ranges: sel
            .ranges
            .iter()
            .map(|r| SelRange::new(to_usize(r.anchor), to_usize(r.head)))
            .collect(),
        main: to_usize(sel.main),
    }
}

pub(crate) fn selection_to_ipc(sel: &Selection) -> ipc::EditorSelection {
    ipc::EditorSelection {
        ranges: sel
            .ranges
            .iter()
            .map(|r| ipc::SelectionRange {
                anchor: to_u32(r.anchor),
                head: to_u32(r.head),
            })
            .collect(),
        main: to_u32(sel.main),
    }
}

pub(crate) fn edit_from_ipc(e: &ipc::TextEdit) -> ferrule_buffer::Edit {
    ferrule_buffer::Edit::new(to_usize(e.from), to_usize(e.to), e.insert.clone())
}

pub(crate) fn edit_to_ipc(e: &ferrule_buffer::Edit) -> ipc::TextEdit {
    ipc::TextEdit {
        from: to_u32(e.from),
        to: to_u32(e.to),
        insert: e.insert.clone(),
    }
}

pub(crate) fn kind_from_ipc(kind: ipc::EditKind) -> EditKind {
    match kind {
        ipc::EditKind::Typing => EditKind::Typing,
        ipc::EditKind::Deleting => EditKind::Deleting,
        ipc::EditKind::Other => EditKind::Other,
    }
}

pub(crate) fn token_to_ipc(class: TokenClass) -> ipc::TokenClass {
    use ipc::TokenClass as T;
    match class {
        TokenClass::Keyword => T::Keyword,
        TokenClass::String => T::String,
        TokenClass::StringSpecial => T::StringSpecial,
        TokenClass::Escape => T::Escape,
        TokenClass::Number => T::Number,
        TokenClass::Constant => T::Constant,
        TokenClass::Comment => T::Comment,
        TokenClass::Function => T::Function,
        TokenClass::Type => T::Type,
        TokenClass::VariableBuiltin => T::VariableBuiltin,
        TokenClass::Parameter => T::Parameter,
        TokenClass::Property => T::Property,
        TokenClass::Operator => T::Operator,
        TokenClass::Punctuation => T::Punctuation,
        TokenClass::Tag => T::Tag,
        TokenClass::Attribute => T::Attribute,
        TokenClass::Variable => T::Variable,
        TokenClass::Heading => T::Heading,
        TokenClass::Emphasis => T::Emphasis,
        TokenClass::Strong => T::Strong,
        TokenClass::Link => T::Link,
    }
}

pub(crate) fn buffer_error(err: BufferError) -> IpcError {
    let kind = match err {
        BufferError::VersionMismatch { .. } => IpcErrorKind::Conflict,
        BufferError::InvalidEdit(_) => IpcErrorKind::InvalidArgument,
    };
    IpcError::new(kind, err.to_string())
}

pub(crate) fn project_error(err: ProjectError) -> IpcError {
    let kind = match &err {
        ProjectError::Io(io) if io.kind() == std::io::ErrorKind::NotFound => IpcErrorKind::NotFound,
        ProjectError::Io(_) => IpcErrorKind::Io,
        ProjectError::NotADirectory(_) | ProjectError::InvalidPath(_) => {
            IpcErrorKind::InvalidArgument
        }
    };
    IpcError::new(kind, err.to_string())
}

pub(crate) fn vfs_error(err: VfsError) -> IpcError {
    let kind = match &err {
        VfsError::Io(io) if io.kind() == std::io::ErrorKind::NotFound => IpcErrorKind::NotFound,
        VfsError::Io(_) => IpcErrorKind::Io,
        VfsError::TooLarge { .. } | VfsError::Unsupported => IpcErrorKind::InvalidArgument,
    };
    IpcError::new(kind, err.to_string())
}

pub(crate) fn io_error(err: &std::io::Error) -> IpcError {
    IpcError::new(IpcErrorKind::Io, err.to_string())
}

pub(crate) fn action_to_ipc(action: ferrule_settings::Action) -> ipc::KeyAction {
    use ferrule_settings::Action as A;
    use ipc::KeyAction as K;
    match action {
        A::Save => K::Save,
        A::Undo => K::Undo,
        A::Redo => K::Redo,
        A::AddCaretAbove => K::AddCaretAbove,
        A::AddCaretBelow => K::AddCaretBelow,
        A::SelectNextOccurrence => K::SelectNextOccurrence,
        A::SelectAllOccurrences => K::SelectAllOccurrences,
        A::CloseTab => K::CloseTab,
        A::SearchEverywhere => K::SearchEverywhere,
        A::GotoFile => K::GotoFile,
        A::GotoSymbol => K::GotoSymbol,
        A::GotoAction => K::GotoAction,
        A::GotoLine => K::GotoLine,
        A::RecentFiles => K::RecentFiles,
        A::FindInFiles => K::FindInFiles,
        A::ReplaceInFiles => K::ReplaceInFiles,
        A::OpenFolder => K::OpenFolder,
        A::SplitRight => K::SplitRight,
        A::FoldCollapse => K::FoldCollapse,
        A::FoldExpand => K::FoldExpand,
        A::FoldCollapseAll => K::FoldCollapseAll,
        A::FoldExpandAll => K::FoldExpandAll,
        A::GotoMatchingBrace => K::GotoMatchingBrace,
        A::QuickDocumentation => K::QuickDocumentation,
        A::GotoDeclaration => K::GotoDeclaration,
        A::GotoTypeDeclaration => K::GotoTypeDeclaration,
        A::GotoImplementation => K::GotoImplementation,
        A::ToggleProblems => K::ToggleProblems,
        A::RestartTypeScriptServer => K::RestartTypeScriptServer,
    }
}

fn modifier_to_ipc(m: ferrule_settings::Modifier) -> ipc::Modifier {
    match m {
        ferrule_settings::Modifier::Control => ipc::Modifier::Control,
        ferrule_settings::Modifier::Alt => ipc::Modifier::Alt,
        ferrule_settings::Modifier::Shift => ipc::Modifier::Shift,
    }
}

pub(crate) fn keymap_to_ipc(bindings: Vec<ferrule_settings::KeyBinding>) -> Vec<ipc::KeyBinding> {
    use ferrule_settings::KeyStroke;
    bindings
        .into_iter()
        .map(|b| ipc::KeyBinding {
            action: action_to_ipc(b.action),
            stroke: match b.stroke {
                KeyStroke::Chord(keys) => ipc::KeyStroke::Chord { keys: keys.into() },
                KeyStroke::DoubleModifier(m, key) => ipc::KeyStroke::DoubleModifier {
                    modifier: modifier_to_ipc(m),
                    key: key.into(),
                },
                KeyStroke::DoubleTap(m) => ipc::KeyStroke::DoubleTap {
                    modifier: modifier_to_ipc(m),
                },
            },
        })
        .collect()
}
