use std::path::{Path, PathBuf};
use std::process::Command;

use sqlx::SqlitePool;

use crate::{
    error::AppError,
    models::engine::{LaunchProjectGameInput, ProjectGameLaunchOption},
    services::{capture_service, log_service},
};

/// Executables that are never the game's main binary (installers, helpers).
const NON_LAUNCHABLE_MARKERS: [&str; 7] = [
    "unins",
    "uninstall",
    "setup",
    "install",
    "python",
    "renpy",
    "updater",
];

/// Every launchable executable of one project, best guess first.
pub async fn launch_options(
    pool: &SqlitePool,
    project_id: &str,
) -> Result<Vec<ProjectGameLaunchOption>, AppError> {
    if project_id.is_empty() {
        return Err(AppError::Validation(
            "project id cannot be empty".to_owned(),
        ));
    }
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT id, engine_root FROM project_engine_integrations WHERE project_id = ? ORDER BY installed_at ASC, id ASC",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await?;
    let mut options = Vec::new();
    for (integration_id, engine_root) in rows {
        let root = PathBuf::from(&engine_root);
        let candidates = game_executables(&root);
        if candidates.is_empty() {
            continue;
        }
        let game_name = root
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .unwrap_or("Ren'Py 游戏")
            .to_owned();
        // One row per candidate so an ambiguous folder (game exe plus mods
        // or tools) is the user's choice; the best guess is recommended.
        for (index, executable) in candidates.into_iter().enumerate() {
            options.push(ProjectGameLaunchOption {
                integration_id: integration_id.clone(),
                engine_root: engine_root.clone(),
                executable: capture_service::path_to_string(&executable),
                game_name: game_name.clone(),
                recommended: index == 0,
            });
        }
    }
    Ok(options)
}

/// Launches one game. Without an explicit executable or integration id the
/// launch only proceeds when exactly one candidate exists.
pub async fn launch(
    pool: &SqlitePool,
    input: LaunchProjectGameInput,
) -> Result<ProjectGameLaunchOption, AppError> {
    let project_id = input.project_id.trim();
    if project_id.is_empty() {
        return Err(AppError::Validation(
            "project id cannot be empty".to_owned(),
        ));
    }
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM projects WHERE id = ?)")
        .bind(project_id)
        .fetch_one(pool)
        .await?;
    if !exists {
        return Err(AppError::NotFound("project".to_owned()));
    }
    let requested_integration = input
        .integration_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let requested_executable = input
        .executable
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let options = launch_options(pool, project_id).await?;
    let selected = if let Some(executable) = requested_executable {
        // The override must be one of the scanned candidates, which keeps it
        // inside the game root by construction.
        options
            .into_iter()
            .find(|option| option.executable == executable)
            .ok_or_else(|| AppError::Validation("该可执行文件不在项目的游戏目录中".to_owned()))?
    } else if let Some(integration_id) = requested_integration {
        let mut matching: Vec<ProjectGameLaunchOption> = options
            .into_iter()
            .filter(|option| option.integration_id == integration_id)
            .collect();
        match matching.len() {
            0 => {
                return Err(AppError::Validation(
                    "该游戏没有可启动的可执行文件".to_owned(),
                ))
            }
            1 => matching
                .pop()
                .ok_or_else(|| AppError::Validation("没有可启动的游戏".to_owned()))?,
            _ => {
                return Err(AppError::Validation(
                    "该游戏有多个可执行文件：请在捕获页选择要启动的程序".to_owned(),
                ))
            }
        }
    } else {
        match options.len() {
            0 => {
                return Err(AppError::Validation(
                    "没有可启动的游戏：请先安装 Ren'Py 插件，或确认游戏目录中存在可执行文件"
                        .to_owned(),
                ))
            }
            1 => options
                .into_iter()
                .next()
                .ok_or_else(|| AppError::Validation("没有可启动的游戏".to_owned()))?,
            _ => {
                return Err(AppError::Validation(
                    "有多个可启动项：请在捕获页选择要启动的游戏与程序".to_owned(),
                ))
            }
        }
    };
    let executable = PathBuf::from(&selected.executable);
    let engine_root = PathBuf::from(&selected.engine_root);
    Command::new(&executable)
        .current_dir(&engine_root)
        .spawn()
        .map_err(|error| AppError::Validation(format!("启动游戏失败：{error}")))?;
    log_service::info(
        "engine.launch",
        format!("launched {} for project {project_id}", selected.executable),
    );
    Ok(selected)
}

/// Executables of a Ren'Py game root, most plausible first: an executable
/// named like the folder wins, then the largest file; installers and helper
/// binaries are excluded. Only the root is scanned, which is where Ren'Py
/// ships its launcher. Callers receive every candidate so an ambiguous
/// folder becomes the user's choice instead of a silent guess.
fn game_executables(engine_root: &Path) -> Vec<PathBuf> {
    let folder = engine_root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_lowercase();
    let Ok(entries) = std::fs::read_dir(engine_root) else {
        return Vec::new();
    };
    let mut candidates: Vec<(u8, u64, String, PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_file() {
            continue;
        }
        let path = entry.path();
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_lowercase();
        if extension != "exe" {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_lowercase();
        if NON_LAUNCHABLE_MARKERS
            .iter()
            .any(|marker| name.contains(marker))
        {
            continue;
        }
        let stem = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_lowercase();
        // Short stems are substrings of almost any folder name ("a" inside
        // "repack"), so the loose containment rule only applies to stems long
        // enough to be a name; the rest falls back to file size.
        let meaningful = stem.len() >= 4;
        let rank = if !folder.is_empty() && stem == folder {
            0
        } else if meaningful
            && !folder.is_empty()
            && (stem.contains(&folder) || folder.contains(&stem))
        {
            1
        } else {
            2
        };
        let size = entry.metadata().map(|metadata| metadata.len()).unwrap_or(0);
        candidates.push((rank, size, name, path));
    }
    candidates.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then(right.1.cmp(&left.1))
            .then(left.2.cmp(&right.2))
    });
    candidates.into_iter().map(|(_, _, _, path)| path).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_exe(dir: &Path, name: &str, bytes: usize) {
        std::fs::write(dir.join(name), vec![0_u8; bytes]).expect("write exe");
    }

    #[test]
    fn prefers_the_folder_named_executable_and_skips_helpers() {
        let workspace = tempfile::tempdir().expect("tempdir");
        let root = workspace.path().join("MyGame");
        std::fs::create_dir_all(&root).unwrap();
        write_exe(&root, "helper.exe", 500);
        write_exe(&root, "MyGame.exe", 100);
        write_exe(&root, "unins000.exe", 900);
        write_exe(&root, "python.exe", 900);
        let found = game_executables(&root);
        assert_eq!(
            found[0].file_name().and_then(|n| n.to_str()),
            Some("MyGame.exe")
        );
        assert_eq!(found.len(), 2, "helpers stay selectable as alternatives");
    }

    #[test]
    fn recognizes_a_suffixed_folder_name() {
        let workspace = tempfile::tempdir().expect("tempdir");
        let root = workspace.path().join("TheBeachHouse-0.2-pc");
        std::fs::create_dir_all(&root).unwrap();
        write_exe(&root, "helper.exe", 900);
        write_exe(&root, "TheBeachHouse.exe", 100);
        let found = game_executables(&root);
        assert_eq!(
            found[0].file_name().and_then(|n| n.to_str()),
            Some("TheBeachHouse.exe")
        );
    }

    #[test]
    fn keeps_a_larger_helper_visible_instead_of_hiding_it() {
        let workspace = tempfile::tempdir().expect("tempdir");
        // Folder name matches the game exe: the match wins the recommendation,
        // the bigger helper stays available as an alternative.
        let root = workspace.path().join("story");
        std::fs::create_dir_all(&root).unwrap();
        write_exe(&root, "Story.exe", 111 * 1024);
        write_exe(&root, "URM.exe", 2 * 1024 * 1024);
        let found = game_executables(&root);
        assert_eq!(found.len(), 2);
        assert_eq!(
            found[0].file_name().and_then(|n| n.to_str()),
            Some("Story.exe")
        );
        assert_eq!(
            found[1].file_name().and_then(|n| n.to_str()),
            Some("URM.exe")
        );

        // No name match: the size fallback may guess wrong, so both
        // candidates must remain selectable for the picker.
        let ambiguous = workspace.path().join("repack");
        std::fs::create_dir_all(&ambiguous).unwrap();
        write_exe(&ambiguous, "Story.exe", 111 * 1024);
        write_exe(&ambiguous, "URM.exe", 2 * 1024 * 1024);
        let found = game_executables(&ambiguous);
        assert_eq!(found.len(), 2);
        assert_eq!(
            found[0].file_name().and_then(|n| n.to_str()),
            Some("URM.exe"),
            "without a name match the guess falls back to size; the user picks"
        );
    }

    #[test]
    fn returns_none_without_executables() {
        let workspace = tempfile::tempdir().expect("tempdir");
        let root = workspace.path().join("empty");
        std::fs::create_dir_all(&root).unwrap();
        assert!(game_executables(&root).is_empty());
    }

    #[tokio::test]
    async fn lists_launchable_games_for_a_project() {
        let pool = crate::db::test_pool().await;
        let workspace = tempfile::tempdir().expect("tempdir");
        let root = workspace.path().join("MyGame");
        std::fs::create_dir_all(&root).unwrap();
        write_exe(&root, "MyGame.exe", 100);
        let project_id = uuid::Uuid::new_v4().to_string();
        sqlx::query("INSERT INTO projects (id, name) VALUES (?, 'Launch')")
            .bind(&project_id)
            .execute(&pool)
            .await
            .expect("project");
        let now = "2026-10-11T00:00:00Z";
        sqlx::query(
            "INSERT INTO project_engine_integrations (id, project_id, engine, engine_root, game_directory, python_major, bridge_version, metadata_directory, manifest_json, installed_at, updated_at) VALUES (?, ?, 'renpy', ?, ?, 3, '1.1.3', NULL, '[]', ?, ?)",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(&project_id)
        .bind(crate::services::capture_service::path_to_string(&root))
        .bind(crate::services::capture_service::path_to_string(&root.join("game")))
        .bind(now)
        .bind(now)
        .execute(&pool)
        .await
        .expect("integration");
        let options = launch_options(&pool, &project_id).await.expect("options");
        assert_eq!(options.len(), 1);
        assert!(options[0].executable.ends_with("MyGame.exe"));
        assert_eq!(options[0].game_name, "MyGame");
        assert!(options[0].recommended);
    }
}
