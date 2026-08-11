use std::path::PathBuf;

use sqlx::SqlitePool;
use uuid::Uuid;

use crate::services::capture_service;
use crate::{
    error::AppError,
    models::project::{
        AddProjectSourceDirectoryInput, CreateProjectInput, Project, ProjectDeletionPreview,
        ProjectOverviewSummary, ProjectSourceDirectory, RemoveProjectSourceDirectoryInput,
        RenameProjectInput, SetProjectCoverInput, SetProjectDestinationInput,
        SetProjectSourceDirectoryEnabledInput,
    },
};

pub async fn create(pool: &SqlitePool, input: CreateProjectInput) -> Result<Project, AppError> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err(AppError::Validation(
            "project name cannot be empty".to_owned(),
        ));
    }

    let description = input
        .description
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());

    let project = sqlx::query_as::<_, Project>(
        r#"
        INSERT INTO projects (id, name, description, cover_asset_id, cover_capture_item_id)
        VALUES (?, ?, ?, ?, NULL)
        RETURNING
            id, name, description, cover_asset_id, cover_capture_item_id,
            last_source_directory, last_destination_directory,
            destination_directory, created_at, updated_at
        "#,
    )
    .bind(Uuid::new_v4().to_string())
    .bind(name)
    .bind(description)
    .bind(input.cover_asset_id)
    .fetch_one(pool)
    .await?;

    Ok(project)
}

pub async fn rename(pool: &SqlitePool, input: RenameProjectInput) -> Result<Project, AppError> {
    let project_id = input.project_id.trim();
    let name = input.name.trim();
    if project_id.is_empty() {
        return Err(AppError::Validation(
            "project id cannot be empty".to_owned(),
        ));
    }
    if name.is_empty() {
        return Err(AppError::Validation(
            "project name cannot be empty".to_owned(),
        ));
    }
    let project = sqlx::query_as::<_, Project>(
        r#"
        UPDATE projects
        SET
            name = ?,
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        WHERE id = ?
        RETURNING
            id, name, description, cover_asset_id, cover_capture_item_id,
            last_source_directory, last_destination_directory,
            destination_directory, created_at, updated_at
        "#,
    )
    .bind(name)
    .bind(project_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound("project".to_owned()))?;
    Ok(project)
}

pub async fn preview_deletion(
    pool: &SqlitePool,
    project_id: &str,
) -> Result<ProjectDeletionPreview, AppError> {
    let project_id = project_id.trim();
    if project_id.is_empty() {
        return Err(AppError::Validation(
            "project id cannot be empty".to_owned(),
        ));
    }
    let exists: i64 = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM projects WHERE id = ?)")
        .bind(project_id)
        .fetch_one(pool)
        .await?;
    if exists == 0 {
        return Err(AppError::NotFound("project".to_owned()));
    }
    let capture_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM capture_items WHERE project_id = ?")
            .bind(project_id)
            .fetch_one(pool)
            .await?;
    let character_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM characters WHERE project_id = ?")
            .bind(project_id)
            .fetch_one(pool)
            .await?;
    let session_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM capture_sessions WHERE project_id = ?")
            .bind(project_id)
            .fetch_one(pool)
            .await?;
    let has_active_session: i64 = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM capture_sessions WHERE project_id = ? AND status = 'active')",
    )
    .bind(project_id)
    .fetch_one(pool)
    .await?;
    let note_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM project_notes WHERE project_id = ?")
            .bind(project_id)
            .fetch_one(pool)
            .await?;
    let collection_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM collections WHERE project_id = ?")
            .bind(project_id)
            .fetch_one(pool)
            .await?;
    let orphan_asset_count: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM project_assets link
        WHERE link.project_id = ?
          AND NOT EXISTS (
              SELECT 1 FROM project_assets other
              WHERE other.asset_id = link.asset_id AND other.project_id != ?
          )
        "#,
    )
    .bind(project_id)
    .bind(project_id)
    .fetch_one(pool)
    .await?;
    Ok(ProjectDeletionPreview {
        capture_count,
        character_count,
        session_count,
        has_active_session: has_active_session == 1,
        note_count,
        collection_count,
        orphan_asset_count,
    })
}

/// Deletes the project and everything it owns. On-disk source screenshots,
/// archived outputs and shared assets are never touched; asset records that
/// no other project references are removed from the library index.
pub async fn delete_project(
    pool: &SqlitePool,
    project_id: &str,
) -> Result<ProjectDeletionPreview, AppError> {
    let preview = preview_deletion(pool, project_id).await?;
    let mut transaction = pool.begin().await?;
    // Orphan assets: drop rows that only this project linked, keeping shared
    // library records and their files intact.
    sqlx::query(
        r#"
        DELETE FROM assets
        WHERE id IN (
            SELECT link.asset_id
            FROM project_assets link
            WHERE link.project_id = ?
              AND NOT EXISTS (
                  SELECT 1 FROM project_assets other
                  WHERE other.asset_id = link.asset_id AND other.project_id != ?
              )
        )
        "#,
    )
    .bind(project_id)
    .bind(project_id)
    .execute(&mut *transaction)
    .await?;
    // The project delete cascades sessions, items, faces, samples, characters,
    // notes, source directories, collections and remaining asset links.
    sqlx::query("DELETE FROM projects WHERE id = ?")
        .bind(project_id)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(preview)
}

pub async fn list(pool: &SqlitePool) -> Result<Vec<Project>, AppError> {
    let projects = sqlx::query_as::<_, Project>(
        r#"
        SELECT
            id, name, description, cover_asset_id, cover_capture_item_id,
            last_source_directory, last_destination_directory,
            destination_directory, created_at, updated_at
        FROM projects
        ORDER BY updated_at DESC, name COLLATE NOCASE ASC
        "#,
    )
    .fetch_all(pool)
    .await?;

    Ok(projects)
}

pub async fn list_overviews(pool: &SqlitePool) -> Result<Vec<ProjectOverviewSummary>, AppError> {
    let summaries = sqlx::query_as::<_, ProjectOverviewSummary>(
        r#"
        WITH capture_stats AS (
            SELECT
                project_id,
                COUNT(*) AS capture_count,
                SUM(CASE WHEN status = 'awaiting_label' THEN 1 ELSE 0 END) AS awaiting_count,
                SUM(CASE WHEN status IN ('queued', 'processing', 'archive_pending') THEN 1 ELSE 0 END) AS processing_count,
                SUM(CASE WHEN status = 'completed' THEN 1 ELSE 0 END) AS completed_count,
                SUM(CASE WHEN status = 'failed' THEN 1 ELSE 0 END) AS failed_count,
                MAX(captured_at) AS last_capture_at
            FROM capture_items
            WHERE classification <> 'private'
            GROUP BY project_id
        ),
        session_stats AS (
            SELECT
                project_id,
                COUNT(*) AS session_count,
                SUM(CASE WHEN status = 'active' THEN 1 ELSE 0 END) AS active_session_count
            FROM capture_sessions
            GROUP BY project_id
        ),
        source_stats AS (
            SELECT project_id, COUNT(*) AS source_count
            FROM project_source_directories
            WHERE enabled = 1
            GROUP BY project_id
        )
        SELECT
            p.id AS project_id,
            p.name,
            p.description,
            CASE
                WHEN EXISTS (
                    SELECT 1
                    FROM capture_items cover
                    WHERE cover.id = p.cover_capture_item_id
                      AND cover.project_id = p.id
                      AND cover.classification <> 'private'
                ) THEN p.cover_capture_item_id
                ELSE NULL
            END AS cover_capture_item_id,
            p.created_at,
            COALESCE(src.source_count, 0) AS source_count,
            (p.destination_directory IS NOT NULL AND p.destination_directory <> '') AS destination_configured,
            COALESCE(ss.session_count, 0) AS session_count,
            COALESCE(ss.active_session_count, 0) AS active_session_count,
            COALESCE(cs.capture_count, 0) AS capture_count,
            COALESCE(cs.awaiting_count, 0) AS awaiting_count,
            COALESCE(cs.processing_count, 0) AS processing_count,
            COALESCE(cs.completed_count, 0) AS completed_count,
            COALESCE(cs.failed_count, 0) AS failed_count,
            COALESCE(cs.last_capture_at, p.updated_at) AS last_activity_at,
            (
                SELECT item.id
                FROM capture_items item
                WHERE item.project_id = p.id
                  AND item.classification <> 'private'
                ORDER BY item.captured_at DESC, item.id DESC
                LIMIT 1
            ) AS latest_capture_item_id
        FROM projects p
        LEFT JOIN capture_stats cs ON cs.project_id = p.id
        LEFT JOIN session_stats ss ON ss.project_id = p.id
        LEFT JOIN source_stats src ON src.project_id = p.id
        ORDER BY last_activity_at DESC, p.name COLLATE NOCASE ASC
        "#,
    )
    .fetch_all(pool)
    .await?;

    Ok(summaries)
}

pub async fn list_source_directories(
    pool: &SqlitePool,
    project_id: &str,
) -> Result<Vec<ProjectSourceDirectory>, AppError> {
    let project_id = project_id.trim();
    if project_id.is_empty() {
        return Err(AppError::Validation(
            "project id cannot be empty".to_owned(),
        ));
    }
    let directories = sqlx::query_as::<_, ProjectSourceDirectory>(
        r#"
        SELECT id, project_id, directory, enabled, created_at
        FROM project_source_directories
        WHERE project_id = ?
        ORDER BY created_at ASC
        "#,
    )
    .bind(project_id)
    .fetch_all(pool)
    .await?;
    Ok(directories)
}

pub async fn add_source_directory(
    pool: &SqlitePool,
    input: AddProjectSourceDirectoryInput,
) -> Result<ProjectSourceDirectory, AppError> {
    let project_id = input.project_id.trim();
    if project_id.is_empty() {
        return Err(AppError::Validation(
            "project id cannot be empty".to_owned(),
        ));
    }
    let project_exists: i64 =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM projects WHERE id = ?)")
            .bind(project_id)
            .fetch_one(pool)
            .await?;
    if project_exists == 0 {
        return Err(AppError::NotFound("project".to_owned()));
    }

    let directory =
        canonical_existing_directory(std::path::Path::new(input.directory.trim())).await?;
    let directory = capture_service::path_to_string(&directory);

    let duplicate: i64 = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM project_source_directories WHERE project_id = ? AND directory = ?)",
    )
    .bind(project_id)
    .bind(&directory)
    .fetch_one(pool)
    .await?;
    if duplicate != 0 {
        return Err(AppError::Conflict(
            "this source directory is already in the project".to_owned(),
        ));
    }

    // The archive destination must never live inside a source directory.
    let destination: Option<String> =
        sqlx::query_scalar("SELECT destination_directory FROM projects WHERE id = ?")
            .bind(project_id)
            .fetch_one(pool)
            .await?;
    if let Some(destination) = destination {
        let destination = std::path::PathBuf::from(&destination);
        let source = std::path::PathBuf::from(&directory);
        if capture_service::path_is_within(&source, &destination)
            || capture_service::path_is_within(&destination, &source)
        {
            return Err(AppError::Validation(
                "archive destination and capture source directories must not contain each other"
                    .to_owned(),
            ));
        }
    }

    let created = sqlx::query_as::<_, ProjectSourceDirectory>(
        r#"
        INSERT INTO project_source_directories (id, project_id, directory)
        VALUES (?, ?, ?)
        RETURNING id, project_id, directory, enabled, created_at
        "#,
    )
    .bind(Uuid::new_v4().to_string())
    .bind(project_id)
    .bind(&directory)
    .fetch_one(pool)
    .await?;
    Ok(created)
}

pub async fn remove_source_directory(
    pool: &SqlitePool,
    input: RemoveProjectSourceDirectoryInput,
) -> Result<(), AppError> {
    let id = input.id.trim();
    if id.is_empty() {
        return Err(AppError::Validation(
            "source directory id cannot be empty".to_owned(),
        ));
    }
    let deleted = sqlx::query("DELETE FROM project_source_directories WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    if deleted.rows_affected() == 0 {
        return Err(AppError::NotFound("source directory".to_owned()));
    }
    Ok(())
}

pub async fn set_source_directory_enabled(
    pool: &SqlitePool,
    input: SetProjectSourceDirectoryEnabledInput,
) -> Result<ProjectSourceDirectory, AppError> {
    let updated = sqlx::query_as::<_, ProjectSourceDirectory>(
        r#"
        UPDATE project_source_directories
        SET enabled = ?
        WHERE id = ?
        RETURNING id, project_id, directory, enabled, created_at
        "#,
    )
    .bind(if input.enabled { 1 } else { 0 })
    .bind(input.id.trim())
    .fetch_optional(pool)
    .await?;
    updated.ok_or_else(|| AppError::NotFound("source directory".to_owned()))
}

pub async fn set_destination_directory(
    pool: &SqlitePool,
    input: SetProjectDestinationInput,
) -> Result<Project, AppError> {
    let project_id = input.project_id.trim();
    if project_id.is_empty() {
        return Err(AppError::Validation(
            "project id cannot be empty".to_owned(),
        ));
    }
    let directory = PathBuf::from(input.directory.trim());
    if !directory.is_absolute() {
        return Err(AppError::Validation(
            "capture destination directory must be an absolute path".to_owned(),
        ));
    }

    // The archive destination must never live inside a source directory.
    let sources: Vec<String> =
        sqlx::query_scalar("SELECT directory FROM project_source_directories WHERE project_id = ?")
            .bind(project_id)
            .fetch_all(pool)
            .await?;
    for source in sources {
        if capture_service::path_is_within(&directory, &PathBuf::from(&source))
            || capture_service::path_is_within(&PathBuf::from(&source), &directory)
        {
            return Err(AppError::Validation(
                "archive destination and capture source directories must not contain each other"
                    .to_owned(),
            ));
        }
    }

    let project = sqlx::query_as::<_, Project>(
        r#"
        UPDATE projects
        SET destination_directory = ?,
            last_destination_directory = ?,
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        WHERE id = ?
        RETURNING
            id, name, description, cover_asset_id, cover_capture_item_id,
            last_source_directory, last_destination_directory,
            destination_directory, created_at, updated_at
        "#,
    )
    .bind(directory.to_string_lossy().into_owned())
    .bind(directory.to_string_lossy().into_owned())
    .bind(project_id)
    .fetch_optional(pool)
    .await?;
    project.ok_or_else(|| AppError::NotFound("project".to_owned()))
}

/// Pins (or clears) the project cover capture. Person, scene and unclassified
/// captures are valid; private captures are rejected because Home hides that
/// category by default. The capture must belong to the project; NULL/empty
/// clears the custom cover so Home falls back to the latest capture again.
pub async fn set_cover(
    pool: &SqlitePool,
    input: SetProjectCoverInput,
) -> Result<Project, AppError> {
    let project_id = input.project_id.trim();
    if project_id.is_empty() {
        return Err(AppError::Validation(
            "project id cannot be empty".to_owned(),
        ));
    }
    let capture_item_id = input
        .capture_item_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let Some(capture_item_id) = capture_item_id {
        let candidate: Option<(String, String)> =
            sqlx::query_as("SELECT project_id, classification FROM capture_items WHERE id = ?")
                .bind(capture_item_id)
                .fetch_optional(pool)
                .await?;
        let Some((candidate_project_id, classification)) = candidate else {
            return Err(AppError::NotFound(format!(
                "capture {capture_item_id} in project {project_id}"
            )));
        };
        if candidate_project_id != project_id {
            return Err(AppError::NotFound(format!(
                "capture {capture_item_id} in project {project_id}"
            )));
        }
        if classification == "private" {
            return Err(AppError::Validation(
                "private captures cannot be used as a project cover".to_owned(),
            ));
        }
    }
    let project = sqlx::query_as::<_, Project>(
        r#"
        UPDATE projects
        SET cover_capture_item_id = ?,
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        WHERE id = ?
        RETURNING
            id, name, description, cover_asset_id, cover_capture_item_id,
            last_source_directory, last_destination_directory,
            destination_directory, created_at, updated_at
        "#,
    )
    .bind(capture_item_id)
    .bind(project_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound("project".to_owned()))?;
    Ok(project)
}

async fn canonical_existing_directory(
    path: &std::path::Path,
) -> Result<std::path::PathBuf, AppError> {
    let canonical = tokio::fs::canonicalize(path).await.map_err(|_| {
        AppError::Validation(format!(
            "source directory does not exist: {}",
            path.display()
        ))
    })?;
    if !canonical.is_dir() {
        return Err(AppError::Validation(
            "source path exists but is not a directory".to_owned(),
        ));
    }
    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    #[tokio::test]
    async fn creates_and_lists_projects() {
        let pool = db::test_pool().await;
        let created = create(
            &pool,
            CreateProjectInput {
                name: "  Cyberpunk Novel  ".to_owned(),
                description: Some("  World building  ".to_owned()),
                cover_asset_id: None,
            },
        )
        .await
        .expect("create project");

        assert_eq!(created.name, "Cyberpunk Novel");
        assert_eq!(created.description.as_deref(), Some("World building"));

        let projects = list(&pool).await.expect("list projects");
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].id, created.id);
    }

    #[tokio::test]
    async fn rejects_blank_project_names() {
        let pool = db::test_pool().await;
        let result = create(
            &pool,
            CreateProjectInput {
                name: "   ".to_owned(),
                description: None,
                cover_asset_id: None,
            },
        )
        .await;

        assert!(matches!(result, Err(AppError::Validation(_))));
    }

    #[tokio::test]
    async fn lists_project_overviews_with_capture_and_session_counts() {
        let pool = db::test_pool().await;
        let active = create(
            &pool,
            CreateProjectInput {
                name: "Active Project".to_owned(),
                description: Some("Currently capturing".to_owned()),
                cover_asset_id: None,
            },
        )
        .await
        .expect("create active project");
        let empty = create(
            &pool,
            CreateProjectInput {
                name: "Empty Project".to_owned(),
                description: None,
                cover_asset_id: None,
            },
        )
        .await
        .expect("create empty project");

        sqlx::query(
            "INSERT INTO capture_sessions (id, project_id, status) VALUES ('session-1', ?, 'active')",
        )
        .bind(&active.id)
        .execute(&pool)
        .await
        .expect("insert session");
        for (id, status, captured_at) in [
            ("capture-1", "awaiting_label", "2026-08-08T01:00:00Z"),
            ("capture-2", "processing", "2026-08-08T02:00:00Z"),
            ("capture-3", "completed", "2026-08-08T03:00:00Z"),
            ("capture-4", "failed", "2026-08-08T04:00:00Z"),
        ] {
            sqlx::query(
                r#"
                INSERT INTO capture_items (
                    id, project_id, session_id, source_path, classification, status, captured_at
                ) VALUES (?, ?, 'session-1', ?, 'person', ?, ?)
                "#,
            )
            .bind(id)
            .bind(&active.id)
            .bind(format!("C:\\shots\\{id}.png"))
            .bind(status)
            .bind(captured_at)
            .execute(&pool)
            .await
            .expect("insert capture item");
        }
        sqlx::query(
            r#"
            INSERT INTO capture_items (
                id, project_id, session_id, source_path, classification, status, captured_at
            ) VALUES ('capture-private', ?, 'session-1', 'C:\shots\private.png', 'private', 'completed', '2026-08-08T05:00:00Z')
            "#,
        )
        .bind(&active.id)
        .execute(&pool)
        .await
        .expect("insert private capture item");

        let summaries = list_overviews(&pool).await.expect("list overviews");
        let active_summary = summaries
            .iter()
            .find(|summary| summary.project_id == active.id)
            .expect("active summary");
        assert_eq!(active_summary.session_count, 1);
        assert_eq!(active_summary.active_session_count, 1);
        assert_eq!(active_summary.capture_count, 4);
        assert_eq!(active_summary.awaiting_count, 1);
        assert_eq!(active_summary.processing_count, 1);
        assert_eq!(active_summary.completed_count, 1);
        assert_eq!(active_summary.failed_count, 1);
        assert_eq!(
            active_summary.latest_capture_item_id.as_deref(),
            Some("capture-4")
        );

        let empty_summary = summaries
            .iter()
            .find(|summary| summary.project_id == empty.id)
            .expect("empty summary");
        assert_eq!(empty_summary.capture_count, 0);
        assert_eq!(empty_summary.session_count, 0);
        assert!(empty_summary.latest_capture_item_id.is_none());
    }

    #[tokio::test]
    async fn set_cover_pins_and_clears_a_project_capture() {
        let pool = db::test_pool().await;
        let project = create(
            &pool,
            CreateProjectInput {
                name: "Cover Project".to_owned(),
                description: None,
                cover_asset_id: None,
            },
        )
        .await
        .expect("create project");
        let other = create(
            &pool,
            CreateProjectInput {
                name: "Other Project".to_owned(),
                description: None,
                cover_asset_id: None,
            },
        )
        .await
        .expect("create other project");
        sqlx::query("INSERT INTO capture_sessions (id, project_id, status) VALUES ('cover-session', ?, 'active')")
            .bind(&project.id)
            .execute(&pool)
            .await
            .expect("insert session");
        for (id, classification) in [
            ("cover-capture-person", "person"),
            ("cover-capture-scene", "scene"),
            ("cover-capture-private", "private"),
            ("cover-capture-unclassified", "unclassified"),
        ] {
            sqlx::query(
                r#"
                INSERT INTO capture_items (
                    id, project_id, session_id, source_path, classification, status, captured_at
                ) VALUES (?, ?, 'cover-session', ?, ?, 'completed', '2026-08-08T01:00:00Z')
                "#,
            )
            .bind(id)
            .bind(&project.id)
            .bind(format!("C:\\shots\\{id}.png"))
            .bind(classification)
            .execute(&pool)
            .await
            .expect("insert capture item");
        }

        // Every Home-visible classification can pin.
        for capture_item_id in [
            "cover-capture-person",
            "cover-capture-scene",
            "cover-capture-unclassified",
        ] {
            let pinned = set_cover(
                &pool,
                SetProjectCoverInput {
                    project_id: project.id.clone(),
                    capture_item_id: Some(capture_item_id.to_owned()),
                },
            )
            .await
            .expect("pin cover");
            assert_eq!(
                pinned.cover_capture_item_id.as_deref(),
                Some(capture_item_id)
            );
        }

        // The overview exposes the pinned cover.
        let summaries = list_overviews(&pool).await.expect("list overviews");
        let summary = summaries
            .iter()
            .find(|summary| summary.project_id == project.id)
            .expect("summary");
        assert_eq!(
            summary.cover_capture_item_id.as_deref(),
            Some("cover-capture-unclassified")
        );

        let error = set_cover(
            &pool,
            SetProjectCoverInput {
                project_id: project.id.clone(),
                capture_item_id: Some("cover-capture-private".to_owned()),
            },
        )
        .await
        .expect_err("private capture must not become a Home cover");
        assert!(matches!(error, AppError::Validation(_)));

        // Defense in depth for older/stale databases: even if a private cover
        // id is present, the Home overview must never expose it.
        sqlx::query(
            "UPDATE projects SET cover_capture_item_id = 'cover-capture-private' WHERE id = ?",
        )
        .bind(&project.id)
        .execute(&pool)
        .await
        .expect("seed stale private cover");
        let summaries = list_overviews(&pool).await.expect("list overviews");
        let summary = summaries
            .iter()
            .find(|summary| summary.project_id == project.id)
            .expect("summary");
        assert!(summary.cover_capture_item_id.is_none());

        // A capture from another project is rejected.
        let error = set_cover(
            &pool,
            SetProjectCoverInput {
                project_id: other.id.clone(),
                capture_item_id: Some("cover-capture-person".to_owned()),
            },
        )
        .await
        .expect_err("cross-project capture must be rejected");
        assert!(
            matches!(error, AppError::NotFound(_)),
            "expected not found, got {error:?}"
        );

        // Clearing restores the latest-capture fallback.
        let cleared = set_cover(
            &pool,
            SetProjectCoverInput {
                project_id: project.id,
                capture_item_id: None,
            },
        )
        .await
        .expect("clear cover");
        assert!(cleared.cover_capture_item_id.is_none());
    }

    #[tokio::test]
    async fn renames_project_and_rejects_blank_names() {
        let pool = db::test_pool().await;
        let created = create(
            &pool,
            CreateProjectInput {
                name: "Old Name".to_owned(),
                description: None,
                cover_asset_id: None,
            },
        )
        .await
        .expect("create project");

        let renamed = rename(
            &pool,
            RenameProjectInput {
                project_id: created.id.clone(),
                name: "  New Name  ".to_owned(),
            },
        )
        .await
        .expect("rename project");
        assert_eq!(renamed.name, "New Name");
        assert_eq!(renamed.id, created.id);

        let blank = rename(
            &pool,
            RenameProjectInput {
                project_id: created.id,
                name: "   ".to_owned(),
            },
        )
        .await;
        assert!(matches!(blank, Err(AppError::Validation(_))));

        let missing = rename(
            &pool,
            RenameProjectInput {
                project_id: "missing-project".to_owned(),
                name: "Anything".to_owned(),
            },
        )
        .await;
        assert!(matches!(missing, Err(AppError::NotFound(_))));
    }

    #[tokio::test]
    async fn delete_project_cascades_data_and_removes_only_orphan_assets() {
        let pool = db::test_pool().await;
        let project_a = create(
            &pool,
            CreateProjectInput {
                name: "Project A".to_owned(),
                description: None,
                cover_asset_id: None,
            },
        )
        .await
        .expect("create project a");
        let project_b = create(
            &pool,
            CreateProjectInput {
                name: "Project B".to_owned(),
                description: None,
                cover_asset_id: None,
            },
        )
        .await
        .expect("create project b");

        // Assets: one only linked to A (orphan on delete), one shared A+B.
        sqlx::query(
            "INSERT INTO assets (id, name, asset_type, path) VALUES ('asset-1', 'a1', 'image', 'p1')",
        )
        .execute(&pool)
        .await
        .expect("asset 1");
        sqlx::query(
            "INSERT INTO assets (id, name, asset_type, path) VALUES ('asset-2', 'a2', 'image', 'p2')",
        )
        .execute(&pool)
        .await
        .expect("asset 2");
        sqlx::query("INSERT INTO project_assets (project_id, asset_id) VALUES (?, 'asset-1')")
            .bind(&project_a.id)
            .execute(&pool)
            .await
            .expect("link a1");
        sqlx::query("INSERT INTO project_assets (project_id, asset_id) VALUES (?, 'asset-2')")
            .bind(&project_a.id)
            .execute(&pool)
            .await
            .expect("link a2 to a");
        sqlx::query("INSERT INTO project_assets (project_id, asset_id) VALUES (?, 'asset-2')")
            .bind(&project_b.id)
            .execute(&pool)
            .await
            .expect("link a2 to b");

        // Project A owns one active session, one capture, one character, one
        // note and one collection.
        sqlx::query(
            "INSERT INTO capture_sessions (id, project_id, status) VALUES ('session-a', ?, 'active')",
        )
        .bind(&project_a.id)
        .execute(&pool)
        .await
        .expect("session");
        sqlx::query(
            r#"
            INSERT INTO capture_items (
                id, project_id, session_id, source_path, classification, status, captured_at
            ) VALUES ('capture-a', ?, 'session-a', 'shot.png', 'person', 'awaiting_label',
                      strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
            "#,
        )
        .bind(&project_a.id)
        .execute(&pool)
        .await
        .expect("capture");
        sqlx::query("INSERT INTO characters (id, project_id, name) VALUES ('char-a', ?, 'A')")
            .bind(&project_a.id)
            .execute(&pool)
            .await
            .expect("character");
        sqlx::query(
            r#"
            INSERT INTO project_notes (id, project_id, destination_directory, remote_path)
            VALUES ('note-a', ?, 'D:\\archive', 'note.md')
            "#,
        )
        .bind(&project_a.id)
        .execute(&pool)
        .await
        .expect("note");
        sqlx::query("INSERT INTO collections (id, project_id, name) VALUES ('col-a', ?, 'World')")
            .bind(&project_a.id)
            .execute(&pool)
            .await
            .expect("collection");

        let preview = preview_deletion(&pool, &project_a.id)
            .await
            .expect("preview");
        assert_eq!(preview.capture_count, 1);
        assert_eq!(preview.character_count, 1);
        assert_eq!(preview.session_count, 1);
        assert!(preview.has_active_session);
        assert_eq!(preview.note_count, 1);
        assert_eq!(preview.collection_count, 1);
        assert_eq!(preview.orphan_asset_count, 1, "only asset-1 is A-only");

        let deleted = delete_project(&pool, &project_a.id)
            .await
            .expect("delete project");
        assert_eq!(deleted.capture_count, 1);

        let project_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM projects")
            .fetch_one(&pool)
            .await
            .expect("project count");
        assert_eq!(project_count, 1, "project A must be gone");
        let remaining: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM capture_items WHERE project_id = 'project-a'")
                .fetch_one(&pool)
                .await
                .expect("remaining items");
        assert_eq!(remaining, 0);
        let session_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM capture_sessions")
            .fetch_one(&pool)
            .await
            .expect("session count");
        assert_eq!(session_count, 0);
        let character_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM characters")
            .fetch_one(&pool)
            .await
            .expect("character count");
        assert_eq!(character_count, 0);
        let note_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM project_notes")
            .fetch_one(&pool)
            .await
            .expect("note count");
        assert_eq!(note_count, 0);
        let collection_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM collections")
            .fetch_one(&pool)
            .await
            .expect("collection count");
        assert_eq!(collection_count, 0);
        let assets: Vec<String> = sqlx::query_scalar("SELECT id FROM assets ORDER BY id")
            .fetch_all(&pool)
            .await
            .expect("assets");
        assert_eq!(
            assets,
            vec!["asset-2".to_owned()],
            "orphan asset-1 is removed, shared asset-2 stays"
        );
        let b_links: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM project_assets WHERE project_id = ?")
                .bind(&project_b.id)
                .fetch_one(&pool)
                .await
                .expect("b links");
        assert_eq!(b_links, 1, "project B keeps its shared link");
    }
}
