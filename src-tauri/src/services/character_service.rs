use sqlx::SqlitePool;
use std::collections::HashSet;
use uuid::Uuid;

use crate::{
    error::AppError,
    models::character::{
        Character, CharacterSummary, CreateCharacterInput, MergeCharactersInput,
        RenameCharacterInput, SetCharacterAvatarInput,
    },
};

pub async fn create(pool: &SqlitePool, input: CreateCharacterInput) -> Result<Character, AppError> {
    let project_id = input.project_id.trim();
    let name = input.name.trim();

    if project_id.is_empty() {
        return Err(AppError::Validation(
            "character project id cannot be empty".to_owned(),
        ));
    }
    if name.is_empty() {
        return Err(AppError::Validation(
            "character name cannot be empty".to_owned(),
        ));
    }

    let aliases_json = input.aliases_json.unwrap_or_else(|| "[]".to_owned());
    let aliases = serde_json::from_str::<serde_json::Value>(&aliases_json)
        .map_err(|error| AppError::Validation(format!("invalid aliases JSON: {error}")))?;
    if !aliases.is_array() {
        return Err(AppError::Validation(
            "character aliases JSON must be an array".to_owned(),
        ));
    }

    let character = sqlx::query_as::<_, Character>(
        r#"
        INSERT INTO characters (id, project_id, name, aliases_json)
        VALUES (?, ?, ?, ?)
        RETURNING
            id, project_id, name, aliases_json, avatar_asset_id,
            created_at, updated_at
        "#,
    )
    .bind(Uuid::new_v4().to_string())
    .bind(project_id)
    .bind(name)
    .bind(aliases_json)
    .fetch_one(pool)
    .await?;

    Ok(character)
}

pub async fn list(pool: &SqlitePool, project_id: &str) -> Result<Vec<Character>, AppError> {
    if project_id.trim().is_empty() {
        return Err(AppError::Validation(
            "character project id cannot be empty".to_owned(),
        ));
    }

    let characters = sqlx::query_as::<_, Character>(
        r#"
        SELECT
            id, project_id, name, aliases_json, avatar_asset_id,
            created_at, updated_at
        FROM characters
        WHERE project_id = ?
        ORDER BY name COLLATE NOCASE ASC
        "#,
    )
    .bind(project_id.trim())
    .fetch_all(pool)
    .await?;

    Ok(characters)
}

pub(crate) async fn get(pool: &SqlitePool, character_id: &str) -> Result<Character, AppError> {
    let character = sqlx::query_as::<_, Character>(
        r#"
        SELECT
            id, project_id, name, aliases_json, avatar_asset_id,
            created_at, updated_at
        FROM characters
        WHERE id = ?
        "#,
    )
    .bind(character_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| AppError::NotFound("character".to_owned()))?;
    Ok(character)
}

/// Renames a character. Existing archive file names are not rewritten; the
/// new name applies to future archives only. The name must stay unique within
/// the project (case-insensitive, enforced by the DB collation).
pub async fn rename(pool: &SqlitePool, input: RenameCharacterInput) -> Result<Character, AppError> {
    let character_id = input.character_id.trim();
    let name = input.name.trim();
    if character_id.is_empty() {
        return Err(AppError::Validation(
            "character id cannot be empty".to_owned(),
        ));
    }
    if name.is_empty() {
        return Err(AppError::Validation(
            "character name cannot be empty".to_owned(),
        ));
    }

    let current = get(pool, character_id).await?;
    if current.name == name {
        return Ok(current);
    }
    let duplicate: i64 = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM characters WHERE project_id = ? AND name = ? AND id != ?)",
    )
    .bind(&current.project_id)
    .bind(name)
    .bind(character_id)
    .fetch_one(pool)
    .await?;
    if duplicate != 0 {
        return Err(AppError::Validation(format!(
            "character name {name:?} already exists in this project"
        )));
    }

    let character = sqlx::query_as::<_, Character>(
        r#"
        UPDATE characters
        SET
            name = ?,
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        WHERE id = ?
        RETURNING
            id, project_id, name, aliases_json, avatar_asset_id,
            created_at, updated_at
        "#,
    )
    .bind(name)
    .bind(character_id)
    .fetch_one(pool)
    .await?;
    Ok(character)
}

/// Merges `source` into `target`: capture items, suggestions, asset links and
/// face samples are re-pointed, the source's aliases (and its old name) are
/// appended to the target, and the target adopts the source's avatar when it
/// has none. The source character row is then deleted.
pub async fn merge(pool: &SqlitePool, input: MergeCharactersInput) -> Result<Character, AppError> {
    let source_id = input.source_character_id.trim();
    let target_id = input.target_character_id.trim();
    if source_id.is_empty() || target_id.is_empty() {
        return Err(AppError::Validation(
            "source and target character ids cannot be empty".to_owned(),
        ));
    }
    if source_id == target_id {
        return Err(AppError::Validation(
            "cannot merge a character into itself".to_owned(),
        ));
    }
    let source = get(pool, source_id).await?;
    let target = get(pool, target_id).await?;
    if source.project_id != target.project_id {
        return Err(AppError::Validation(
            "cannot merge characters from different projects".to_owned(),
        ));
    }

    let aliases_json = merge_aliases(&target.aliases_json, &source.aliases_json, &source.name)?;
    let avatar_asset_id = target
        .avatar_asset_id
        .clone()
        .or_else(|| source.avatar_asset_id.clone());

    let mut transaction = pool.begin().await?;
    sqlx::query("UPDATE capture_items SET character_id = ? WHERE character_id = ?")
        .bind(target_id)
        .bind(source_id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query(
        "UPDATE capture_items SET suggested_character_id = ? WHERE suggested_character_id = ?",
    )
    .bind(target_id)
    .bind(source_id)
    .execute(&mut *transaction)
    .await?;
    // Keep the target's existing asset links; drop source duplicates.
    sqlx::query("UPDATE OR IGNORE asset_characters SET character_id = ? WHERE character_id = ?")
        .bind(target_id)
        .bind(source_id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query("DELETE FROM asset_characters WHERE character_id = ?")
        .bind(source_id)
        .execute(&mut *transaction)
        .await?;
    sqlx::query(
        "UPDATE OR IGNORE character_face_samples SET character_id = ? WHERE character_id = ?",
    )
    .bind(target_id)
    .bind(source_id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query("DELETE FROM character_face_samples WHERE character_id = ?")
        .bind(source_id)
        .execute(&mut *transaction)
        .await?;
    let merged = sqlx::query_as::<_, Character>(
        r#"
        UPDATE characters
        SET
            aliases_json = ?,
            avatar_asset_id = ?,
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        WHERE id = ?
        RETURNING
            id, project_id, name, aliases_json, avatar_asset_id,
            created_at, updated_at
        "#,
    )
    .bind(&aliases_json)
    .bind(&avatar_asset_id)
    .bind(target_id)
    .fetch_one(&mut *transaction)
    .await?;
    sqlx::query("DELETE FROM characters WHERE id = ?")
        .bind(source_id)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(merged)
}

/// Sets (or clears) the character's representative avatar. The asset must
/// belong to the character's project.
pub async fn set_avatar(
    pool: &SqlitePool,
    input: SetCharacterAvatarInput,
) -> Result<Character, AppError> {
    let character_id = input.character_id.trim();
    if character_id.is_empty() {
        return Err(AppError::Validation(
            "character id cannot be empty".to_owned(),
        ));
    }
    get(pool, character_id).await?;
    if let Some(avatar_asset_id) = input.avatar_asset_id.as_deref() {
        if avatar_asset_id.trim().is_empty() {
            return Err(AppError::Validation(
                "avatar asset id cannot be empty".to_owned(),
            ));
        }
        let linked: i64 = sqlx::query_scalar(
            r#"
            SELECT EXISTS (
                SELECT 1
                FROM project_assets link
                JOIN characters character
                    ON character.id = ? AND character.project_id = link.project_id
                WHERE link.asset_id = ?
            )
            "#,
        )
        .bind(character_id)
        .bind(avatar_asset_id)
        .fetch_one(pool)
        .await?;
        if linked == 0 {
            return Err(AppError::Validation(
                "avatar asset does not belong to this character's project".to_owned(),
            ));
        }
    }

    let character = sqlx::query_as::<_, Character>(
        r#"
        UPDATE characters
        SET
            avatar_asset_id = ?,
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        WHERE id = ?
        RETURNING
            id, project_id, name, aliases_json, avatar_asset_id,
            created_at, updated_at
        "#,
    )
    .bind(&input.avatar_asset_id)
    .bind(character_id)
    .fetch_one(pool)
    .await?;
    Ok(character)
}

/// Workbench overview: one row per character with capture counts, pending
/// suggestion counts (suggestions pointing at this character awaiting human
/// review) and the most recent capture timestamp.
pub async fn list_project_summaries(
    pool: &SqlitePool,
    project_id: &str,
) -> Result<Vec<CharacterSummary>, AppError> {
    if project_id.trim().is_empty() {
        return Err(AppError::Validation(
            "character project id cannot be empty".to_owned(),
        ));
    }
    let summaries = sqlx::query_as::<_, CharacterSummary>(
        r#"
        SELECT
            character.id,
            character.name,
            character.aliases_json,
            character.avatar_asset_id,
            (
                SELECT representative.id
                FROM capture_items representative
                WHERE representative.asset_id = character.avatar_asset_id
                  AND representative.classification = 'person'
                  AND representative.avatar_path IS NOT NULL
                ORDER BY representative.captured_at DESC, representative.created_at DESC
                LIMIT 1
            ) AS avatar_capture_item_id,
            COUNT(item.id) AS capture_count,
            (
                SELECT COUNT(*)
                FROM capture_items review_item
                WHERE review_item.character_id = character.id
                  AND review_item.review_status = 'pending'
            ) AS pending_review_count,
            (
                SELECT COUNT(*)
                FROM character_face_samples sample
                WHERE sample.character_id = character.id
                  AND sample.status = 'active'
            ) AS sample_count,
            (
                SELECT COUNT(*)
                FROM capture_items degraded
                WHERE degraded.character_id = character.id
                  AND degraded.classification = 'person'
                  AND degraded.status = 'completed'
                  AND degraded.annotated_path IS NULL
                  AND degraded.face_box_json IS NULL
            ) AS degraded_count,
            MAX(item.captured_at) AS last_captured_at,
            (
                SELECT latest.id
                FROM capture_items latest
                WHERE latest.character_id = character.id
                  AND latest.classification = 'person'
                ORDER BY latest.captured_at DESC, latest.created_at DESC
                LIMIT 1
            ) AS latest_capture_item_id,
            (
                SELECT latest.avatar_path
                FROM capture_items latest
                WHERE latest.character_id = character.id
                  AND latest.classification = 'person'
                ORDER BY latest.captured_at DESC, latest.created_at DESC
                LIMIT 1
            ) AS latest_avatar_path
        FROM characters character
        LEFT JOIN capture_items item
            ON item.character_id = character.id
            AND item.classification = 'person'
        WHERE character.project_id = ?
        GROUP BY character.id
        ORDER BY character.name COLLATE NOCASE ASC
        "#,
    )
    .bind(project_id.trim())
    .fetch_all(pool)
    .await?;
    Ok(summaries)
}

fn merge_aliases(target: &str, source: &str, source_name: &str) -> Result<String, AppError> {
    let mut aliases: Vec<String> = serde_json::from_str(target).unwrap_or_default();
    let mut seen: HashSet<String> = aliases
        .iter()
        .map(|alias| alias.trim().to_ascii_lowercase())
        .collect();
    let mut push = |value: &str| {
        let trimmed = value.trim();
        if !trimmed.is_empty() && seen.insert(trimmed.to_ascii_lowercase()) {
            aliases.push(trimmed.to_owned());
        }
    };
    if let Ok(source_aliases) = serde_json::from_str::<Vec<String>>(source) {
        for alias in source_aliases {
            push(&alias);
        }
    }
    push(source_name);
    serde_json::to_string(&aliases)
        .map_err(|error| AppError::Validation(format!("cannot encode merged aliases: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        db,
        models::{
            capture::{LabelCaptureInput, RegisterCaptureInput},
            project::CreateProjectInput,
            recognition::SetRecognitionSuggestionInput,
        },
        services::{capture_service, project_service, recognition_service, test_support},
    };
    use tempfile::tempdir;

    async fn character(
        pool: &SqlitePool,
        project_id: &str,
        name: &str,
        aliases: Option<&str>,
    ) -> Character {
        create(
            pool,
            CreateCharacterInput {
                project_id: project_id.to_owned(),
                name: name.to_owned(),
                aliases_json: aliases.map(str::to_owned),
            },
        )
        .await
        .expect("create character")
    }

    async fn asset(pool: &SqlitePool, project_id: &str, id: &str) {
        sqlx::query(
            r#"
            INSERT INTO assets (id, name, asset_type, path, mime_type, file_size, status)
            VALUES (?, 'Asset', 'image', ?, 'image/png', 10, 'available')
            "#,
        )
        .bind(id)
        .bind(format!("C:\\tmp\\{id}.png"))
        .execute(pool)
        .await
        .expect("insert asset");
        sqlx::query("INSERT INTO project_assets (project_id, asset_id) VALUES (?, ?)")
            .bind(project_id)
            .bind(id)
            .execute(pool)
            .await
            .expect("link asset");
    }

    #[tokio::test]
    async fn creates_and_lists_project_characters() {
        let pool = db::test_pool().await;
        let project = project_service::create(
            &pool,
            CreateProjectInput {
                name: "Sample Game".to_owned(),
                description: None,
                cover_asset_id: None,
            },
        )
        .await
        .expect("create project");

        let created = create(
            &pool,
            CreateCharacterInput {
                project_id: project.id.clone(),
                name: " Aurora ".to_owned(),
                aliases_json: Some(r#"["Auri"]"#.to_owned()),
            },
        )
        .await
        .expect("create character");

        assert_eq!(created.name, "Aurora");
        let characters = list(&pool, &project.id).await.expect("list characters");
        assert_eq!(characters.len(), 1);
        assert_eq!(characters[0].id, created.id);
    }

    #[tokio::test]
    async fn rename_updates_name_and_rejects_duplicates() {
        let pool = db::test_pool().await;
        let project = project_service::create(
            &pool,
            CreateProjectInput {
                name: "Sample Game".to_owned(),
                description: None,
                cover_asset_id: None,
            },
        )
        .await
        .expect("project");
        let first = character(&pool, &project.id, "Ava", None).await;
        let second = character(&pool, &project.id, "Bella", None).await;

        let renamed = rename(
            &pool,
            RenameCharacterInput {
                character_id: first.id.clone(),
                name: "Ava:Prime".to_owned(),
            },
        )
        .await
        .expect("rename");
        assert_eq!(renamed.name, "Ava:Prime");

        let error = rename(
            &pool,
            RenameCharacterInput {
                character_id: first.id.clone(),
                name: second.name.clone(),
            },
        )
        .await
        .expect_err("duplicate name");
        assert!(matches!(error, AppError::Validation(_)));

        let unchanged = rename(
            &pool,
            RenameCharacterInput {
                character_id: first.id.clone(),
                name: "Ava:Prime".to_owned(),
            },
        )
        .await
        .expect("no-op rename");
        assert_eq!(unchanged.name, "Ava:Prime");
    }

    #[tokio::test]
    async fn merge_moves_references_aliases_and_avatar() {
        let pool = db::test_pool().await;
        let project = project_service::create(
            &pool,
            CreateProjectInput {
                name: "Sample Game".to_owned(),
                description: None,
                cover_asset_id: None,
            },
        )
        .await
        .expect("project");
        let source = character(&pool, &project.id, "Ava", Some(r#"["Auri","Ava"]"#)).await;
        let target = character(&pool, &project.id, "Bella", Some(r#"["Bels"]"#)).await;

        // A capture labeled to the source, and a pending suggestion that
        // points at the source from another capture.
        let workspace = tempdir().expect("tempdir");
        let source_dir = workspace.path().join("source");
        let archive_dir = workspace.path().join("archive");
        tokio::fs::create_dir(&source_dir)
            .await
            .expect("source dir");
        tokio::fs::create_dir(&archive_dir)
            .await
            .expect("archive dir");
        project_service::set_destination_directory(
            &pool,
            crate::models::project::SetProjectDestinationInput {
                project_id: project.id.clone(),
                directory: capture_service::path_to_string(&archive_dir),
            },
        )
        .await
        .expect("set destination");
        project_service::add_source_directory(
            &pool,
            crate::models::project::AddProjectSourceDirectoryInput {
                project_id: project.id.clone(),
                directory: capture_service::path_to_string(&source_dir),
            },
        )
        .await
        .expect("add source directory");
        let session = test_support::start_session(&pool, &project.id)
            .await
            .expect("session");
        let mut labeled_item_id = None;
        let mut suggested_item_id = None;
        for (name, classification) in [("one.png", "person"), ("two.png", "person")] {
            let path = source_dir.join(name);
            tokio::fs::write(&path, name).await.expect("source");
            let item = capture_service::register_capture(
                &pool,
                RegisterCaptureInput {
                    session_id: session.id.clone(),
                    source_path: capture_service::path_to_string(&path),
                },
            )
            .await
            .expect("register");
            let character_id = if name == "one.png" {
                labeled_item_id = Some(item.id.clone());
                source.id.clone()
            } else {
                suggested_item_id = Some(item.id.clone());
                target.id.clone()
            };
            capture_service::label_capture(
                &pool,
                LabelCaptureInput {
                    capture_item_id: item.id,
                    character_id: Some(character_id),
                    classification: Some(classification.to_owned()),
                },
            )
            .await
            .expect("label");
        }
        recognition_service::set_suggestion(
            &pool,
            SetRecognitionSuggestionInput {
                capture_item_id: suggested_item_id.clone().expect("suggested item"),
                suggested_character_id: Some(source.id.clone()),
                confidence: Some(0.8),
                source: Some("face_bank".to_owned()),
            },
        )
        .await
        .expect("suggest");

        // Asset links: one asset already linked to the target (duplicate
        // after merge must collapse) and one to the source.
        asset(&pool, &project.id, "asset-source").await;
        asset(&pool, &project.id, "asset-target").await;
        sqlx::query(
            "INSERT INTO asset_characters (asset_id, character_id, is_primary) VALUES (?, ?, 1)",
        )
        .bind("asset-source")
        .bind(&source.id)
        .execute(&pool)
        .await
        .expect("asset character source");
        sqlx::query(
            "INSERT INTO asset_characters (asset_id, character_id, is_primary) VALUES (?, ?, 1)",
        )
        .bind("asset-target")
        .bind(&target.id)
        .execute(&pool)
        .await
        .expect("asset character target");
        sqlx::query(
            "INSERT INTO asset_characters (asset_id, character_id, is_primary) VALUES (?, ?, 1)",
        )
        .bind("asset-source")
        .bind(&target.id)
        .execute(&pool)
        .await
        .expect("asset character duplicate");

        // A face sample for the source, plus a duplicate sample pair.
        sqlx::query(
            r#"
            INSERT INTO character_face_samples
                (id, character_id, capture_item_id, feature_json, confidence, status)
            VALUES (?, ?, ?, '[]', 0.9, 'active')
            "#,
        )
        .bind("sample-source")
        .bind(&source.id)
        .bind(labeled_item_id.as_deref().expect("labeled item"))
        .execute(&pool)
        .await
        .expect("face sample source");
        sqlx::query(
            r#"
            INSERT INTO character_face_samples
                (id, character_id, capture_item_id, feature_json, confidence, status)
            VALUES (?, ?, ?, '[]', 0.7, 'active')
            "#,
        )
        .bind("sample-duplicate")
        .bind(&source.id)
        .bind(suggested_item_id.as_deref().expect("suggested item"))
        .execute(&pool)
        .await
        .expect("face sample duplicate");
        sqlx::query(
            r#"
            INSERT INTO character_face_samples
                (id, character_id, capture_item_id, feature_json, confidence, status)
            VALUES (?, ?, ?, '[]', 0.6, 'active')
            "#,
        )
        .bind("sample-target")
        .bind(&target.id)
        .bind(suggested_item_id.as_deref().expect("suggested item"))
        .execute(&pool)
        .await
        .expect("face sample target");

        sqlx::query("UPDATE characters SET avatar_asset_id = 'asset-source' WHERE id = ?")
            .bind(&source.id)
            .execute(&pool)
            .await
            .expect("source avatar");

        let merged = merge(
            &pool,
            MergeCharactersInput {
                source_character_id: source.id.clone(),
                target_character_id: target.id.clone(),
            },
        )
        .await
        .expect("merge");
        assert_eq!(merged.id, target.id);
        assert_eq!(merged.name, "Bella");
        assert_eq!(merged.avatar_asset_id.as_deref(), Some("asset-source"));
        let aliases: Vec<String> = serde_json::from_str(&merged.aliases_json).expect("aliases");
        assert_eq!(aliases, vec!["Bels", "Auri", "Ava"]);
        assert!(matches!(
            get(&pool, &source.id).await,
            Err(AppError::NotFound(_))
        ));

        let labeled = capture_service::get_item(&pool, labeled_item_id.as_deref().expect("id"))
            .await
            .expect("labeled item");
        assert_eq!(labeled.character_id.as_deref(), Some(target.id.as_str()));
        let suggested = capture_service::get_item(&pool, suggested_item_id.as_deref().expect("id"))
            .await
            .expect("suggested item");
        assert_eq!(
            suggested.suggested_character_id.as_deref(),
            Some(target.id.as_str())
        );

        let target_links: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM asset_characters WHERE asset_id = 'asset-source' AND character_id = ?",
        )
        .bind(&target.id)
        .fetch_one(&pool)
        .await
        .expect("asset link count");
        assert_eq!(target_links, 1, "duplicate asset link must collapse");
        let source_links: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM asset_characters WHERE character_id = ?")
                .bind(&source.id)
                .fetch_one(&pool)
                .await
                .expect("source link count");
        assert_eq!(source_links, 0);

        let sample_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM character_face_samples WHERE character_id = ?",
        )
        .bind(&source.id)
        .fetch_one(&pool)
        .await
        .expect("source sample count");
        assert_eq!(sample_count, 0);
        let target_samples: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM character_face_samples WHERE character_id = ?",
        )
        .bind(&target.id)
        .fetch_one(&pool)
        .await
        .expect("target sample count");
        assert_eq!(target_samples, 2, "both distinct captures keep a sample");
        let suggested_capture_samples: i64 = sqlx::query_scalar(
            r#"
            SELECT COUNT(*)
            FROM character_face_samples
            WHERE character_id = ? AND capture_item_id = ?
            "#,
        )
        .bind(&target.id)
        .bind(suggested_item_id.as_deref().expect("suggested item"))
        .fetch_one(&pool)
        .await
        .expect("duplicate capture sample count");
        assert_eq!(
            suggested_capture_samples, 1,
            "duplicate sample for the same capture must collapse"
        );
    }

    #[tokio::test]
    async fn merge_keeps_the_target_representative_avatar() {
        let pool = db::test_pool().await;
        let project = project_service::create(
            &pool,
            CreateProjectInput {
                name: "Sample Game".to_owned(),
                description: None,
                cover_asset_id: None,
            },
        )
        .await
        .expect("project");
        let source = character(&pool, &project.id, "Ava", None).await;
        let target = character(&pool, &project.id, "Bella", None).await;
        asset(&pool, &project.id, "asset-source-avatar").await;
        asset(&pool, &project.id, "asset-target-avatar").await;
        set_avatar(
            &pool,
            SetCharacterAvatarInput {
                character_id: source.id.clone(),
                avatar_asset_id: Some("asset-source-avatar".to_owned()),
            },
        )
        .await
        .expect("source avatar");
        set_avatar(
            &pool,
            SetCharacterAvatarInput {
                character_id: target.id.clone(),
                avatar_asset_id: Some("asset-target-avatar".to_owned()),
            },
        )
        .await
        .expect("target avatar");

        let merged = merge(
            &pool,
            MergeCharactersInput {
                source_character_id: source.id,
                target_character_id: target.id,
            },
        )
        .await
        .expect("merge");

        assert_eq!(
            merged.avatar_asset_id.as_deref(),
            Some("asset-target-avatar")
        );
    }

    #[tokio::test]
    async fn merge_rejects_self_and_different_projects() {
        let pool = db::test_pool().await;
        let project = project_service::create(
            &pool,
            CreateProjectInput {
                name: "Sample Game".to_owned(),
                description: None,
                cover_asset_id: None,
            },
        )
        .await
        .expect("project");
        let other_project = project_service::create(
            &pool,
            CreateProjectInput {
                name: "Other".to_owned(),
                description: None,
                cover_asset_id: None,
            },
        )
        .await
        .expect("other project");
        let first = character(&pool, &project.id, "Ava", None).await;
        let second = character(&pool, &project.id, "Bella", None).await;
        let stranger = character(&pool, &other_project.id, "Stranger", None).await;

        let error = merge(
            &pool,
            MergeCharactersInput {
                source_character_id: first.id.clone(),
                target_character_id: first.id.clone(),
            },
        )
        .await
        .expect_err("self merge");
        assert!(matches!(error, AppError::Validation(_)));

        let error = merge(
            &pool,
            MergeCharactersInput {
                source_character_id: first.id,
                target_character_id: stranger.id,
            },
        )
        .await
        .expect_err("cross project merge");
        assert!(matches!(error, AppError::Validation(_)));
        assert!(get(&pool, &second.id).await.is_ok());
    }

    #[tokio::test]
    async fn set_avatar_requires_project_linked_asset() {
        let pool = db::test_pool().await;
        let project = project_service::create(
            &pool,
            CreateProjectInput {
                name: "Sample Game".to_owned(),
                description: None,
                cover_asset_id: None,
            },
        )
        .await
        .expect("project");
        let other_project = project_service::create(
            &pool,
            CreateProjectInput {
                name: "Other".to_owned(),
                description: None,
                cover_asset_id: None,
            },
        )
        .await
        .expect("other project");
        let ava = character(&pool, &project.id, "Ava", None).await;
        asset(&pool, &project.id, "asset-own").await;
        asset(&pool, &other_project.id, "asset-other").await;

        let updated = set_avatar(
            &pool,
            SetCharacterAvatarInput {
                character_id: ava.id.clone(),
                avatar_asset_id: Some("asset-own".to_owned()),
            },
        )
        .await
        .expect("set avatar");
        assert_eq!(updated.avatar_asset_id.as_deref(), Some("asset-own"));

        let error = set_avatar(
            &pool,
            SetCharacterAvatarInput {
                character_id: ava.id.clone(),
                avatar_asset_id: Some("asset-other".to_owned()),
            },
        )
        .await
        .expect_err("foreign asset");
        assert!(matches!(error, AppError::Validation(_)));

        let cleared = set_avatar(
            &pool,
            SetCharacterAvatarInput {
                character_id: ava.id,
                avatar_asset_id: None,
            },
        )
        .await
        .expect("clear avatar");
        assert!(cleared.avatar_asset_id.is_none());
    }

    #[tokio::test]
    async fn summaries_aggregate_capture_and_pending_review_counts() {
        let pool = db::test_pool().await;
        let project = project_service::create(
            &pool,
            CreateProjectInput {
                name: "Sample Game".to_owned(),
                description: None,
                cover_asset_id: None,
            },
        )
        .await
        .expect("project");
        let ava = character(&pool, &project.id, "Ava", None).await;
        let bella = character(&pool, &project.id, "Bella", None).await;

        let workspace = tempdir().expect("tempdir");
        let source_dir = workspace.path().join("source");
        let archive_dir = workspace.path().join("archive");
        tokio::fs::create_dir(&source_dir)
            .await
            .expect("source dir");
        tokio::fs::create_dir(&archive_dir)
            .await
            .expect("archive dir");
        project_service::set_destination_directory(
            &pool,
            crate::models::project::SetProjectDestinationInput {
                project_id: project.id.clone(),
                directory: capture_service::path_to_string(&archive_dir),
            },
        )
        .await
        .expect("set destination");
        project_service::add_source_directory(
            &pool,
            crate::models::project::AddProjectSourceDirectoryInput {
                project_id: project.id.clone(),
                directory: capture_service::path_to_string(&source_dir),
            },
        )
        .await
        .expect("add source directory");
        let session = test_support::start_session(&pool, &project.id)
            .await
            .expect("session");
        let mut suggested_item = None;
        let mut first_item_id = None;
        for name in ["one.png", "two.png"] {
            let path = source_dir.join(name);
            tokio::fs::write(&path, name).await.expect("source");
            let item = capture_service::register_capture(
                &pool,
                RegisterCaptureInput {
                    session_id: session.id.clone(),
                    source_path: capture_service::path_to_string(&path),
                },
            )
            .await
            .expect("register");
            let item = capture_service::label_capture(
                &pool,
                LabelCaptureInput {
                    capture_item_id: item.id,
                    character_id: Some(ava.id.clone()),
                    classification: Some("person".to_owned()),
                },
            )
            .await
            .expect("label");
            if first_item_id.is_none() {
                first_item_id = Some(item.id.clone());
            }
            if name == "two.png" {
                suggested_item = Some(item.id);
            }
        }
        recognition_service::set_suggestion(
            &pool,
            SetRecognitionSuggestionInput {
                capture_item_id: suggested_item.expect("item"),
                suggested_character_id: Some(bella.id.clone()),
                confidence: Some(0.6),
                source: Some("vision".to_owned()),
            },
        )
        .await
        .expect("suggest");

        // A scene capture that keeps the character id must not count toward
        // the person workbench overview.
        let scene_path = source_dir.join("scene.png");
        tokio::fs::write(&scene_path, b"scene")
            .await
            .expect("scene source");
        let scene_item = capture_service::register_capture(
            &pool,
            RegisterCaptureInput {
                session_id: session.id,
                source_path: capture_service::path_to_string(&scene_path),
            },
        )
        .await
        .expect("register scene");
        capture_service::label_capture(
            &pool,
            LabelCaptureInput {
                capture_item_id: scene_item.id,
                character_id: Some(ava.id.clone()),
                classification: Some("scene".to_owned()),
            },
        )
        .await
        .expect("label scene");

        let summaries = list_project_summaries(&pool, &project.id)
            .await
            .expect("summaries");
        assert_eq!(summaries.len(), 2);
        let ava_summary = summaries
            .iter()
            .find(|summary| summary.id == ava.id)
            .expect("ava summary");
        assert_eq!(ava_summary.capture_count, 2);
        // The pending suggestion sits on Ava's own capture (the item labeled
        // Ava has review_status pending), so the badge belongs to Ava even
        // though it points at Bella.
        assert_eq!(ava_summary.pending_review_count, 1);
        assert!(ava_summary.last_captured_at.is_some());
        assert!(ava_summary.latest_capture_item_id.is_some());
        assert!(ava_summary.latest_avatar_path.is_none());
        let bella_summary = summaries
            .iter()
            .find(|summary| summary.id == bella.id)
            .expect("bella summary");
        assert_eq!(bella_summary.capture_count, 0);
        assert_eq!(bella_summary.pending_review_count, 0);
        assert!(bella_summary.last_captured_at.is_none());
        assert!(bella_summary.latest_capture_item_id.is_none());

        // The overview card prefers the face-crop avatar of the newest capture
        // when the vision engine produced one.
        let latest_id = ava_summary.latest_capture_item_id.clone().expect("latest");
        let avatar_path = source_dir.join("latest-avatar.png");
        sqlx::query("UPDATE capture_items SET avatar_path = ? WHERE id = ?")
            .bind(avatar_path.to_str().expect("avatar path"))
            .bind(&latest_id)
            .execute(&pool)
            .await
            .expect("set avatar path");
        let updated = list_project_summaries(&pool, &project.id)
            .await
            .expect("updated summaries");
        let ava_updated = updated
            .iter()
            .find(|summary| summary.id == ava.id)
            .expect("ava summary");
        assert_eq!(
            ava_updated.latest_avatar_path.as_deref(),
            Some(avatar_path.to_str().expect("path"))
        );

        // An explicit representative avatar points at its capture even when
        // that capture is not the newest one. The workbench can then render
        // the user-selected crop instead of silently continuing to show the
        // latest image.
        let representative_item_id = first_item_id.as_deref().expect("first item");
        asset(&pool, &project.id, "representative-asset").await;
        let representative_avatar_path = source_dir.join("representative-avatar.png");
        sqlx::query("UPDATE capture_items SET asset_id = ?, avatar_path = ? WHERE id = ?")
            .bind("representative-asset")
            .bind(representative_avatar_path.to_str().expect("avatar path"))
            .bind(representative_item_id)
            .execute(&pool)
            .await
            .expect("set representative capture");
        set_avatar(
            &pool,
            SetCharacterAvatarInput {
                character_id: ava.id.clone(),
                avatar_asset_id: Some("representative-asset".to_owned()),
            },
        )
        .await
        .expect("set representative avatar");
        let representative = list_project_summaries(&pool, &project.id)
            .await
            .expect("representative summary");
        let ava_representative = representative
            .iter()
            .find(|summary| summary.id == ava.id)
            .expect("ava representative summary");
        assert_eq!(
            ava_representative.avatar_capture_item_id.as_deref(),
            Some(representative_item_id)
        );

        // Enrolling a face sample surfaces in the overview count.
        sqlx::query(
            r#"
            INSERT INTO capture_faces (
                id, capture_item_id, face_index, is_primary, feature_json,
                feature_model_id, feature_model_version, feature_dim,
                face_sharpness, face_area_ratio
            )
            VALUES (?, ?, 0, 1, ?, 'opencv-sface', '2021dec', 2, 50.0, 0.03)
            "#,
        )
        .bind(Uuid::new_v4().to_string())
        .bind(first_item_id.as_deref().expect("first item"))
        .bind("[1.0, 0.0]")
        .execute(&pool)
        .await
        .expect("set feature");
        recognition_service::enroll_face_sample(
            &pool,
            first_item_id.as_deref().expect("first item"),
        )
        .await
        .expect("enroll");
        let summaries = list_project_summaries(&pool, &project.id)
            .await
            .expect("summaries");
        let ava_summary = summaries
            .iter()
            .find(|summary| summary.id == ava.id)
            .expect("ava summary");
        assert_eq!(ava_summary.sample_count, 1);
    }
}
