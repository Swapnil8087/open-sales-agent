//! Local file/directory layout and timestamped output naming.

use chrono::NaiveDateTime;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub const ROOT_DIR_NAME: &str = "Open-Sales-Agent";
pub const SUBDIRS: [&str; 4] = ["MoM", "Inquiries", "Proposals", "logs"];
const MAX_TOPIC_LEN: usize = 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    MoM,
    Inquiry,
    Proposal,
}

impl Category {
    /// Label used inside the file name.
    pub fn label(self) -> &'static str {
        match self {
            Category::MoM => "MoM",
            Category::Inquiry => "Inquiry",
            Category::Proposal => "Proposal",
        }
    }

    /// Sub-directory of the root the file is saved in.
    pub fn folder(self) -> &'static str {
        match self {
            Category::MoM => "MoM",
            Category::Inquiry => "Inquiries",
            Category::Proposal => "Proposals",
        }
    }
}

/// `~/Documents/Open-Sales-Agent`
pub fn default_root() -> Option<PathBuf> {
    dirs::document_dir().map(|d| d.join(ROOT_DIR_NAME))
}

/// Creates the root and all subdirectories. Idempotent.
pub fn ensure_dirs(root: &Path) -> io::Result<()> {
    for d in SUBDIRS {
        fs::create_dir_all(root.join(d))?;
    }
    Ok(())
}

/// Make a user/LLM-supplied topic safe for use in a file name: ASCII letters,
/// digits, `-` and `_` only; whitespace becomes `_`; capped at 60 chars.
pub fn sanitize_topic(topic: &str) -> String {
    let mut out = String::new();
    for c in topic.chars() {
        let c = if c.is_whitespace() { '_' } else { c };
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
            if c == '_' && out.ends_with('_') {
                continue;
            }
            out.push(c);
        }
    }
    let mut out: String = out.trim_matches('_').chars().take(MAX_TOPIC_LEN).collect();
    out = out.trim_end_matches('_').to_string();
    if out.is_empty() { "Untitled".into() } else { out }
}

/// `YYYY-MM-DD_HH-MM_Category_TopicName.md`
pub fn file_name(now: NaiveDateTime, category: Category, topic: &str) -> String {
    format!(
        "{}_{}_{}.md",
        now.format("%Y-%m-%d_%H-%M"),
        category.label(),
        sanitize_topic(topic)
    )
}

/// Saves a new document, never overwriting an existing one (adds `_2`, `_3`, ...).
pub fn save_new(
    root: &Path,
    now: NaiveDateTime,
    category: Category,
    topic: &str,
    content: &str,
) -> io::Result<PathBuf> {
    let dir = root.join(category.folder());
    fs::create_dir_all(&dir)?;
    let base = file_name(now, category, topic);
    let stem = base.trim_end_matches(".md");
    let mut n = 1u32;
    loop {
        let name = if n == 1 { base.clone() } else { format!("{stem}_{n}.md") };
        let path = dir.join(name);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut f) => {
                f.write_all(content.as_bytes())?;
                return Ok(path);
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => n += 1,
            Err(e) => return Err(e),
        }
    }
}

/// Atomically replaces the content of an existing document (draft revisions).
pub fn overwrite(path: &Path, content: &str) -> io::Result<()> {
    let dir = path.parent().ok_or_else(|| io::Error::other("no parent dir"))?;
    let tmp = dir.join(format!(".{}.tmp", path.file_name().and_then(|n| n.to_str()).unwrap_or("doc")));
    fs::write(&tmp, content)?;
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use regex::Regex;
    use std::fs;

    fn t(h: u32, m: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 11, 5).unwrap().and_hms_opt(h, m, 0).unwrap()
    }

    #[test]
    fn creates_root_and_all_subdirs() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("Documents").join(ROOT_DIR_NAME);
        ensure_dirs(&root).unwrap();
        for d in ["MoM", "Inquiries", "Proposals", "logs"] {
            assert!(root.join(d).is_dir(), "{d} missing");
        }
    }

    #[test]
    fn ensure_dirs_is_idempotent_and_keeps_files() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join(ROOT_DIR_NAME);
        ensure_dirs(&root).unwrap();
        let f = root.join("MoM").join("keep.md");
        fs::write(&f, "hi").unwrap();
        ensure_dirs(&root).unwrap();
        assert_eq!(fs::read_to_string(f).unwrap(), "hi");
    }

    #[test]
    fn default_root_is_under_documents() {
        let r = default_root().unwrap();
        assert!(r.ends_with("Documents/Open-Sales-Agent"));
    }

    #[test]
    fn file_name_matches_convention() {
        let n = file_name(t(14, 30), Category::MoM, "Lodha Group Lease");
        assert_eq!(n, "2026-11-05_14-30_MoM_Lodha_Group_Lease.md");
        let re = Regex::new(r"^\d{4}-\d{2}-\d{2}_\d{2}-\d{2}_(MoM|Inquiry|Proposal)_[A-Za-z0-9_\-]+\.md$").unwrap();
        for c in [Category::MoM, Category::Inquiry, Category::Proposal] {
            assert!(re.is_match(&file_name(t(9, 5), c, "Acme / Co: \"Q4\"?")));
        }
    }

    #[test]
    fn sanitize_cases() {
        assert_eq!(sanitize_topic("  Lodha   Group  "), "Lodha_Group");
        assert_eq!(sanitize_topic("a/b\\c:d*e?f\"g<h>i|j"), "abcdefghij");
        assert_eq!(sanitize_topic(""), "Untitled");
        assert_eq!(sanitize_topic("///"), "Untitled");
        assert_eq!(sanitize_topic("Café Münch"), "Caf_Mnch");
        assert_eq!(sanitize_topic("a\nb\tc"), "a_b_c");
        assert!(sanitize_topic(&"x".repeat(200)).len() <= 60);
        assert_eq!(sanitize_topic("a - b"), "a_-_b");
    }

    #[test]
    fn labels_and_folders() {
        assert_eq!(Category::MoM.label(), "MoM");
        assert_eq!(Category::Inquiry.label(), "Inquiry");
        assert_eq!(Category::Proposal.label(), "Proposal");
        assert_eq!(Category::MoM.folder(), "MoM");
        assert_eq!(Category::Inquiry.folder(), "Inquiries");
        assert_eq!(Category::Proposal.folder(), "Proposals");
    }

    #[test]
    fn save_new_routes_to_correct_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join(ROOT_DIR_NAME);
        ensure_dirs(&root).unwrap();
        let p = save_new(&root, t(14, 30), Category::Inquiry, "Raj Call", "body").unwrap();
        assert_eq!(p, root.join("Inquiries/2026-11-05_14-30_Inquiry_Raj_Call.md"));
        assert_eq!(fs::read_to_string(p).unwrap(), "body");
    }

    #[test]
    fn save_new_never_overwrites() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join(ROOT_DIR_NAME);
        ensure_dirs(&root).unwrap();
        let a = save_new(&root, t(1, 1), Category::MoM, "X", "one").unwrap();
        let b = save_new(&root, t(1, 1), Category::MoM, "X", "two").unwrap();
        let c = save_new(&root, t(1, 1), Category::MoM, "X", "three").unwrap();
        assert_ne!(a, b);
        assert!(b.to_string_lossy().ends_with("_X_2.md"));
        assert!(c.to_string_lossy().ends_with("_X_3.md"));
        assert_eq!(fs::read_to_string(a).unwrap(), "one");
    }

    #[test]
    fn save_new_creates_missing_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join(ROOT_DIR_NAME);
        let p = save_new(&root, t(2, 2), Category::Proposal, "Y", "z").unwrap();
        assert!(p.exists());
    }

    #[test]
    fn overwrite_replaces_content_without_leftovers() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join(ROOT_DIR_NAME);
        let p = save_new(&root, t(3, 3), Category::Proposal, "Draft", "v1").unwrap();
        overwrite(&p, "v2 longer").unwrap();
        assert_eq!(fs::read_to_string(&p).unwrap(), "v2 longer");
        let entries: Vec<_> = fs::read_dir(p.parent().unwrap()).unwrap().collect();
        assert_eq!(entries.len(), 1, "temp file left behind");
    }
}
