use super::*;
use crate::{
    db,
    models::{
        capture::{LabelCaptureInput, RegisterCaptureInput, RelabelCaptureInput},
        character::CreateCharacterInput,
        project::CreateProjectInput,
        recognition::FACE_MODEL_ID,
        recognition::FACE_MODEL_VERSION,
    },
    services::{capture_service, character_service, project_service, test_support},
};
use tempfile::tempdir;

struct Fixture {
    _workspace: tempfile::TempDir,
    project_id: String,
    character_id: String,
    other_character_id: String,
    item_id: String,
    second_item_id: String,
}

async fn fixture(pool: &SqlitePool) -> Fixture {
    let project = project_service::create(
        pool,
        CreateProjectInput {
            name: "Recognition".to_owned(),
            description: None,
            cover_asset_id: None,
        },
    )
    .await
    .expect("project");
    let character = character_service::create(
        pool,
        CreateCharacterInput {
            project_id: project.id.clone(),
            name: "Ava".to_owned(),
            aliases_json: None,
        },
    )
    .await
    .expect("character");
    let other = character_service::create(
        pool,
        CreateCharacterInput {
            project_id: project.id.clone(),
            name: "Bella".to_owned(),
            aliases_json: None,
        },
    )
    .await
    .expect("other character");
    let workspace = tempdir().expect("tempdir");
    let source_directory = workspace.path().join("source");
    let destination_directory = workspace.path().join("archive");
    tokio::fs::create_dir(&source_directory)
        .await
        .expect("source directory");
    tokio::fs::create_dir(&destination_directory)
        .await
        .expect("destination directory");
    project_service::set_destination_directory(
        pool,
        crate::models::project::SetProjectDestinationInput {
            project_id: project.id.clone(),
            directory: capture_service::path_to_string(&destination_directory),
        },
    )
    .await
    .expect("set destination");
    project_service::add_source_directory(
        pool,
        crate::models::project::AddProjectSourceDirectoryInput {
            project_id: project.id.clone(),
            directory: capture_service::path_to_string(&source_directory),
        },
    )
    .await
    .expect("add source directory");
    let session = test_support::start_session(pool, &project.id)
        .await
        .expect("session");

    let mut item_ids = Vec::new();
    for name in ["first.png", "second.png"] {
        let source = source_directory.join(name);
        tokio::fs::write(&source, name).await.expect("source");
        let item = capture_service::register_capture(
            pool,
            RegisterCaptureInput {
                session_id: session.id.clone(),
                source_path: capture_service::path_to_string(&source),
            },
        )
        .await
        .expect("register");
        let item = capture_service::label_capture(
            pool,
            LabelCaptureInput {
                capture_item_id: item.id.clone(),
                character_id: Some(character.id.clone()),
                classification: Some("person".to_owned()),
            },
        )
        .await
        .expect("label");
        item_ids.push(item.id);
    }

    Fixture {
        _workspace: workspace,
        project_id: project.id,
        character_id: character.id,
        other_character_id: other.id,
        item_id: item_ids[0].clone(),
        second_item_id: item_ids[1].clone(),
    }
}

#[tokio::test]
async fn writes_and_clears_a_suggestion() {
    let pool = db::test_pool().await;
    let fixture = fixture(&pool).await;
    let item = set_suggestion(
        &pool,
        SetRecognitionSuggestionInput {
            capture_item_id: fixture.item_id.clone(),
            suggested_character_id: Some(fixture.other_character_id.clone()),
            confidence: Some(0.87),
            source: Some("face_bank".to_owned()),
        },
    )
    .await
    .expect("suggest");
    assert_eq!(
        item.suggested_character_id.as_deref(),
        Some(fixture.other_character_id.as_str())
    );
    assert_eq!(item.recognition_confidence, Some(0.87));
    assert_eq!(item.recognition_source.as_deref(), Some("face_bank"));
    assert_eq!(item.review_status, "pending");
    assert_eq!(
        item.character_id.as_deref(),
        Some(fixture.character_id.as_str())
    );

    let cleared = set_suggestion(
        &pool,
        SetRecognitionSuggestionInput {
            capture_item_id: fixture.item_id,
            suggested_character_id: None,
            confidence: None,
            source: None,
        },
    )
    .await
    .expect("clear");
    assert!(cleared.suggested_character_id.is_none());
    assert!(cleared.recognition_confidence.is_none());
    assert!(cleared.recognition_source.is_none());
    assert_eq!(cleared.review_status, "none");
}

#[tokio::test]
async fn rejects_invalid_suggestions() {
    let pool = db::test_pool().await;
    let fixture = fixture(&pool).await;
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
    let stranger = character_service::create(
        &pool,
        CreateCharacterInput {
            project_id: other_project.id.clone(),
            name: "Stranger".to_owned(),
            aliases_json: None,
        },
    )
    .await
    .expect("stranger");

    let error = set_suggestion(
        &pool,
        SetRecognitionSuggestionInput {
            capture_item_id: fixture.item_id.clone(),
            suggested_character_id: Some(stranger.id),
            confidence: Some(0.9),
            source: Some("vision".to_owned()),
        },
    )
    .await
    .expect_err("cross-project suggestion");
    assert!(matches!(error, AppError::Validation(_)));

    let error = set_suggestion(
        &pool,
        SetRecognitionSuggestionInput {
            capture_item_id: fixture.item_id.clone(),
            suggested_character_id: Some(fixture.character_id.clone()),
            confidence: Some(1.5),
            source: Some("vision".to_owned()),
        },
    )
    .await
    .expect_err("out of range confidence");
    assert!(matches!(error, AppError::Validation(_)));

    let error = set_suggestion(
        &pool,
        SetRecognitionSuggestionInput {
            capture_item_id: fixture.item_id,
            suggested_character_id: Some(fixture.character_id),
            confidence: Some(0.9),
            source: Some("magic".to_owned()),
        },
    )
    .await
    .expect_err("unknown source");
    assert!(matches!(error, AppError::Validation(_)));
}

#[tokio::test]
async fn accepts_and_rejects_suggestions() {
    let pool = db::test_pool().await;
    let fixture = fixture(&pool).await;
    set_suggestion(
        &pool,
        SetRecognitionSuggestionInput {
            capture_item_id: fixture.item_id.clone(),
            suggested_character_id: Some(fixture.other_character_id.clone()),
            confidence: Some(0.6),
            source: Some("vision".to_owned()),
        },
    )
    .await
    .expect("suggest");

    let accepted = review_suggestion(
        &pool,
        ReviewRecognitionInput {
            capture_item_id: fixture.item_id.clone(),
            decision: "accepted".to_owned(),
        },
    )
    .await
    .expect("accept");
    assert_eq!(accepted.review_status, "accepted");
    assert!(accepted.suggested_character_id.is_some());

    let rejected = review_suggestion(
        &pool,
        ReviewRecognitionInput {
            capture_item_id: fixture.item_id,
            decision: "rejected".to_owned(),
        },
    )
    .await
    .expect("reject");
    assert_eq!(rejected.review_status, "rejected");
    assert!(rejected.suggested_character_id.is_none());
    assert!(rejected.recognition_confidence.is_none());
    assert!(rejected.recognition_source.is_none());

    let error = review_suggestion(
        &pool,
        ReviewRecognitionInput {
            capture_item_id: fixture.second_item_id,
            decision: "accepted".to_owned(),
        },
    )
    .await
    .expect_err("no suggestion to review");
    assert!(matches!(error, AppError::Validation(_)));
}

#[tokio::test]
async fn accepting_a_suggestion_relabels_and_consumes_it() {
    let pool = db::test_pool().await;
    let fixture = fixture(&pool).await;
    set_item_feature(&pool, &fixture.item_id, "[1.0, 0.0, 0.0]").await;
    assert!(enroll_face_sample(&pool, &fixture.item_id)
        .await
        .expect("enroll current character"));
    seed_pending_suggestion(&pool, &fixture.item_id, &fixture.other_character_id).await;
    sqlx::query(
            "UPDATE capture_items SET status = 'completed', destination_path = 'old-archive.png' WHERE id = ?",
        )
        .bind(&fixture.item_id)
        .execute(&pool)
        .await
        .expect("mark completed");

    let corrected = accept_suggestion(&pool, &fixture.item_id)
        .await
        .expect("accept suggestion");
    assert_eq!(
        corrected.character_id.as_deref(),
        Some(fixture.other_character_id.as_str())
    );
    assert_eq!(corrected.status, "queued");
    assert_eq!(
        corrected.destination_path.as_deref(),
        Some("old-archive.png")
    );
    assert!(corrected.suggested_character_id.is_none());
    assert_eq!(corrected.review_status, "none");
    let samples: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM character_face_samples WHERE capture_item_id = ?")
            .bind(&fixture.item_id)
            .fetch_one(&pool)
            .await
            .expect("sample count");
    assert_eq!(
        samples, 0,
        "relabel must invalidate the old character sample"
    );
}

#[tokio::test]
async fn lists_character_items_newest_first() {
    let pool = db::test_pool().await;
    let fixture = fixture(&pool).await;
    // A scene capture that keeps the character id must NOT appear in the
    // person workbench grid.
    let session_id: String =
        sqlx::query_scalar("SELECT session_id FROM capture_items WHERE id = ?")
            .bind(&fixture.item_id)
            .fetch_one(&pool)
            .await
            .expect("session id");
    let scene_item =
        register_item(&pool, &fixture._workspace, &session_id, "scene.png", None).await;
    capture_service::label_capture(
        &pool,
        LabelCaptureInput {
            capture_item_id: scene_item.id,
            character_id: Some(fixture.character_id.clone()),
            classification: Some("scene".to_owned()),
        },
    )
    .await
    .expect("label scene");
    let items = list_character_items(
        &pool,
        ListCharacterItemsInput {
            project_id: fixture.project_id.clone(),
            character_id: fixture.character_id.clone(),
        },
    )
    .await
    .expect("list");
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].id, fixture.second_item_id);
    assert_eq!(items[1].id, fixture.item_id);

    let error = list_character_items(
        &pool,
        ListCharacterItemsInput {
            project_id: fixture.project_id,
            character_id: "missing".to_owned(),
        },
    )
    .await
    .expect_err("unknown character");
    assert!(matches!(error, AppError::NotFound(_)));
}

async fn set_item_feature(pool: &SqlitePool, capture_item_id: &str, feature: &str) {
    let dim = if feature
        .trim_matches('[')
        .trim_matches(']')
        .trim()
        .is_empty()
    {
        None
    } else {
        Some(
            feature
                .trim_start_matches('[')
                .trim_end_matches(']')
                .split(',')
                .filter(|part| !part.trim().is_empty())
                .count() as i64,
        )
    };
    let (model_id, model_version) = if dim.is_some() {
        (Some(FACE_MODEL_ID), Some(FACE_MODEL_VERSION))
    } else {
        (None, None)
    };
    sqlx::query(
        r#"
            INSERT INTO capture_faces (
                id, capture_item_id, face_index, is_primary, feature_json,
                feature_model_id, feature_model_version, feature_dim,
                face_sharpness, face_area_ratio
            )
            VALUES (?, ?, 0, 1, ?, ?, ?, ?, 50.0, 0.03)
            ON CONFLICT(capture_item_id, face_index) DO UPDATE SET
                feature_json = excluded.feature_json,
                feature_model_id = excluded.feature_model_id,
                feature_model_version = excluded.feature_model_version,
                feature_dim = excluded.feature_dim,
                face_sharpness = excluded.face_sharpness,
                face_area_ratio = excluded.face_area_ratio,
                updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
            "#,
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(capture_item_id)
    .bind(feature)
    .bind(model_id)
    .bind(model_version)
    .bind(dim)
    .execute(pool)
    .await
    .expect("set feature");
}

/// Seeds a face row whose model differs from the app's.
async fn set_item_feature_other_model(
    pool: &SqlitePool,
    capture_item_id: &str,
    feature: &str,
    model_id: &str,
    model_version: &str,
) {
    let dim = if feature
        .trim_matches('[')
        .trim_matches(']')
        .trim()
        .is_empty()
    {
        None
    } else {
        Some(
            feature
                .trim_start_matches('[')
                .trim_end_matches(']')
                .split(',')
                .filter(|part| !part.trim().is_empty())
                .count() as i64,
        )
    };
    sqlx::query(
        r#"
            INSERT INTO capture_faces (
                id, capture_item_id, face_index, is_primary, feature_json,
                feature_model_id, feature_model_version, feature_dim,
                face_sharpness, face_area_ratio
            )
            VALUES (?, ?, 0, 1, ?, ?, ?, ?, 50.0, 0.03)
            "#,
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(capture_item_id)
    .bind(feature)
    .bind(model_id)
    .bind(model_version)
    .bind(dim)
    .execute(pool)
    .await
    .expect("set feature");
}

async fn seed_pending_suggestion(pool: &SqlitePool, capture_item_id: &str, character_id: &str) {
    sqlx::query(
        r#"
            UPDATE capture_items
            SET suggested_character_id = ?, recognition_confidence = 0.87,
                recognition_source = 'face_bank', review_status = 'pending'
            WHERE id = ?
            "#,
    )
    .bind(character_id)
    .bind(capture_item_id)
    .execute(pool)
    .await
    .expect("seed suggestion");
}

async fn register_item(
    pool: &SqlitePool,
    workspace: &tempfile::TempDir,
    session_id: &str,
    name: &str,
    character_id: Option<&str>,
) -> CaptureItem {
    let source = workspace.path().join("source").join(name);
    tokio::fs::write(&source, name).await.expect("source");
    let item = capture_service::register_capture(
        pool,
        RegisterCaptureInput {
            session_id: session_id.to_owned(),
            source_path: capture_service::path_to_string(&source),
        },
    )
    .await
    .expect("register");
    if let Some(character_id) = character_id {
        capture_service::label_capture(
            pool,
            LabelCaptureInput {
                capture_item_id: item.id.clone(),
                character_id: Some(character_id.to_owned()),
                classification: Some("person".to_owned()),
            },
        )
        .await
        .expect("label");
    }
    item
}

#[tokio::test]
async fn enrolls_and_updates_face_samples_in_place() {
    let pool = db::test_pool().await;
    let fixture = fixture(&pool).await;
    set_item_feature(&pool, &fixture.item_id, "[1.0, 0.0, 0.0]").await;
    enroll_face_sample(&pool, &fixture.item_id)
        .await
        .expect("enroll");
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM character_face_samples WHERE character_id = ?")
            .bind(&fixture.character_id)
            .fetch_one(&pool)
            .await
            .expect("sample count");
    assert_eq!(count, 1);

    set_item_feature(&pool, &fixture.item_id, "[0.0, 1.0, 0.0]").await;
    enroll_face_sample(&pool, &fixture.item_id)
        .await
        .expect("re-enroll");
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM character_face_samples WHERE character_id = ?")
            .bind(&fixture.character_id)
            .fetch_one(&pool)
            .await
            .expect("sample count");
    assert_eq!(count, 1, "reprocessing must update in place");
    let stored: String = sqlx::query_scalar(
        "SELECT feature_json FROM character_face_samples WHERE character_id = ?",
    )
    .bind(&fixture.character_id)
    .fetch_one(&pool)
    .await
    .expect("stored feature");
    assert_eq!(stored, "[0.0, 1.0, 0.0]");

    // A successful no-face result invalidates the old projection.
    set_item_feature(&pool, &fixture.item_id, "[]").await;
    enroll_face_sample(&pool, &fixture.item_id)
        .await
        .expect("marker enroll");
    let remaining: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM character_face_samples WHERE character_id = ?")
            .bind(&fixture.character_id)
            .fetch_one(&pool)
            .await
            .expect("remaining samples");
    assert_eq!(remaining, 0, "no-face result must remove the stale sample");
}

#[tokio::test]
async fn suggests_best_match_above_threshold_within_project() {
    let pool = db::test_pool().await;
    let fixture = fixture(&pool).await;
    let session_id: String =
        sqlx::query_scalar("SELECT session_id FROM capture_items WHERE id = ?")
            .bind(&fixture.item_id)
            .fetch_one(&pool)
            .await
            .expect("session id");

    set_item_feature(&pool, &fixture.item_id, "[1.0, 0.0, 0.0]").await;
    enroll_face_sample(&pool, &fixture.item_id)
        .await
        .expect("enroll A");
    let bella_item = register_item(
        &pool,
        &fixture._workspace,
        &session_id,
        "bella.png",
        Some(&fixture.other_character_id),
    )
    .await;
    set_item_feature(&pool, &bella_item.id, "[0.0, 1.0, 0.0]").await;
    enroll_face_sample(&pool, &bella_item.id)
        .await
        .expect("enroll B");

    // A stranger sample in another project with an even closer feature:
    // project isolation must keep it out of the match.
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
    let stranger = character_service::create(
        &pool,
        CreateCharacterInput {
            project_id: other_project.id,
            name: "Stranger".to_owned(),
            aliases_json: None,
        },
    )
    .await
    .expect("stranger");
    sqlx::query(
        r#"
            INSERT INTO character_face_samples (
                id, character_id, capture_item_id, feature_json, status
            )
            VALUES ('stranger-sample', ?, ?, '[0.9,0.1,0.05]', 'active')
            "#,
    )
    .bind(&stranger.id)
    .bind(&fixture.item_id)
    .execute(&pool)
    .await
    .expect("stranger sample");

    let candidate = register_item(
        &pool,
        &fixture._workspace,
        &session_id,
        "candidate.png",
        None,
    )
    .await;
    set_item_feature(&pool, &candidate.id, "[0.9, 0.1, 0.0]").await;
    suggest_from_face_bank(&pool, &candidate.id)
        .await
        .expect("suggest");
    let suggested = capture_service::get_item(&pool, &candidate.id)
        .await
        .expect("candidate");
    assert_eq!(
        suggested.suggested_character_id.as_deref(),
        Some(fixture.character_id.as_str())
    );
    assert_eq!(suggested.recognition_source.as_deref(), Some("face_bank"));
    assert_eq!(suggested.review_status, "pending");
    let confidence = suggested.recognition_confidence.expect("confidence");
    assert!(
        (confidence - 0.994).abs() < 0.01,
        "unexpected confidence {confidence}"
    );
}

#[tokio::test]
async fn does_not_suggest_own_character_after_enrollment() {
    let pool = db::test_pool().await;
    let fixture = fixture(&pool).await;
    set_item_feature(&pool, &fixture.item_id, "[1.0, 0.0, 0.0]").await;
    enroll_face_sample(&pool, &fixture.item_id)
        .await
        .expect("enroll");
    // The item is already labeled with fixture.character_id and has just
    // enrolled its own sample; suggesting must not self-match at 1.0.
    suggest_from_face_bank(&pool, &fixture.item_id)
        .await
        .expect("suggest");
    let item = capture_service::get_item(&pool, &fixture.item_id)
        .await
        .expect("item");
    assert_eq!(
        item.suggested_character_id, None,
        "own character must not be suggested"
    );
    assert_eq!(item.review_status, "none");
}

#[tokio::test]
async fn does_not_suggest_when_face_bank_is_empty() {
    let pool = db::test_pool().await;
    let fixture = fixture(&pool).await;
    set_item_feature(&pool, &fixture.item_id, "[1.0, 0.0, 0.0]").await;

    suggest_from_face_bank(&pool, &fixture.item_id)
        .await
        .expect("suggest");

    let item = capture_service::get_item(&pool, &fixture.item_id)
        .await
        .expect("item");
    assert_eq!(
        item.suggested_character_id, None,
        "an empty bank must not write a suggestion"
    );
    assert_eq!(item.review_status, "none");
}

#[tokio::test]
async fn reject_and_enroll_clears_suggestion_and_registers_sample() {
    let pool = db::test_pool().await;
    let fixture = fixture(&pool).await;
    set_item_feature(&pool, &fixture.item_id, "[1.0, 0.0, 0.0]").await;
    sqlx::query(
        r#"
            UPDATE capture_items
            SET suggested_character_id = ?,
                recognition_confidence = 0.9,
                recognition_source = 'face_bank',
                review_status = 'pending'
            WHERE id = ?
            "#,
    )
    .bind(&fixture.other_character_id)
    .bind(&fixture.item_id)
    .execute(&pool)
    .await
    .expect("seed suggestion");

    let item = reject_and_enroll(&pool, &fixture.item_id)
        .await
        .expect("reject and enroll");
    assert_eq!(
        item.suggested_character_id, None,
        "suggestion must be cleared"
    );
    let sample: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM character_face_samples WHERE character_id = ? AND status = 'active'",
    )
    .bind(&fixture.character_id)
    .fetch_one(&pool)
    .await
    .expect("sample count");
    assert_eq!(sample, 1, "face must be enrolled to the labeled character");
}

#[tokio::test]
async fn batch_reject_and_enroll_clears_all_pending_for_character() {
    let pool = db::test_pool().await;
    let fixture = fixture(&pool).await;
    let session_id: String =
        sqlx::query_scalar("SELECT session_id FROM capture_items WHERE id = ?")
            .bind(&fixture.item_id)
            .fetch_one(&pool)
            .await
            .expect("session id");

    set_item_feature(&pool, &fixture.item_id, "[1.0, 0.0, 0.0]").await;
    sqlx::query(
        r#"
            UPDATE capture_items
            SET suggested_character_id = ?, recognition_confidence = 0.9,
                recognition_source = 'face_bank', review_status = 'pending'
            WHERE id = ?
            "#,
    )
    .bind(&fixture.other_character_id)
    .bind(&fixture.item_id)
    .execute(&pool)
    .await
    .expect("seed suggestion 1");
    let second = register_item(
        &pool,
        &fixture._workspace,
        &session_id,
        "third.png",
        Some(&fixture.character_id),
    )
    .await;
    set_item_feature(&pool, &second.id, "[0.0, 1.0, 0.0]").await;
    sqlx::query(
        r#"
            UPDATE capture_items
            SET suggested_character_id = ?, recognition_confidence = 0.8,
                recognition_source = 'face_bank', review_status = 'pending'
            WHERE id = ?
            "#,
    )
    .bind(&fixture.other_character_id)
    .bind(&second.id)
    .execute(&pool)
    .await
    .expect("seed suggestion 2");

    let project_id: String =
        sqlx::query_scalar("SELECT project_id FROM capture_sessions WHERE id = ?")
            .bind(&session_id)
            .fetch_one(&pool)
            .await
            .expect("project id");
    let count = batch_reject_and_enroll(&pool, &project_id, &fixture.character_id)
        .await
        .expect("batch");
    assert_eq!(count, 2);
    let remaining: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM capture_items WHERE character_id = ? AND suggested_character_id IS NOT NULL",
        )
        .bind(&fixture.character_id)
        .fetch_one(&pool)
        .await
        .expect("remaining");
    assert_eq!(remaining, 0, "all pending suggestions must be cleared");
    let samples: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM character_face_samples WHERE character_id = ? AND status = 'active'",
    )
    .bind(&fixture.character_id)
    .fetch_one(&pool)
    .await
    .expect("samples");
    assert_eq!(samples, 2, "both faces must be enrolled");
}

#[tokio::test]
async fn label_auto_resolves_matching_suggestion_keeps_contradiction() {
    let pool = db::test_pool().await;
    let fixture = fixture(&pool).await;
    let session_id: String =
        sqlx::query_scalar("SELECT session_id FROM capture_items WHERE id = ?")
            .bind(&fixture.item_id)
            .fetch_one(&pool)
            .await
            .expect("session id");

    async fn seed_suggestion(pool: &SqlitePool, capture_item_id: &str, character_id: &str) {
        sqlx::query(
            r#"
                UPDATE capture_items
                SET suggested_character_id = ?,
                    recognition_confidence = 0.87,
                    recognition_source = 'face_bank',
                    review_status = 'pending'
                WHERE id = ?
                "#,
        )
        .bind(character_id)
        .bind(capture_item_id)
        .execute(pool)
        .await
        .expect("seed suggestion");
    }

    // Labeling as the suggested character auto-accepts the suggestion.
    let matching = register_item(
        &pool,
        &fixture._workspace,
        &session_id,
        "matching.png",
        None,
    )
    .await;
    seed_suggestion(&pool, &matching.id, &fixture.character_id).await;
    let labeled = capture_service::label_capture(
        &pool,
        LabelCaptureInput {
            capture_item_id: matching.id.clone(),
            character_id: Some(fixture.character_id.clone()),
            classification: Some("person".to_owned()),
        },
    )
    .await
    .expect("label matching");
    assert_eq!(
        labeled.review_status, "accepted",
        "matching label must auto-accept the suggestion"
    );
    assert_eq!(
        labeled.suggested_character_id.as_deref(),
        Some(fixture.character_id.as_str()),
        "accepted suggestion stays visible in the workbench history"
    );

    // Labeling as a different character keeps the suggestion pending as a
    // mislabel hint.
    let contradicting = register_item(
        &pool,
        &fixture._workspace,
        &session_id,
        "contradicting.png",
        None,
    )
    .await;
    seed_suggestion(&pool, &contradicting.id, &fixture.character_id).await;
    let labeled_other = capture_service::label_capture(
        &pool,
        LabelCaptureInput {
            capture_item_id: contradicting.id.clone(),
            character_id: Some(fixture.other_character_id.clone()),
            classification: Some("person".to_owned()),
        },
    )
    .await
    .expect("label contradicting");
    assert_eq!(
        labeled_other.review_status, "pending",
        "contradicting label must keep the suggestion pending"
    );
    assert_eq!(
        labeled_other.suggested_character_id.as_deref(),
        Some(fixture.character_id.as_str())
    );
}

#[tokio::test]
async fn labeled_capture_can_still_be_suggested_as_another_character() {
    let pool = db::test_pool().await;
    let fixture = fixture(&pool).await;
    let session_id: String =
        sqlx::query_scalar("SELECT session_id FROM capture_items WHERE id = ?")
            .bind(&fixture.item_id)
            .fetch_one(&pool)
            .await
            .expect("session id");
    set_item_feature(&pool, &fixture.item_id, "[1.0, 0.0, 0.0]").await;
    enroll_face_sample(&pool, &fixture.item_id)
        .await
        .expect("enroll A");
    let bella_item = register_item(
        &pool,
        &fixture._workspace,
        &session_id,
        "bella.png",
        Some(&fixture.other_character_id),
    )
    .await;
    set_item_feature(&pool, &bella_item.id, "[0.0, 1.0, 0.0]").await;
    enroll_face_sample(&pool, &bella_item.id)
        .await
        .expect("enroll B");

    // A capture labeled as A whose face actually matches B's sample must
    // still be suggested as B — excluding A's own samples must not hide
    // real cross-character matches.
    let candidate = register_item(
        &pool,
        &fixture._workspace,
        &session_id,
        "mislabeled.png",
        Some(&fixture.character_id),
    )
    .await;
    set_item_feature(&pool, &candidate.id, "[0.0, 1.0, 0.0]").await;
    suggest_from_face_bank(&pool, &candidate.id)
        .await
        .expect("suggest");
    let suggested = capture_service::get_item(&pool, &candidate.id)
        .await
        .expect("candidate");
    assert_eq!(
        suggested.suggested_character_id.as_deref(),
        Some(fixture.other_character_id.as_str()),
        "cross-character match must still be suggested"
    );
    assert_eq!(suggested.review_status, "pending");
}

#[tokio::test]
async fn does_not_suggest_below_threshold() {
    let pool = db::test_pool().await;
    let fixture = fixture(&pool).await;
    let session_id: String =
        sqlx::query_scalar("SELECT session_id FROM capture_items WHERE id = ?")
            .bind(&fixture.item_id)
            .fetch_one(&pool)
            .await
            .expect("session id");
    set_item_feature(&pool, &fixture.item_id, "[1.0, 0.0, 0.0]").await;
    enroll_face_sample(&pool, &fixture.item_id)
        .await
        .expect("enroll A");

    let candidate = register_item(
        &pool,
        &fixture._workspace,
        &session_id,
        "candidate.png",
        None,
    )
    .await;
    set_item_feature(&pool, &candidate.id, "[0.4, 0.8, 0.0]").await;
    suggest_from_face_bank(&pool, &candidate.id)
        .await
        .expect("suggest");
    let suggested = capture_service::get_item(&pool, &candidate.id)
        .await
        .expect("candidate");
    assert!(suggested.suggested_character_id.is_none());
    assert_eq!(suggested.review_status, "none");
}

#[tokio::test]
async fn refresh_clears_a_stale_face_bank_suggestion_below_threshold() {
    let pool = db::test_pool().await;
    let fixture = fixture(&pool).await;
    let session_id: String =
        sqlx::query_scalar("SELECT session_id FROM capture_items WHERE id = ?")
            .bind(&fixture.item_id)
            .fetch_one(&pool)
            .await
            .expect("session id");
    set_item_feature(&pool, &fixture.item_id, "[1.0, 0.0, 0.0]").await;
    enroll_face_sample(&pool, &fixture.item_id)
        .await
        .expect("enroll baseline");

    let candidate = register_item(
        &pool,
        &fixture._workspace,
        &session_id,
        "stale-suggestion.png",
        None,
    )
    .await;
    set_item_feature(&pool, &candidate.id, "[0.4, 0.8, 0.0]").await;
    set_suggestion(
        &pool,
        SetRecognitionSuggestionInput {
            capture_item_id: candidate.id.clone(),
            suggested_character_id: Some(fixture.character_id),
            confidence: Some(0.9),
            source: Some("face_bank".to_owned()),
        },
    )
    .await
    .expect("seed stale suggestion");

    suggest_from_face_bank(&pool, &candidate.id)
        .await
        .expect("refresh suggestion");
    let refreshed = capture_service::get_item(&pool, &candidate.id)
        .await
        .expect("candidate");
    assert!(refreshed.suggested_character_id.is_none());
    assert!(refreshed.recognition_confidence.is_none());
    assert!(refreshed.recognition_source.is_none());
    assert_eq!(refreshed.review_status, "none");
}

#[tokio::test]
async fn sample_list_and_status_toggle_control_matching() {
    let pool = db::test_pool().await;
    let fixture = fixture(&pool).await;
    let session_id: String =
        sqlx::query_scalar("SELECT session_id FROM capture_items WHERE id = ?")
            .bind(&fixture.item_id)
            .fetch_one(&pool)
            .await
            .expect("session id");
    set_item_feature(&pool, &fixture.item_id, "[1.0, 0.0, 0.0]").await;
    enroll_face_sample(&pool, &fixture.item_id)
        .await
        .expect("enroll");

    let samples = list_character_face_samples(&pool, &fixture.character_id)
        .await
        .expect("list");
    assert_eq!(samples.len(), 1);
    assert_eq!(samples[0].status, "active");
    assert_eq!(samples[0].capture_item_id, fixture.item_id);

    let candidate = register_item(
        &pool,
        &fixture._workspace,
        &session_id,
        "candidate.png",
        None,
    )
    .await;
    set_item_feature(&pool, &candidate.id, "[0.9, 0.1, 0.0]").await;

    let revoked = set_face_sample_status(
        &pool,
        SetFaceSampleStatusInput {
            sample_id: samples[0].id.clone(),
            status: "revoked".to_owned(),
        },
    )
    .await
    .expect("revoke");
    assert_eq!(revoked.status, "revoked");

    suggest_from_face_bank(&pool, &candidate.id)
        .await
        .expect("suggest");
    let suggested = capture_service::get_item(&pool, &candidate.id)
        .await
        .expect("candidate");
    assert!(
        suggested.suggested_character_id.is_none(),
        "revoked samples must not match"
    );

    set_face_sample_status(
        &pool,
        SetFaceSampleStatusInput {
            sample_id: samples[0].id.clone(),
            status: "active".to_owned(),
        },
    )
    .await
    .expect("restore");
    suggest_from_face_bank(&pool, &candidate.id)
        .await
        .expect("suggest");
    let suggested = capture_service::get_item(&pool, &candidate.id)
        .await
        .expect("candidate");
    assert_eq!(
        suggested.suggested_character_id.as_deref(),
        Some(fixture.character_id.as_str())
    );

    let error = set_face_sample_status(
        &pool,
        SetFaceSampleStatusInput {
            sample_id: "missing".to_owned(),
            status: "active".to_owned(),
        },
    )
    .await
    .expect_err("missing sample");
    assert!(matches!(error, AppError::NotFound(_)));
    let error = set_face_sample_status(
        &pool,
        SetFaceSampleStatusInput {
            sample_id: samples[0].id.clone(),
            status: "banned".to_owned(),
        },
    )
    .await
    .expect_err("invalid status");
    assert!(matches!(error, AppError::Validation(_)));
}

#[tokio::test]
async fn verify_capture_identity_tiers_scores() {
    let pool = db::test_pool().await;
    let fixture = fixture(&pool).await;
    let session_id: String =
        sqlx::query_scalar("SELECT session_id FROM capture_items WHERE id = ?")
            .bind(&fixture.item_id)
            .fetch_one(&pool)
            .await
            .expect("session id");
    set_item_feature(&pool, &fixture.item_id, "[1.0, 0.0, 0.0]").await;
    enroll_face_sample(&pool, &fixture.item_id)
        .await
        .expect("enroll A");

    let candidate = register_item(
        &pool,
        &fixture._workspace,
        &session_id,
        "candidate.png",
        None,
    )
    .await;
    set_item_feature(&pool, &candidate.id, "[0.9, 0.1, 0.0]").await;
    let result = verify_capture_identity(
        &pool,
        VerifyCaptureIdentityInput {
            capture_item_id: candidate.id.clone(),
            character_id: fixture.character_id.clone(),
        },
    )
    .await
    .expect("verify");
    assert_eq!(result.level, "ok");
    assert!(result.score.unwrap() > 0.65);
    assert!(result.has_samples);
    assert!(result.has_feature);

    // A character without samples cannot verify.
    let result = verify_capture_identity(
        &pool,
        VerifyCaptureIdentityInput {
            capture_item_id: candidate.id.clone(),
            character_id: fixture.other_character_id.clone(),
        },
    )
    .await
    .expect("verify");
    assert_eq!(result.level, "unverified");
    assert!(!result.has_samples);

    // A capture without a feature cannot verify.
    let no_feature = register_item(
        &pool,
        &fixture._workspace,
        &session_id,
        "nofeature.png",
        None,
    )
    .await;
    let result = verify_capture_identity(
        &pool,
        VerifyCaptureIdentityInput {
            capture_item_id: no_feature.id,
            character_id: fixture.character_id,
        },
    )
    .await
    .expect("verify");
    assert_eq!(result.level, "unverified");
    assert!(!result.has_feature);
}

#[tokio::test]
async fn label_persists_verification_and_seeds_sample() {
    let pool = db::test_pool().await;
    let fixture = fixture(&pool).await;
    let session_id: String =
        sqlx::query_scalar("SELECT session_id FROM capture_items WHERE id = ?")
            .bind(&fixture.item_id)
            .fetch_one(&pool)
            .await
            .expect("session id");
    set_item_feature(&pool, &fixture.item_id, "[1.0, 0.0, 0.0]").await;
    enroll_face_sample(&pool, &fixture.item_id)
        .await
        .expect("enroll baseline");

    let fresh = register_item(&pool, &fixture._workspace, &session_id, "fresh.png", None).await;
    set_item_feature(&pool, &fresh.id, "[0.95, 0.05, 0.0]").await;
    capture_service::label_capture(
        &pool,
        LabelCaptureInput {
            capture_item_id: fresh.id.clone(),
            character_id: Some(fixture.character_id.clone()),
            classification: Some("person".to_owned()),
        },
    )
    .await
    .expect("label");

    let verification_status: String =
        sqlx::query_scalar("SELECT verification_status FROM capture_items WHERE id = ?")
            .bind(&fresh.id)
            .fetch_one(&pool)
            .await
            .expect("verification status");
    assert_eq!(verification_status, "ok");
    let flagged: i64 =
        sqlx::query_scalar("SELECT flagged FROM character_face_samples WHERE capture_item_id = ?")
            .bind(&fresh.id)
            .fetch_one(&pool)
            .await
            .expect("sample flagged");
    assert_eq!(flagged, 0);
    let sample_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM character_face_samples WHERE character_id = ? AND capture_item_id = ?",
        )
        .bind(&fixture.character_id)
        .bind(&fresh.id)
        .fetch_one(&pool)
        .await
        .expect("sample count");
    assert_eq!(sample_count, 1, "label time seeds the sample");
}

#[tokio::test]
async fn flagged_samples_are_excluded_and_restorable() {
    let pool = db::test_pool().await;
    let fixture = fixture(&pool).await;
    let session_id: String =
        sqlx::query_scalar("SELECT session_id FROM capture_items WHERE id = ?")
            .bind(&fixture.item_id)
            .fetch_one(&pool)
            .await
            .expect("session id");
    set_item_feature(&pool, &fixture.item_id, "[1.0, 0.0, 0.0]").await;
    // Simulate a low-confidence forced confirm at label time.
    sqlx::query("UPDATE capture_items SET verification_status = 'flagged' WHERE id = ?")
        .bind(&fixture.item_id)
        .execute(&pool)
        .await
        .expect("mark flagged");
    enroll_face_sample(&pool, &fixture.item_id)
        .await
        .expect("enroll flagged");
    let sample_id: String =
        sqlx::query_scalar("SELECT id FROM character_face_samples WHERE character_id = ?")
            .bind(&fixture.character_id)
            .fetch_one(&pool)
            .await
            .expect("sample id");
    let flagged: i64 =
        sqlx::query_scalar("SELECT flagged FROM character_face_samples WHERE id = ?")
            .bind(&sample_id)
            .fetch_one(&pool)
            .await
            .expect("flagged");
    assert_eq!(flagged, 1);

    let candidate = register_item(
        &pool,
        &fixture._workspace,
        &session_id,
        "candidate.png",
        None,
    )
    .await;
    set_item_feature(&pool, &candidate.id, "[0.9, 0.1, 0.0]").await;
    suggest_from_face_bank(&pool, &candidate.id)
        .await
        .expect("suggest");
    let suggested = capture_service::get_item(&pool, &candidate.id)
        .await
        .expect("candidate");
    assert!(
        suggested.suggested_character_id.is_none(),
        "flagged samples must not match"
    );

    let restored = set_face_sample_flagged(
        &pool,
        SetFaceSampleFlaggedInput {
            sample_id: sample_id.clone(),
            flagged: false,
        },
    )
    .await
    .expect("restore");
    assert_eq!(restored.flagged, 0);
    let verification_status: String =
        sqlx::query_scalar("SELECT verification_status FROM capture_items WHERE id = ?")
            .bind(&fixture.item_id)
            .fetch_one(&pool)
            .await
            .expect("status");
    assert_eq!(
        verification_status, "ok",
        "restoring marks the label verified"
    );

    suggest_from_face_bank(&pool, &candidate.id)
        .await
        .expect("suggest");
    let suggested = capture_service::get_item(&pool, &candidate.id)
        .await
        .expect("candidate");
    assert_eq!(
        suggested.suggested_character_id.as_deref(),
        Some(fixture.character_id.as_str()),
        "restored samples participate in matching again"
    );
}

#[tokio::test]
async fn manually_flagged_sample_stays_flagged_when_reenrolled() {
    let pool = db::test_pool().await;
    let fixture = fixture(&pool).await;
    set_item_feature(&pool, &fixture.item_id, "[1.0, 0.0, 0.0]").await;
    enroll_face_sample(&pool, &fixture.item_id)
        .await
        .expect("initial enrollment");
    let sample_id: String =
        sqlx::query_scalar("SELECT id FROM character_face_samples WHERE capture_item_id = ?")
            .bind(&fixture.item_id)
            .fetch_one(&pool)
            .await
            .expect("sample id");

    set_face_sample_flagged(
        &pool,
        SetFaceSampleFlaggedInput {
            sample_id: sample_id.clone(),
            flagged: true,
        },
    )
    .await
    .expect("mark suspicious");
    enroll_face_sample(&pool, &fixture.item_id)
        .await
        .expect("reenroll after feature refresh");

    let flagged: i64 =
        sqlx::query_scalar("SELECT flagged FROM character_face_samples WHERE id = ?")
            .bind(&sample_id)
            .fetch_one(&pool)
            .await
            .expect("flagged state");
    assert_eq!(
        flagged, 1,
        "feature refresh or Face Bank rebuild must preserve a manual suspicious flag"
    );

    set_face_sample_flagged(
        &pool,
        SetFaceSampleFlaggedInput {
            sample_id: sample_id.clone(),
            flagged: false,
        },
    )
    .await
    .expect("restore sample");
    enroll_face_sample(&pool, &fixture.item_id)
        .await
        .expect("reenroll restored sample");
    let restored: i64 =
        sqlx::query_scalar("SELECT flagged FROM character_face_samples WHERE id = ?")
            .bind(sample_id)
            .fetch_one(&pool)
            .await
            .expect("restored state");
    assert_eq!(restored, 0, "explicit restore must remain effective");
}

#[tokio::test]
async fn suggestion_ignores_samples_from_other_models() {
    let pool = db::test_pool().await;
    let fixture = fixture(&pool).await;
    let session_id: String =
        sqlx::query_scalar("SELECT session_id FROM capture_items WHERE id = ?")
            .bind(&fixture.item_id)
            .fetch_one(&pool)
            .await
            .expect("session id");

    // Ava's bank sample (SFace identity) scores 0.0 against the candidate;
    // Bella's sample uses a different model identity but would score 1.0
    // if the model filter were missing.
    set_item_feature(&pool, &fixture.item_id, "[0.0, 1.0, 0.0]").await;
    enroll_face_sample(&pool, &fixture.item_id)
        .await
        .expect("enroll Ava");
    let bella_item = register_item(
        &pool,
        &fixture._workspace,
        &session_id,
        "bella-arcface.png",
        Some(&fixture.other_character_id),
    )
    .await;
    set_item_feature_other_model(
        &pool,
        &bella_item.id,
        "[1.0, 0.0, 0.0]",
        "arcface-r50",
        "w600k",
    )
    .await;
    enroll_face_sample(&pool, &bella_item.id)
        .await
        .expect("enroll Bella");

    let candidate = register_item(
        &pool,
        &fixture._workspace,
        &session_id,
        "candidate-other-model.png",
        None,
    )
    .await;
    set_item_feature(&pool, &candidate.id, "[1.0, 0.0, 0.0]").await;
    suggest_from_face_bank(&pool, &candidate.id)
        .await
        .expect("suggest");
    let suggested = capture_service::get_item(&pool, &candidate.id)
        .await
        .expect("candidate");
    assert!(
        suggested.suggested_character_id.is_none(),
        "samples from another model must never be suggested"
    );
    assert_eq!(suggested.review_status, "none");
}

#[tokio::test]
async fn verification_ignores_samples_from_other_models() {
    let pool = db::test_pool().await;
    let fixture = fixture(&pool).await;
    let session_id: String =
        sqlx::query_scalar("SELECT session_id FROM capture_items WHERE id = ?")
            .bind(&fixture.item_id)
            .fetch_one(&pool)
            .await
            .expect("session id");

    // Ava's SFace sample scores low against the candidate; Bella's
    // ArcFace sample would score 1.0 ("looks more like Bella") but must
    // be excluded by the model filter.
    set_item_feature(&pool, &fixture.item_id, "[0.0, 1.0, 0.0]").await;
    enroll_face_sample(&pool, &fixture.item_id)
        .await
        .expect("enroll Ava");
    let bella_item = register_item(
        &pool,
        &fixture._workspace,
        &session_id,
        "bella-arcface.png",
        Some(&fixture.other_character_id),
    )
    .await;
    set_item_feature_other_model(
        &pool,
        &bella_item.id,
        "[1.0, 0.0, 0.0]",
        "arcface-r50",
        "w600k",
    )
    .await;
    enroll_face_sample(&pool, &bella_item.id)
        .await
        .expect("enroll Bella");

    let candidate = register_item(
        &pool,
        &fixture._workspace,
        &session_id,
        "candidate-verify.png",
        None,
    )
    .await;
    set_item_feature(&pool, &candidate.id, "[1.0, 0.0, 0.0]").await;
    let result = verify_capture_identity(
        &pool,
        VerifyCaptureIdentityInput {
            capture_item_id: candidate.id,
            character_id: fixture.character_id,
        },
    )
    .await
    .expect("verify");
    assert!(
        result.best_other_score.is_none(),
        "cross-character samples from another model must not count"
    );
    assert_eq!(result.best_other_character_id, None);
}

#[tokio::test]
async fn rewriting_the_feature_clears_stale_suggestion() {
    let pool = db::test_pool().await;
    let fixture = fixture(&pool).await;
    set_item_feature(&pool, &fixture.item_id, "[1.0, 0.0, 0.0]").await;
    enroll_face_sample(&pool, &fixture.item_id)
        .await
        .expect("enroll");
    seed_pending_suggestion(&pool, &fixture.item_id, &fixture.character_id).await;

    // A rebuild rewrites the feature through the service writer; the
    // suggestion produced by the previous feature/model must not survive.
    capture_service::store_face_feature(
        &pool,
        &fixture.item_id,
        capture_service::FaceFeatureWrite {
            feature_json: Some("[0.0, 1.0, 0.0]".to_owned()),
            face_box_json: None,
            model_id: Some(FACE_MODEL_ID.to_owned()),
            model_version: Some(FACE_MODEL_VERSION.to_owned()),
            face_count: None,
            face_sharpness: None,
            face_area_ratio: None,
        },
    )
    .await
    .expect("rewrite feature");
    let item = capture_service::get_item(&pool, &fixture.item_id)
        .await
        .expect("item");
    assert!(item.suggested_character_id.is_none());
    assert!(item.recognition_confidence.is_none());
    assert!(item.recognition_source.is_none());
    assert_eq!(item.review_status, "none");
}

#[tokio::test]
async fn suggestion_margin_suppresses_close_calls() {
    let pool = db::test_pool().await;
    let fixture = fixture(&pool).await;
    let session_id: String =
        sqlx::query_scalar("SELECT session_id FROM capture_items WHERE id = ?")
            .bind(&fixture.item_id)
            .fetch_one(&pool)
            .await
            .expect("session id");

    // Ava and Bella's bank samples are near-identical; a query between
    // them clears the threshold but not the configured margin (0.05 for
    // opencv-sface), so no suggestion is written.
    set_item_feature(&pool, &fixture.item_id, "[1.0, 0.0, 0.0]").await;
    enroll_face_sample(&pool, &fixture.item_id)
        .await
        .expect("enroll Ava");
    let bella_item = register_item(
        &pool,
        &fixture._workspace,
        &session_id,
        "bella-close.png",
        Some(&fixture.other_character_id),
    )
    .await;
    set_item_feature(&pool, &bella_item.id, "[0.99, 0.01, 0.0]").await;
    enroll_face_sample(&pool, &bella_item.id)
        .await
        .expect("enroll Bella");

    let candidate = register_item(
        &pool,
        &fixture._workspace,
        &session_id,
        "close-call.png",
        None,
    )
    .await;
    set_item_feature(&pool, &candidate.id, "[0.995, 0.005, 0.0]").await;
    suggest_from_face_bank(&pool, &candidate.id)
        .await
        .expect("suggest");
    let suggested = capture_service::get_item(&pool, &candidate.id)
        .await
        .expect("candidate");
    assert!(
        suggested.suggested_character_id.is_none(),
        "a too-close runner-up must suppress the suggestion"
    );
    assert_eq!(suggested.review_status, "none");
}

#[tokio::test]
async fn low_quality_faces_are_not_enrolled_and_warn() {
    let pool = db::test_pool().await;
    let fixture = fixture(&pool).await;
    let session_id: String =
        sqlx::query_scalar("SELECT session_id FROM capture_items WHERE id = ?")
            .bind(&fixture.item_id)
            .fetch_one(&pool)
            .await
            .expect("session id");
    let candidate = register_item(
        &pool,
        &fixture._workspace,
        &session_id,
        "printed-card-face.png",
        Some(&fixture.character_id),
    )
    .await;
    // A printed/photo face scores near-zero sharpness (e.g. 1.4); the
    // default gate (min 3.0) must refuse to enroll it and remove a sample
    // that was valid before the quality metric changed.
    set_item_feature(&pool, &candidate.id, "[1.0, 0.0, 0.0]").await;
    assert!(enroll_face_sample(&pool, &candidate.id)
        .await
        .expect("initial good enrollment"));
    sqlx::query(
        r#"
            UPDATE capture_faces
            SET face_sharpness = 0.5
            WHERE capture_item_id = ? AND is_primary = 1
            "#,
    )
    .bind(&candidate.id)
    .execute(&pool)
    .await
    .expect("degrade sharpness");
    enroll_face_sample(&pool, &candidate.id)
        .await
        .expect("enroll attempt");
    let samples: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM character_face_samples WHERE capture_item_id = ?")
            .bind(&candidate.id)
            .fetch_one(&pool)
            .await
            .expect("samples");
    assert_eq!(samples, 0, "low-quality faces must not be enrolled");
    let warnings: String =
        sqlx::query_scalar("SELECT processing_warnings_json FROM capture_items WHERE id = ?")
            .bind(&candidate.id)
            .fetch_one(&pool)
            .await
            .expect("warnings");
    assert!(
        warnings.contains("sample_not_enrolled_low_quality"),
        "the skipped enrollment must be explained: {warnings}"
    );

    // A sharp face on the same capture enrolls normally.
    sqlx::query(
        r#"
            UPDATE capture_faces
            SET face_sharpness = 50.0
            WHERE capture_item_id = ? AND is_primary = 1
            "#,
    )
    .bind(&candidate.id)
    .execute(&pool)
    .await
    .expect("restore sharpness");
    enroll_face_sample(&pool, &candidate.id)
        .await
        .expect("enroll again");
    let samples: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM character_face_samples WHERE capture_item_id = ?")
            .bind(&candidate.id)
            .fetch_one(&pool)
            .await
            .expect("samples");
    assert_eq!(samples, 1, "sharp faces must be enrolled");
}

#[tokio::test]
async fn no_face_result_removes_an_existing_sample() {
    let pool = db::test_pool().await;
    let fixture = fixture(&pool).await;
    set_item_feature(&pool, &fixture.item_id, "[1.0, 0.0, 0.0]").await;
    assert!(enroll_face_sample(&pool, &fixture.item_id)
        .await
        .expect("initial enrollment"));

    set_item_feature(&pool, &fixture.item_id, "[]").await;
    assert!(!enroll_face_sample(&pool, &fixture.item_id)
        .await
        .expect("no-face reconciliation"));
    let samples: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM character_face_samples WHERE capture_item_id = ?")
            .bind(&fixture.item_id)
            .fetch_one(&pool)
            .await
            .expect("sample count");
    assert_eq!(samples, 0);
}

#[tokio::test]
async fn model_status_reports_every_group_and_mixed_bank_as_incompatible() {
    let pool = db::test_pool().await;
    let fixture = fixture(&pool).await;
    for item_id in [&fixture.item_id, &fixture.second_item_id] {
        set_item_feature(&pool, item_id, "[1.0, 0.0, 0.0]").await;
        assert!(enroll_face_sample(&pool, item_id)
            .await
            .expect("enroll sample"));
    }
    sqlx::query(
        r#"
            UPDATE character_face_samples
            SET model_id = 'arcface-r50', model_version = 'w600k-r50', embedding_dim = 512
            WHERE capture_item_id = ?
            "#,
    )
    .bind(&fixture.second_item_id)
    .execute(&pool)
    .await
    .expect("make mixed bank");

    let status = face_bank_model_status(&pool, &fixture.project_id)
        .await
        .expect("model status");
    assert_eq!(status.sample_count, 2);
    assert_eq!(status.incompatible_sample_count, 1);
    assert!(!status.compatible);
    assert_eq!(status.model_counts.len(), 2);
    assert_eq!(
        status
            .model_counts
            .iter()
            .filter(|group| group.compatible)
            .map(|group| group.sample_count)
            .sum::<i64>(),
        1
    );
}

#[tokio::test]
async fn configured_model_version_uses_the_same_content_fingerprint_as_python() {
    let temporary = tempdir().expect("temporary model directory");
    let model_path = temporary.path().join("sface.onnx");
    std::fs::write(&model_path, b"first-model").expect("write model");

    let first = model_version_with_fingerprint(FACE_MODEL_VERSION, model_path.to_str()).await;
    std::fs::write(&model_path, b"second-model").expect("replace model");
    let second = model_version_with_fingerprint(FACE_MODEL_VERSION, model_path.to_str()).await;

    assert!(first.starts_with("2021dec+sha256:"));
    assert_ne!(first, second);
}

#[tokio::test]
async fn tiny_background_faces_are_not_enrolled_and_warn() {
    let pool = db::test_pool().await;
    let fixture = fixture(&pool).await;
    let session_id: String =
        sqlx::query_scalar("SELECT session_id FROM capture_items WHERE id = ?")
            .bind(&fixture.item_id)
            .fetch_one(&pool)
            .await
            .expect("session id");
    let candidate = register_item(
        &pool,
        &fixture._workspace,
        &session_id,
        "background-face.png",
        Some(&fixture.character_id),
    )
    .await;
    set_item_feature(&pool, &candidate.id, "[1.0, 0.0, 0.0]").await;
    // A background face occupies a tiny area share (default gate 0.5%).
    sqlx::query(
        r#"
            UPDATE capture_faces
            SET face_area_ratio = 0.0008
            WHERE capture_item_id = ? AND is_primary = 1
            "#,
    )
    .bind(&candidate.id)
    .execute(&pool)
    .await
    .expect("shrink face");
    enroll_face_sample(&pool, &candidate.id)
        .await
        .expect("enroll attempt");
    let samples: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM character_face_samples WHERE capture_item_id = ?")
            .bind(&candidate.id)
            .fetch_one(&pool)
            .await
            .expect("samples");
    assert_eq!(samples, 0, "tiny background faces must not be enrolled");
    let warnings: String =
        sqlx::query_scalar("SELECT processing_warnings_json FROM capture_items WHERE id = ?")
            .bind(&candidate.id)
            .fetch_one(&pool)
            .await
            .expect("warnings");
    assert!(
        warnings.contains("area ratio") && warnings.contains("sample_not_enrolled_low_quality"),
        "the skipped enrollment must explain the area check: {warnings}"
    );
}

#[tokio::test]
async fn migration_0013_cleans_polluted_face_data_and_backfills_primary_rows() {
    use sqlx::sqlite::SqliteConnectOptions;
    use sqlx::sqlite::SqlitePoolOptions;

    // Rebuild a pre-0013 schema by applying migrations 0001..0012 from
    // disk, then seed polluted face data and run 0013 on top of it.
    let options = SqliteConnectOptions::new()
        .filename(":memory:")
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("pool");
    let mut files: Vec<String> = std::fs::read_dir("./migrations")
        .expect("migrations directory")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".sql"))
        .collect();
    files.sort();
    {
        let mut conn = pool.acquire().await.expect("connection");
        for file in &files {
            if file.as_str() >= "0013_capture_faces.sql" {
                continue;
            }
            let sql =
                std::fs::read_to_string(format!("./migrations/{file}")).expect("read migration");
            sqlx::raw_sql(&sql)
                .execute(&mut *conn)
                .await
                .expect("apply migration");
        }
        // 0014 only adds capture_items.face_count, which the service
        // layer requires for seeding; it is independent of 0013's face
        // cleanup assertions, so it is applied before the seed.
        let migration_0014 =
            std::fs::read_to_string("./migrations/0014_face_count.sql").expect("read 0014");
        sqlx::raw_sql(&migration_0014)
            .execute(&mut *conn)
            .await
            .expect("apply 0014");
        // 0018 only adds capture_items.recognition_deferred, which the
        // service layer now requires when registering captures; it is
        // likewise independent of 0013's cleanup assertions.
        let migration_0018 =
            std::fs::read_to_string("./migrations/0018_deferred_import_recognition.sql")
                .expect("read 0018");
        sqlx::raw_sql(&migration_0018)
            .execute(&mut *conn)
            .await
            .expect("apply 0018");
        // 0019 only adds projects.cover_capture_item_id, which the project
        // fixture now requires; it is likewise independent of 0013's
        // cleanup assertions.
        let migration_0019 =
            std::fs::read_to_string("./migrations/0019_project_cover.sql").expect("read 0019");
        sqlx::raw_sql(&migration_0019)
            .execute(&mut *conn)
            .await
            .expect("apply 0019");
    }

    // Seed without label_capture: label-time verification would already
    // query capture_faces, which does not exist in the pre-0013 schema.
    let fixture = test_support::project_with_directories(&pool, "Migration")
        .await
        .expect("project");
    let session = test_support::start_session(&pool, &fixture.project_id)
        .await
        .expect("session");
    let character = character_service::create(
        &pool,
        CreateCharacterInput {
            project_id: fixture.project_id.clone(),
            name: "Ava".to_owned(),
            aliases_json: None,
        },
    )
    .await
    .expect("character");
    let source = fixture.source_directory.join("shot.png");
    tokio::fs::write(&source, b"image").await.expect("source");
    let item = capture_service::register_capture(
        &pool,
        RegisterCaptureInput {
            session_id: session.id.clone(),
            source_path: capture_service::path_to_string(&source),
        },
    )
    .await
    .expect("register");
    let character_id = character.id;
    let item_id = item.id;
    sqlx::query(
        r#"
            UPDATE capture_items
            SET face_feature_json = '[1.0, 2.0, 3.0]',
                face_box_json = '{"x":1,"y":1,"width":10,"height":10,"confidence":0.9}',
                suggested_character_id = ?,
                recognition_confidence = 0.9,
                recognition_source = 'face_bank',
                review_status = 'pending',
                verification_status = 'ok',
                verification_score = 0.8
            WHERE id = ?
            "#,
    )
    .bind(&character_id)
    .bind(&item_id)
    .execute(&pool)
    .await
    .expect("pollute item");
    sqlx::query(
        r#"
            INSERT INTO character_face_samples (id, character_id, capture_item_id, feature_json)
            VALUES (?, ?, ?, '[9.0, 9.0]')
            "#,
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(&character_id)
    .bind(&item_id)
    .execute(&pool)
    .await
    .expect("polluted sample");

    let migration_0013 =
        std::fs::read_to_string("./migrations/0013_capture_faces.sql").expect("read 0013");
    {
        let mut conn = pool.acquire().await.expect("connection");
        sqlx::raw_sql(&migration_0013)
            .execute(&mut *conn)
            .await
            .expect("apply 0013");
    }

    // Structural backfill: exactly one primary face row, box preserved,
    // feature and model left NULL (polluted embeddings never migrate).
    let (count, is_primary, face_index, box_json, feature_json): (
        i64,
        i64,
        i64,
        Option<String>,
        Option<String>,
    ) = sqlx::query_as(
        r#"
            SELECT COUNT(*), MAX(is_primary), MAX(face_index), MAX(box_json), MAX(feature_json)
            FROM capture_faces
            WHERE capture_item_id = ?
            "#,
    )
    .bind(&item_id)
    .fetch_one(&pool)
    .await
    .expect("backfilled faces");
    assert_eq!(count, 1, "one primary face row per boxed capture");
    assert_eq!(is_primary, 1);
    assert_eq!(face_index, 0);
    assert!(
        box_json
            .as_deref()
            .is_some_and(|value| value.contains("\"width\":10")),
        "the recorded box must be preserved"
    );
    assert!(feature_json.is_none(), "polluted features must not migrate");

    let samples: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM character_face_samples")
        .fetch_one(&pool)
        .await
        .expect("sample count");
    assert_eq!(samples, 0, "polluted face samples must be wiped");

    let (legacy, suggestion, review, verification): (
        Option<String>,
        Option<String>,
        String,
        String,
    ) = sqlx::query_as(
        r#"
            SELECT face_feature_json, suggested_character_id, review_status,
                   verification_status
            FROM capture_items
            WHERE id = ?
            "#,
    )
    .bind(&item_id)
    .fetch_one(&pool)
    .await
    .expect("cleaned item");
    assert!(legacy.is_none(), "legacy feature column must be cleared");
    assert!(suggestion.is_none(), "stale suggestion must be cleared");
    assert_eq!(review, "none");
    assert_eq!(verification, "unverified");

    // The database-level invariant rejects a second primary face.
    let second_primary = sqlx::query(
        r#"
            INSERT INTO capture_faces (id, capture_item_id, face_index, is_primary)
            VALUES (?, ?, 1, 1)
            "#,
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(&item_id)
    .execute(&pool)
    .await;
    assert!(
        second_primary.is_err(),
        "a capture must have at most one primary face"
    );
}

#[tokio::test]
async fn rebuild_face_bank_rejects_unknown_project() {
    let pool = db::test_pool().await;
    let error = rebuild_face_bank(&pool, "missing-project", |_, _| {})
        .await
        .expect_err("unknown project");
    assert!(matches!(error, AppError::NotFound(_)));
}

#[tokio::test]
async fn rebuild_face_bank_requires_configured_engine() {
    let pool = db::test_pool().await;
    let fixture = fixture(&pool).await;
    let error = rebuild_face_bank(&pool, &fixture.project_id, |_, _| {})
        .await
        .expect_err("engine missing");
    assert!(matches!(error, AppError::Vision(_)));
}

#[tokio::test]
async fn refresh_face_feature_rejects_scene_and_private_without_engine() {
    let pool = db::test_pool().await;
    let fixture = fixture(&pool).await;
    let session_id: String =
        sqlx::query_scalar("SELECT session_id FROM capture_items WHERE id = ?")
            .bind(&fixture.item_id)
            .fetch_one(&pool)
            .await
            .expect("session id");
    let scene = register_item(&pool, &fixture._workspace, &session_id, "scene.png", None).await;
    capture_service::relabel_capture_item(
        &pool,
        RelabelCaptureInput {
            capture_item_id: scene.id.clone(),
            character_id: None,
            classification: Some("scene".to_owned()),
        },
    )
    .await
    .expect("relabel to scene");
    let private = register_item(&pool, &fixture._workspace, &session_id, "private.png", None).await;
    capture_service::relabel_capture_item(
        &pool,
        RelabelCaptureInput {
            capture_item_id: private.id.clone(),
            character_id: None,
            classification: Some("private".to_owned()),
        },
    )
    .await
    .expect("relabel to private");

    for capture_item_id in [&scene.id, &private.id] {
        let error = refresh_capture_face_feature(&pool, capture_item_id, |_, _| {})
            .await
            .expect_err("scene/private must be rejected before any engine work");
        assert!(
            matches!(error, AppError::Validation(_)),
            "expected validation error, got {error:?}"
        );
    }
}

#[tokio::test]
async fn rebuild_face_bank_empty_project_reports_zero() {
    use crate::models::vision::{UpdateVisionSettingsInput, VisionSettings};

    let pool = db::test_pool().await;
    let fixture = test_support::project_with_directories(&pool, "Rebuild")
        .await
        .expect("project");
    let workspace = tempdir().expect("tempdir");
    let python = workspace.path().join("python.exe");
    let module_root = workspace.path().join("pymod");
    let yunet = workspace.path().join("yunet.onnx");
    let sface = workspace.path().join("sface.onnx");
    tokio::fs::create_dir_all(&module_root)
        .await
        .expect("module root");
    tokio::fs::write(&python, b"").await.expect("python");
    tokio::fs::write(&yunet, b"").await.expect("yunet");
    tokio::fs::write(&sface, b"").await.expect("sface");
    vision_settings_service::update(
        &pool,
        UpdateVisionSettingsInput {
            settings: VisionSettings {
                python_executable_path: Some(capture_service::path_to_string(&python)),
                python_module_root: Some(capture_service::path_to_string(&module_root)),
                yunet_model_path: Some(capture_service::path_to_string(&yunet)),
                sface_model_path: Some(capture_service::path_to_string(&sface)),
                recognizer: None,
                arcface_model_path: None,
                font_path: None,
            },
        },
    )
    .await
    .expect("vision settings");

    let mut progress: Vec<(u32, u32)> = Vec::new();
    let summary = rebuild_face_bank(&pool, &fixture.project_id, |processed, total| {
        progress.push((processed, total));
    })
    .await
    .expect("rebuild");
    assert_eq!(summary.total, 0);
    assert_eq!(summary.rebuilt, 0);
    assert_eq!(summary.no_face, 0);
    assert_eq!(summary.skipped_missing_source, 0);
    assert_eq!(summary.failed, 0);
    assert_eq!(summary.suggestions_refreshed, 0);
    assert!(progress.is_empty(), "no items, no progress ticks");
}
