//! Application core: owns runtime state and wires the domain crates together.
//!
//! In M0 it only provides application metadata and the frontend startup
//! check. Sessions, projects and documents arrive in M1 (see
//! `docs/ROADMAP.md`).

use ferrule_ipc::{
    AppInfo, FrontendReadyReport, IpcError, IpcErrorKind, PROTOCOL_VERSION, Platform,
};

/// Product name shown in window titles and the UI.
pub const PRODUCT_NAME: &str = "Ferrule";

/// Returns static information about the running application.
pub fn app_info() -> AppInfo {
    AppInfo {
        name: PRODUCT_NAME.to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        protocol_version: PROTOCOL_VERSION,
        platform: current_platform(),
    }
}

/// Checks the report the frontend sends after mounting.
///
/// Succeeds only if the frontend speaks the same protocol version and
/// rendered the product name both in the DOM and in the document title.
pub fn check_frontend_ready(report: &FrontendReadyReport) -> Result<(), IpcError> {
    if report.protocol_version != PROTOCOL_VERSION {
        return Err(IpcError::new(
            IpcErrorKind::Conflict,
            format!(
                "frontend protocol version {} does not match backend version {PROTOCOL_VERSION}",
                report.protocol_version
            ),
        ));
    }
    if report.rendered_app_name != PRODUCT_NAME {
        return Err(IpcError::new(
            IpcErrorKind::InvalidArgument,
            format!(
                "frontend rendered app name {:?}, expected {PRODUCT_NAME:?}",
                report.rendered_app_name
            ),
        ));
    }
    if !report.document_title.starts_with(PRODUCT_NAME) {
        return Err(IpcError::new(
            IpcErrorKind::InvalidArgument,
            format!(
                "document title {:?} does not start with {PRODUCT_NAME:?}",
                report.document_title
            ),
        ));
    }
    Ok(())
}

fn current_platform() -> Platform {
    match std::env::consts::OS {
        "linux" | "freebsd" | "openbsd" | "netbsd" | "dragonfly" => Platform::Linux,
        "macos" => Platform::Macos,
        "windows" => Platform::Windows,
        _ => Platform::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn good_report() -> FrontendReadyReport {
        FrontendReadyReport {
            rendered_app_name: PRODUCT_NAME.into(),
            document_title: PRODUCT_NAME.into(),
            protocol_version: PROTOCOL_VERSION,
        }
    }

    #[test]
    fn app_info_reports_product_and_crate_version() {
        let info = app_info();
        assert_eq!(info.name, "Ferrule");
        assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
        assert_eq!(info.protocol_version, PROTOCOL_VERSION);
        #[cfg(target_os = "linux")]
        assert_eq!(info.platform, Platform::Linux);
        #[cfg(target_os = "macos")]
        assert_eq!(info.platform, Platform::Macos);
        #[cfg(target_os = "windows")]
        assert_eq!(info.platform, Platform::Windows);
    }

    #[test]
    fn accepts_matching_report() {
        assert_eq!(check_frontend_ready(&good_report()), Ok(()));
    }

    #[test]
    fn rejects_protocol_mismatch() {
        let report = FrontendReadyReport {
            protocol_version: PROTOCOL_VERSION + 1,
            ..good_report()
        };
        let err = check_frontend_ready(&report).unwrap_err();
        assert_eq!(err.kind, IpcErrorKind::Conflict);
    }

    #[test]
    fn rejects_wrong_rendered_name() {
        let report = FrontendReadyReport {
            rendered_app_name: String::new(),
            ..good_report()
        };
        let err = check_frontend_ready(&report).unwrap_err();
        assert_eq!(err.kind, IpcErrorKind::InvalidArgument);
    }

    #[test]
    fn rejects_wrong_title() {
        let report = FrontendReadyReport {
            document_title: "Untitled".into(),
            ..good_report()
        };
        let err = check_frontend_ready(&report).unwrap_err();
        assert_eq!(err.kind, IpcErrorKind::InvalidArgument);
    }
}
