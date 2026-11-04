pub mod storage;

use std::path::PathBuf;

/// Provision the data directory on boot. Returns the root that was prepared.
pub fn init_storage(root: Option<PathBuf>) -> std::io::Result<PathBuf> {
    let root = root
        .or_else(storage::default_root)
        .ok_or_else(|| std::io::Error::other("cannot resolve Documents directory"))?;
    storage::ensure_dirs(&root)?;
    Ok(root)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|_app| {
            init_storage(None)?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boot_provisions_directory_tree() {
        let tmp = tempfile::tempdir().unwrap();
        let root = init_storage(Some(tmp.path().join("Open-Sales-Agent"))).unwrap();
        for d in ["MoM", "Inquiries", "Proposals", "logs"] {
            assert!(root.join(d).is_dir());
        }
    }
}
