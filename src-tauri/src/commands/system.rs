//! OS-level helpers that do not belong to a specific domain.
//!
//! The opener plugin's `open_path` command is scope-gated: any path outside
//! the capability scope is rejected with "Not allowed to open path", and the
//! scope can only be configured statically (no runtime registration API). That
//! cannot cover user-configured directories (custom models/python dirs), so
//! directories are opened through the OS directly instead. Files keep using
//! the plugin's reveal (select-in-explorer), which is not scope-gated.

use std::path::PathBuf;

/// Opens a directory in the system file explorer.
///
/// Only directories are accepted; for files use `reveal_item_in_dir` (the
/// plugin's reveal command) instead.
#[tauri::command]
pub fn open_directory(path: String) -> Result<(), String> {
    let dir = ensure_directory(&path)?;

    #[cfg(target_os = "windows")]
    {
        // explorer.exe detaches immediately and reports exit code 1 even on
        // success, so only the spawn itself is checked.
        std::process::Command::new("explorer")
            .arg(&dir)
            .spawn()
            .map_err(|err| format!("无法打开目录 {path}: {err}"))?;
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::process::Command::new("xdg-open")
            .arg(&dir)
            .spawn()
            .map_err(|err| format!("无法打开目录 {path}: {err}"))?;
    }
    Ok(())
}

/// App-managed directories (thumbnail caches, logs, capture output) are
/// created lazily and may not exist yet when the user clicks "open"; create
/// the missing directory so the explorer can show it. Real files and paths
/// blocked by the filesystem still fail with a clear error.
fn ensure_directory(path: &str) -> Result<PathBuf, String> {
    let dir = PathBuf::from(path);
    if dir.is_dir() {
        return Ok(dir);
    }
    if dir.exists() {
        return Err(format!("不是目录: {path}"));
    }
    std::fs::create_dir_all(&dir).map_err(|err| format!("无法创建目录 {path}: {err}"))?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn creates_missing_directories() {
        let workspace = tempfile::tempdir().expect("tempdir");
        let missing = workspace.path().join("nested").join("cache");
        let created = ensure_directory(missing.to_str().unwrap()).expect("created");
        assert_eq!(created, missing);
        assert!(missing.is_dir());
    }

    #[test]
    fn rejects_plain_files() {
        let workspace = tempfile::tempdir().expect("tempdir");
        let file = workspace.path().join("x.txt");
        fs::write(&file, b"x").expect("write");
        assert!(ensure_directory(file.to_str().unwrap()).is_err());
    }

    #[test]
    fn accepts_existing_directories() {
        let workspace = tempfile::tempdir().expect("tempdir");
        let dir = workspace.path().join("models");
        fs::create_dir_all(&dir).expect("create dir");
        assert_eq!(
            ensure_directory(dir.to_str().unwrap()).expect("valid dir"),
            dir
        );
    }
}
