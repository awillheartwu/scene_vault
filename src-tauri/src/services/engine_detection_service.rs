use std::path::{Path, PathBuf};

use crate::{error::AppError, models::engine::EngineDetection};

const ENGINE_GAME_DIR: &str = "game";

/// Detects a Ren'Py build from a user-selected game folder or executable.
/// Read-only: nothing is written and the game is never launched.
pub async fn detect(path: &Path) -> Result<EngineDetection, AppError> {
    let canonical = tokio::fs::canonicalize(path)
        .await
        .map_err(|error| AppError::Validation(format!("无法访问所选路径：{}", error)))?;
    let start = canonical
        .parent()
        .map(Path::to_path_buf)
        .filter(|_| canonical.is_file())
        .unwrap_or_else(|| canonical.clone());

    let engine_root = match resolve_engine_root(&start).await {
        Some(root) => root,
        None => {
            return Err(AppError::Validation(
                "所选目录看起来不是 Ren'Py 游戏：缺少 game/ 目录或 renpy/ 运行库".to_owned(),
            ))
        }
    };
    // The actual game directory is the ownership key: resolve symlinks and
    // junctions here so two differently spelled roots cannot claim the same
    // directory as two separate installs.
    let game_directory = tokio::fs::canonicalize(engine_root.join(ENGINE_GAME_DIR))
        .await
        .map_err(|error| AppError::Validation(format!("无法访问 game 目录：{error}")))?;
    let python_major = detect_python_major(&engine_root).await;
    let engine_version = detect_engine_version(&engine_root).await;
    let bridge_installed = tokio::fs::metadata(
        game_directory.join(crate::services::renpy_bridge_service::BRIDGE_FILE_NAME),
    )
    .await
    .is_ok_and(|metadata| metadata.is_file());

    Ok(EngineDetection {
        engine: crate::models::engine::ENGINE_RENPY.to_owned(),
        engine_root: crate::services::capture_service::path_to_string(&engine_root),
        game_directory: crate::services::capture_service::path_to_string(&game_directory),
        python_major,
        engine_version,
        bridge_installed,
    })
}

/// Reads the exact engine version from `renpy/vc_version.py`. Ren'Py 7 and 8
/// both ship that file; Python 2 builds spell the value as `u'…'`.
async fn detect_engine_version(root: &Path) -> Option<String> {
    let text = tokio::fs::read_to_string(root.join("renpy").join("vc_version.py"))
        .await
        .ok()?;
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("version") else {
            continue;
        };
        let rest = rest.trim_start();
        let Some(rest) = rest.strip_prefix('=') else {
            continue;
        };
        let rest = rest.trim_start();
        let rest = match rest.chars().next() {
            Some('u') | Some('U') => rest[1..].trim_start(),
            _ => rest,
        };
        let Some(quote) = rest.chars().next().filter(|c| *c == '\'' || *c == '"') else {
            continue;
        };
        let inner = &rest[1..];
        let Some(end) = inner.find(quote) else {
            continue;
        };
        let value = inner[..end].trim();
        if !value.is_empty() {
            return Some(value.to_owned());
        }
    }
    None
}

/// Accepts the game root, the `game/` folder itself, or a subfolder of a
/// game root. Returns the folder that owns `game/`.
async fn resolve_engine_root(start: &Path) -> Option<PathBuf> {
    if is_engine_root(start).await {
        return Some(start.to_path_buf());
    }
    if start
        .file_name()
        .is_some_and(|name| name == ENGINE_GAME_DIR)
    {
        if let Some(parent) = start.parent() {
            if is_engine_root(parent).await {
                return Some(parent.to_path_buf());
            }
        }
    }
    if let Some(parent) = start.parent() {
        if is_engine_root(parent).await {
            return Some(parent.to_path_buf());
        }
    }
    None
}

async fn is_engine_root(root: &Path) -> bool {
    let game_directory = root.join(ENGINE_GAME_DIR);
    if !tokio::fs::metadata(&game_directory)
        .await
        .is_ok_and(|metadata| metadata.is_dir())
    {
        return false;
    }
    if tokio::fs::metadata(root.join("renpy"))
        .await
        .is_ok_and(|metadata| metadata.is_dir())
    {
        return true;
    }
    // Some repacks drop renpy/ but keep compiled scripts or archives.
    let mut entries = match tokio::fs::read_dir(&game_directory).await {
        Ok(entries) => entries,
        Err(_) => return false,
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if name.ends_with(".rpyc") || name.ends_with(".rpa") || name.ends_with(".rpy") {
            return true;
        }
    }
    false
}

/// Reads the bundled Python major version from `lib/`. Windows builds are
/// preferred when a game ships several platform runtimes.
async fn detect_python_major(root: &Path) -> Option<i64> {
    let lib = root.join("lib");
    let mut entries = tokio::fs::read_dir(&lib).await.ok()?;
    let mut windows: Option<i64> = None;
    let mut any: Option<i64> = None;
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name().to_string_lossy().to_lowercase();
        let major = if name.starts_with("py3-") {
            3
        } else if name.starts_with("py2-") {
            2
        } else {
            continue;
        };
        if any.map(|current| major > current).unwrap_or(true) {
            any = Some(major);
        }
        if name.contains("windows") {
            windows = Some(major);
        }
    }
    windows.or(any)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    async fn write_minimal_game(root: &Path, python: &str) {
        tokio::fs::create_dir_all(root.join("game")).await.unwrap();
        tokio::fs::write(root.join("game/script.rpyc"), b"compiled")
            .await
            .unwrap();
        tokio::fs::create_dir_all(root.join(format!("lib/{python}")))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn detects_engine_root_and_python_major() {
        let workspace = tempdir().unwrap();
        let root = workspace.path().join("demo");
        write_minimal_game(&root, "py3-windows-x86_64").await;

        let detection = detect(&root).await.expect("detect");
        assert_eq!(detection.engine, "renpy");
        assert_eq!(detection.python_major, Some(3));
        assert!(!detection.bridge_installed);
    }

    #[tokio::test]
    async fn reads_the_exact_engine_version_from_vc_version_py() {
        let workspace = tempdir().unwrap();
        let root = workspace.path().join("demo");
        write_minimal_game(&root, "py3-windows-x86_64").await;
        tokio::fs::create_dir_all(root.join("renpy")).await.unwrap();
        tokio::fs::write(
            root.join("renpy/vc_version.py"),
            "branch = 'fix'\nversion = '8.1.3.23091805'\nversion_name = 'x'\n",
        )
        .await
        .unwrap();
        let detection = detect(&root).await.expect("detect");
        assert_eq!(detection.engine_version.as_deref(), Some("8.1.3.23091805"));

        // Python 2 games spell the same line with a u prefix.
        tokio::fs::write(
            root.join("renpy/vc_version.py"),
            "version = u'7.6.1.23060707'\n",
        )
        .await
        .unwrap();
        let detection = detect(&root).await.expect("detect py2");
        assert_eq!(detection.engine_version.as_deref(), Some("7.6.1.23060707"));

        // A repack without the file simply reports no version.
        tokio::fs::remove_file(root.join("renpy/vc_version.py"))
            .await
            .unwrap();
        let detection = detect(&root).await.expect("detect without version");
        assert_eq!(detection.engine_version, None);
    }

    #[tokio::test]
    async fn accepts_the_game_folder_itself() {
        let workspace = tempdir().unwrap();
        let root = workspace.path().join("demo");
        write_minimal_game(&root, "py2-windows-x86_64").await;
        tokio::fs::create_dir_all(root.join("renpy")).await.unwrap();

        let detection = detect(&root.join("game"))
            .await
            .expect("detect from game dir");
        assert_eq!(detection.python_major, Some(2));
        // `path_to_string` normalizes the `\\?\` prefix canonicalize adds.
        let canonical = tokio::fs::canonicalize(&root).await.unwrap();
        assert_eq!(
            detection.engine_root,
            crate::services::capture_service::path_to_string(&canonical)
        );
    }

    #[tokio::test]
    async fn reports_bridge_presence() {
        let workspace = tempdir().unwrap();
        let root = workspace.path().join("demo");
        write_minimal_game(&root, "py3-windows-x86_64").await;
        tokio::fs::write(
            root.join("game")
                .join(crate::services::renpy_bridge_service::BRIDGE_FILE_NAME),
            b"bridge",
        )
        .await
        .unwrap();

        let detection = detect(&root).await.expect("detect");
        assert!(detection.bridge_installed);
    }

    #[tokio::test]
    async fn rejects_directories_without_game_data() {
        let workspace = tempdir().unwrap();
        let root = workspace.path().join("plain");
        tokio::fs::create_dir_all(&root).await.unwrap();
        let error = detect(&root).await.expect_err("not a renpy game");
        assert!(matches!(error, AppError::Validation(_)));
    }
}
