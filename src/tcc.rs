//! Detection of a probable macOS TCC (Full Disk Access) denial from a finished scan.
//!
//! Without Full Disk Access macOS makes `read_dir` on a few home folders fail (EPERM, which
//! std reports as `PermissionDenied`) or, for some iCloud-backed folders, return nothing. The
//! user would conclude "my Documents folder is empty". It is a permission, so we say so.
//!
//! The logic is pure and cfg-independent (it only looks at [`ScanResult`] records and a path
//! list), so it is unit-tested on every OS. Only [`PROTECTED_HOME_RELATIVE`] is macOS-specific
//! and [`detect_for_current_os`] returns an empty report elsewhere.

use crate::model::{DirState, ErrorClass, ScanResult};
use std::path::{Path, PathBuf};

/// Home-relative folders that macOS guards with TCC. Source: Apple's documentation on
/// protecting user data with TCC / Full Disk Access, and the behaviour documented by other
/// disk analyzers. Confidence: Desktop, Documents, Downloads and Library/Mobile Documents
/// (iCloud Drive) are high; Library/Safari, Mail, Messages and AddressBook are FDA-only data
/// (high). This list was written on Windows from documentation and has never been run
/// against a real macOS TCC setup: UNVERIFIED.
pub const PROTECTED_HOME_RELATIVE: &[&str] = &[
    "Desktop",
    "Documents",
    "Downloads",
    "Library/Mobile Documents",
    "Library/Safari",
    "Library/Mail",
    "Library/Messages",
    "Library/Application Support/AddressBook",
];

/// System Settings pane that grants Full Disk Access (the deep link and the click path).
pub const FULL_DISK_ACCESS_URL: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles";
pub const FULL_DISK_ACCESS_PANE: &str = "System Settings > Privacy & Security > Full Disk Access";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProtectedStatus {
    /// The OS refused to list the folder (EPERM/EACCES): a permission, contents UNKNOWN.
    Denied,
    /// Listed fine but contained nothing while the rest of the home had files: suspicious,
    /// possibly a silent TCC/iCloud denial, possibly really empty.
    EmptySuspicious,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    pub path: PathBuf,
    pub status: ProtectedStatus,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TccReport {
    pub findings: Vec<Finding>,
}

impl TccReport {
    /// True when the totals probably exclude protected folders.
    pub fn probable(&self) -> bool {
        !self.findings.is_empty()
    }

    pub fn denied(&self) -> impl Iterator<Item = &Finding> {
        self.of(ProtectedStatus::Denied)
    }

    pub fn empty_suspicious(&self) -> impl Iterator<Item = &Finding> {
        self.of(ProtectedStatus::EmptySuspicious)
    }

    fn of(&self, status: ProtectedStatus) -> impl Iterator<Item = &Finding> {
        self.findings
            .iter()
            .filter(move |finding| finding.status == status)
    }

    /// Short note for the totals area, next to the unaccounted figure.
    pub fn partial_note(&self) -> Option<String> {
        if !self.probable() {
            return None;
        }
        Some(format!(
            "partial: permission - {} protected folder{} blocked or suspiciously empty, \
             totals exclude them",
            self.findings.len(),
            if self.findings.len() == 1 { "" } else { "s" }
        ))
    }

    /// Banner body; the two states are worded differently on purpose.
    pub fn banner_text(&self) -> Option<String> {
        if !self.probable() {
            return None;
        }
        let names = |iter: &mut dyn Iterator<Item = &Finding>| {
            iter.map(|finding| finding.path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        };
        let mut text = String::new();
        let denied = names(&mut self.denied());
        if !denied.is_empty() {
            text.push_str(&format!(
                "macOS refused permission to read: {denied}. These folders are NOT empty, \
                 their contents are unknown and not counted. "
            ));
        }
        let empty = names(&mut self.empty_suspicious());
        if !empty.is_empty() {
            text.push_str(&format!(
                "Read as empty, which may be a silent permission block (or truly empty): \
                 {empty}. "
            ));
        }
        text.push_str(&format!(
            "Grant access in {FULL_DISK_ACCESS_PANE}, then rescan."
        ));
        Some(text)
    }
}

/// Pure detection. `protected` are home-relative folders; `home` is the user's home directory.
/// Only folders that appear in the scan's directory records are judged (a scan that never
/// reached them says nothing about them).
pub fn detect(home: &Path, protected: &[&str], result: &ScanResult) -> TccReport {
    let protected_paths: Vec<PathBuf> = protected.iter().map(|rel| home.join(rel)).collect();
    let in_protected = |path: &Path| protected_paths.iter().any(|p| path.starts_with(p));
    let home_has_other_data = result
        .files
        .iter()
        .any(|file| file.path.starts_with(home) && !in_protected(&file.path));

    let mut findings = Vec::new();
    for path in &protected_paths {
        let Some(record) = result.dirs.iter().find(|dir| dir.path == *path) else {
            continue;
        };
        match record.state {
            DirState::Unreadable(ErrorClass::PermissionDenied) => findings.push(Finding {
                path: path.clone(),
                status: ProtectedStatus::Denied,
            }),
            DirState::Unreadable(_) => {}
            DirState::Read => {
                let has_files = result.files.iter().any(|file| file.path.starts_with(path));
                let has_subdirs = result
                    .dirs
                    .iter()
                    .any(|dir| dir.path != *path && dir.path.starts_with(path));
                if !has_files && !has_subdirs && home_has_other_data {
                    findings.push(Finding {
                        path: path.clone(),
                        status: ProtectedStatus::EmptySuspicious,
                    });
                }
            }
        }
    }
    TccReport { findings }
}

/// macOS only (runtime check so the code is type-checked everywhere); empty report elsewhere.
pub fn detect_for_current_os(result: &ScanResult) -> TccReport {
    if !cfg!(target_os = "macos") {
        return TccReport::default();
    }
    match std::env::var_os("HOME") {
        Some(home) => detect(Path::new(&home), PROTECTED_HOME_RELATIVE, result),
        None => TccReport::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DirRecord, FileEntry, LogicalSize, PhysicalSize};

    fn home() -> PathBuf {
        PathBuf::from("/Users/u")
    }

    fn dir(path: &str, state: DirState) -> DirRecord {
        DirRecord {
            path: home().join(path),
            state,
        }
    }

    fn file(path: &str) -> FileEntry {
        FileEntry {
            path: home().join(path),
            name: "f".into(),
            extension: String::new(),
            logical: LogicalSize(1),
            physical: Some(PhysicalSize(1)),
            modified_unix: 0,
            links: 1,
            id: None,
            extra_link: false,
        }
    }

    fn scan(dirs: Vec<DirRecord>, files: Vec<FileEntry>) -> ScanResult {
        ScanResult {
            dirs,
            files,
            ..Default::default()
        }
    }

    const LIST: &[&str] = &["Documents", "Desktop"];

    #[test]
    fn permission_denied_protected_dir_is_flagged_denied() {
        let result = scan(
            vec![
                dir("", DirState::Read),
                dir(
                    "Documents",
                    DirState::Unreadable(ErrorClass::PermissionDenied),
                ),
                dir("Work", DirState::Read),
            ],
            vec![file("Work/a.txt")],
        );
        let report = detect(&home(), LIST, &result);
        assert!(report.probable());
        assert_eq!(report.denied().count(), 1);
        assert_eq!(report.empty_suspicious().count(), 0);
        assert!(report.banner_text().unwrap().contains("NOT empty"));
        assert!(report.banner_text().unwrap().contains("Full Disk Access"));
        assert!(
            report
                .partial_note()
                .unwrap()
                .starts_with("partial: permission")
        );
    }

    #[test]
    fn empty_protected_dir_with_busy_siblings_is_suspicious_not_denied() {
        let result = scan(
            vec![dir("Desktop", DirState::Read), dir("Work", DirState::Read)],
            vec![file("Work/a.txt")],
        );
        let report = detect(&home(), LIST, &result);
        assert_eq!(report.denied().count(), 0);
        assert_eq!(report.empty_suspicious().count(), 1);
        assert!(report.banner_text().unwrap().contains("truly empty"));
    }

    #[test]
    fn empty_protected_dir_alone_is_not_flagged() {
        let result = scan(vec![dir("Desktop", DirState::Read)], vec![]);
        assert!(!detect(&home(), LIST, &result).probable());
    }

    #[test]
    fn protected_dir_with_content_is_clean() {
        let result = scan(
            vec![
                dir("Documents", DirState::Read),
                dir("Documents/sub", DirState::Read),
            ],
            vec![file("Documents/sub/x"), file("Other/y")],
        );
        assert!(!detect(&home(), LIST, &result).probable());
        // Subdirectory present but no files: still content, not "empty".
        let result = scan(
            vec![
                dir("Documents", DirState::Read),
                dir("Documents/sub", DirState::Read),
            ],
            vec![file("Other/y")],
        );
        assert!(!detect(&home(), LIST, &result).probable());
    }

    #[test]
    fn other_errors_and_unvisited_dirs_are_not_tcc() {
        let result = scan(
            vec![dir("Documents", DirState::Unreadable(ErrorClass::NotFound))],
            vec![file("Other/y")],
        );
        assert!(!detect(&home(), LIST, &result).probable());
        // Scan never reached Desktop/Documents (e.g. root was elsewhere): no claim.
        assert!(!detect(&home(), LIST, &scan(vec![], vec![file("Other/y")])).probable());
    }

    #[test]
    fn non_macos_gets_an_empty_report_and_list_is_sane() {
        if !cfg!(target_os = "macos") {
            let result = scan(
                vec![dir(
                    "Documents",
                    DirState::Unreadable(ErrorClass::PermissionDenied),
                )],
                vec![],
            );
            assert!(!detect_for_current_os(&result).probable());
        }
        assert!(PROTECTED_HOME_RELATIVE.contains(&"Documents"));
        assert!(FULL_DISK_ACCESS_URL.starts_with("x-apple.systempreferences:"));
    }
}
