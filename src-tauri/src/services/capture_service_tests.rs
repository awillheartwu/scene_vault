use super::*;
use crate::{
    db,
    models::{
        capture::{
            CaptureItemListResponse, CaptureItemPage, ImportDirectoryCapturesInput,
            RelabelCaptureInput, StartCaptureSessionInput,
        },
        character::CreateCharacterInput,
        project::{
            AddProjectSourceDirectoryInput, CreateProjectInput, SetProjectCoverInput,
            SetProjectDestinationInput,
        },
        recognition::SetRecognitionSuggestionInput,
    },
    services::{character_service, project_service, recognition_service, test_support},
};
use tempfile::tempdir;

async fn project(pool: &SqlitePool) -> crate::models::project::Project {
    project_service::create(
        pool,
        CreateProjectInput {
            name: "Love and Jealousy".to_owned(),
            description: None,
            cover_asset_id: None,
        },
    )
    .await
    .expect("create project")
}

async fn session_with_source(
    pool: &SqlitePool,
    project_id: &str,
) -> (tempfile::TempDir, CaptureSession, PathBuf) {
    let workspace = tempdir().expect("tempdir");
    let source = workspace.path().join("source");
    let destination = workspace.path().join("archive");
    tokio::fs::create_dir(&source)
        .await
        .expect("source directory");
    tokio::fs::create_dir(&destination)
        .await
        .expect("destination directory");
    project_service::set_destination_directory(
        pool,
        crate::models::project::SetProjectDestinationInput {
            project_id: project_id.to_owned(),
            directory: path_to_string(&destination),
        },
    )
    .await
    .expect("set destination");
    project_service::add_source_directory(
        pool,
        crate::models::project::AddProjectSourceDirectoryInput {
            project_id: project_id.to_owned(),
            directory: path_to_string(&source),
        },
    )
    .await
    .expect("add source directory");
    let session = start_session(
        pool,
        StartCaptureSessionInput {
            project_id: project_id.to_owned(),
        },
    )
    .await
    .expect("start capture session")
    .session;
    (workspace, session, source)
}

/// Inserts a capture row directly so pagination tests can build deep lists
/// without the stability-delay cost of the full registration path.
async fn insert_item(
    pool: &SqlitePool,
    session_id: &str,
    project_id: &str,
    classification: &str,
    name: &str,
) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        r#"
        INSERT INTO capture_items (
            id, project_id, session_id, source_path, classification, status
        )
        VALUES (?, ?, ?, ?, ?, 'awaiting_label')
        "#,
    )
    .bind(&id)
    .bind(project_id)
    .bind(session_id)
    .bind(name)
    .bind(classification)
    .execute(pool)
    .await
    .expect("insert capture item");
    id
}

#[test]
fn capture_item_list_response_keeps_legacy_array_and_paged_object_shapes() {
    let legacy = CaptureItemListResponse::Legacy(Vec::new());
    assert_eq!(
        serde_json::to_string(&legacy).expect("serialize legacy"),
        "[]"
    );

    let paged = CaptureItemListResponse::Paged(CaptureItemPage {
        items: Vec::new(),
        total: 7,
        page: 2,
        page_size: 20,
    });
    assert_eq!(
        serde_json::to_string(&paged).expect("serialize paged"),
        r#"{"items":[],"total":7,"page":2,"pageSize":20}"#
    );
}

#[tokio::test]
async fn start_session_auto_ends_active_sessions_of_other_projects() {
    let pool = db::test_pool().await;
    let project_a = project(&pool).await;
    let (workspace_a, _session_a, _source_a) = session_with_source(&pool, &project_a.id).await;
    let project_b = project(&pool).await;
    let workspace_b = tempdir().expect("tempdir");
    let source_b = workspace_b.path().join("source");
    let destination_b = workspace_b.path().join("archive");
    tokio::fs::create_dir(&source_b)
        .await
        .expect("source directory");
    tokio::fs::create_dir(&destination_b)
        .await
        .expect("destination directory");
    project_service::set_destination_directory(
        &pool,
        crate::models::project::SetProjectDestinationInput {
            project_id: project_b.id.clone(),
            directory: path_to_string(&destination_b),
        },
    )
    .await
    .expect("set destination");
    project_service::add_source_directory(
        &pool,
        crate::models::project::AddProjectSourceDirectoryInput {
            project_id: project_b.id.clone(),
            directory: path_to_string(&source_b),
        },
    )
    .await
    .expect("add source directory");
    let session_b = start_session(
        &pool,
        StartCaptureSessionInput {
            project_id: project_b.id.clone(),
        },
    )
    .await
    .expect("start session b");

    assert_eq!(
        session_b.stopped_projects,
        vec![project_a.name],
        "starting a session must report the project it auto-ended"
    );
    let status_a: String = sqlx::query_scalar("SELECT status FROM capture_sessions WHERE id = ?")
        .bind(&_session_a.id)
        .fetch_one(&pool)
        .await
        .expect("session a status");
    assert_eq!(
        status_a, "cancelled",
        "project A's session must be cancelled when project B starts"
    );
    let _ = (workspace_a, workspace_b, _source_a);
}

#[tokio::test]
async fn history_hides_private_captures_by_default() {
    let pool = db::test_pool().await;
    let project = project(&pool).await;
    let (_workspace, session, source) = session_with_source(&pool, &project.id).await;
    let scene = source.join("scene.png");
    let private = source.join("private.png");
    tokio::fs::write(&scene, b"scene").await.expect("scene");
    tokio::fs::write(&private, b"private")
        .await
        .expect("private");
    let scene_item = register_capture(
        &pool,
        RegisterCaptureInput {
            session_id: session.id.clone(),
            source_path: path_to_string(&scene),
        },
    )
    .await
    .expect("register scene");
    let private_item = register_capture(
        &pool,
        RegisterCaptureInput {
            session_id: session.id.clone(),
            source_path: path_to_string(&private),
        },
    )
    .await
    .expect("register private");
    label_capture(
        &pool,
        LabelCaptureInput {
            capture_item_id: scene_item.id,
            character_id: None,
            classification: Some("scene".to_owned()),
        },
    )
    .await
    .expect("label scene");
    label_capture(
        &pool,
        LabelCaptureInput {
            capture_item_id: private_item.id,
            character_id: None,
            classification: Some("private".to_owned()),
        },
    )
    .await
    .expect("label private");

    let hidden = list_history(
        &pool,
        ListCaptureHistoryInput {
            project_id: project.id.clone(),
            session_id: None,
            character_id: None,
            status: None,
            limit: None,
            offset: None,
            include_private: None,
        },
    )
    .await
    .expect("list history");
    assert_eq!(hidden.total, 1);
    assert_eq!(hidden.entries.len(), 1);
    assert_eq!(hidden.entries[0].classification, "scene");

    let visible = list_history(
        &pool,
        ListCaptureHistoryInput {
            project_id: project.id,
            session_id: None,
            character_id: None,
            status: None,
            limit: None,
            offset: None,
            include_private: Some(true),
        },
    )
    .await
    .expect("list history with private");
    assert_eq!(visible.total, 2);
    assert_eq!(visible.entries.len(), 2);
}

#[tokio::test]
async fn history_paginates_with_offset_and_reports_total() {
    let pool = db::test_pool().await;
    let project = project(&pool).await;
    let (_workspace, session, source) = session_with_source(&pool, &project.id).await;
    for name in ["001.png", "002.png", "003.png"] {
        tokio::fs::write(source.join(name), name)
            .await
            .expect("write");
        register_capture(
            &pool,
            RegisterCaptureInput {
                session_id: session.id.clone(),
                source_path: path_to_string(&source.join(name)),
            },
        )
        .await
        .expect("register");
    }

    let page1 = list_history(
        &pool,
        ListCaptureHistoryInput {
            project_id: project.id.clone(),
            session_id: None,
            character_id: None,
            status: None,
            limit: Some(2),
            offset: Some(0),
            include_private: None,
        },
    )
    .await
    .expect("page 1");
    assert_eq!(page1.total, 3);
    assert_eq!(page1.entries.len(), 2);

    let page2 = list_history(
        &pool,
        ListCaptureHistoryInput {
            project_id: project.id,
            session_id: None,
            character_id: None,
            status: None,
            limit: Some(2),
            offset: Some(2),
            include_private: None,
        },
    )
    .await
    .expect("page 2");
    assert_eq!(page2.total, 3);
    assert_eq!(page2.entries.len(), 1);
}

#[tokio::test]
async fn runs_session_label_and_processing_state_transitions() {
    let pool = db::test_pool().await;
    let project = project(&pool).await;
    let (workspace, session, source) = session_with_source(&pool, &project.id).await;
    let output = workspace.path().join("output");
    tokio::fs::create_dir_all(&output)
        .await
        .expect("output directory");
    let screenshot = source.join("001.png");
    tokio::fs::write(&screenshot, b"source")
        .await
        .expect("source screenshot");
    let item = register_capture(
        &pool,
        RegisterCaptureInput {
            session_id: session.id.clone(),
            source_path: path_to_string(&screenshot),
        },
    )
    .await
    .expect("register capture");
    assert_eq!(item.status, "awaiting_label");

    let character = character_service::create(
        &pool,
        CreateCharacterInput {
            project_id: project.id.clone(),
            name: "Aurora".to_owned(),
            aliases_json: None,
        },
    )
    .await
    .expect("create character");
    let labelled = label_capture(
        &pool,
        LabelCaptureInput {
            capture_item_id: item.id,
            character_id: Some(character.id.clone()),
            classification: Some("person".to_owned()),
        },
    )
    .await
    .expect("label capture");
    assert_eq!(labelled.status, "queued");

    // Competing workers claim with one atomic queued -> processing update.
    let first_claim_input = CaptureItemIdInput {
        capture_item_id: labelled.id.clone(),
    };
    let second_claim_input = CaptureItemIdInput {
        capture_item_id: labelled.id.clone(),
    };
    let (first_claim, second_claim) = tokio::join!(
        mark_processing(&pool, first_claim_input),
        mark_processing(&pool, second_claim_input)
    );
    assert_ne!(first_claim.is_ok(), second_claim.is_ok());
    let processing = first_claim
        .or(second_claim)
        .expect("one worker claims item");
    assert_eq!(processing.status, "processing");

    let annotated = output.join("001-annotated.png");
    tokio::fs::write(&annotated, b"annotated")
        .await
        .expect("annotated output");
    let pending = complete_processing(
        &pool,
        CompleteCaptureProcessingInput {
            capture_item_id: labelled.id,
            annotated_path: path_to_string(&annotated),
            avatar_path: None,
            face_box_json: Some(r#"{"x":10,"y":20,"width":30,"height":40}"#.to_owned()),
            face_feature: None,
            face_feature_model_id: None,
            face_feature_model_version: None,
            face_count: None,
            face_sharpness: None,
            face_area_ratio: None,
            warnings_json: None,
        },
    )
    .await
    .expect("complete processing");
    assert_eq!(pending.status, "archive_pending");
    assert!(pending.processed_at.is_some());

    let history = list_history(
        &pool,
        ListCaptureHistoryInput {
            project_id: project.id,
            session_id: None,
            character_id: None,
            status: Some("archive_pending".to_owned()),
            limit: None,
            offset: None,
            include_private: None,
        },
    )
    .await
    .expect("list history");
    assert_eq!(history.total, 1);
    assert_eq!(history.entries.len(), 1);
    assert_eq!(history.entries[0].character_name.as_deref(), Some("Aurora"));

    let ended = end_session(
        &pool,
        EndCaptureSessionInput {
            session_id: session.id,
            status: None,
        },
    )
    .await
    .expect("end session");
    assert_eq!(ended.status, "completed");
    assert!(ended.ended_at.is_some());
}

#[tokio::test]
async fn rejects_source_overwrite_and_invalid_transitions() {
    let pool = db::test_pool().await;
    let project = project(&pool).await;
    let (_workspace, session, source) = session_with_source(&pool, &project.id).await;
    let screenshot = source.join("capture.png");
    tokio::fs::write(&screenshot, b"source")
        .await
        .expect("source screenshot");
    let item = register_capture(
        &pool,
        RegisterCaptureInput {
            session_id: session.id,
            source_path: path_to_string(&screenshot),
        },
    )
    .await
    .expect("register");

    let result = mark_processing(
        &pool,
        CaptureItemIdInput {
            capture_item_id: item.id.clone(),
        },
    )
    .await;
    assert!(matches!(result, Err(AppError::Conflict(_))));

    sqlx::query("UPDATE capture_items SET status = 'processing' WHERE id = ?")
        .bind(&item.id)
        .execute(&pool)
        .await
        .expect("arrange processing");
    let result = complete_processing(
        &pool,
        CompleteCaptureProcessingInput {
            capture_item_id: item.id,
            annotated_path: path_to_string(&screenshot),
            avatar_path: None,
            face_box_json: None,
            face_feature: None,
            face_feature_model_id: None,
            face_feature_model_version: None,
            face_count: None,
            face_sharpness: None,
            face_area_ratio: None,
            warnings_json: None,
        },
    )
    .await;
    assert!(matches!(result, Err(AppError::Validation(_))));
    assert!(tokio::fs::try_exists(&screenshot)
        .await
        .expect("source still exists"));
}

#[tokio::test]
async fn claim_next_queued_skip_flag_filters_person_items() {
    let pool = db::test_pool().await;
    let project = project(&pool).await;
    let (_workspace, session, source) = session_with_source(&pool, &project.id).await;

    let person_path = source.join("person.png");
    let scene_path = source.join("scene.png");
    tokio::fs::write(&person_path, b"person")
        .await
        .expect("person");
    tokio::fs::write(&scene_path, b"scene")
        .await
        .expect("scene");
    let person_item = register_capture(
        &pool,
        RegisterCaptureInput {
            session_id: session.id.clone(),
            source_path: path_to_string(&person_path),
        },
    )
    .await
    .expect("register person");
    let scene_item = register_capture(
        &pool,
        RegisterCaptureInput {
            session_id: session.id.clone(),
            source_path: path_to_string(&scene_path),
        },
    )
    .await
    .expect("register scene");

    let character = character_service::create(
        &pool,
        CreateCharacterInput {
            project_id: project.id.clone(),
            name: "Aurora".to_owned(),
            aliases_json: None,
        },
    )
    .await
    .expect("create character");
    label_capture(
        &pool,
        LabelCaptureInput {
            capture_item_id: person_item.id.clone(),
            character_id: Some(character.id),
            classification: Some("person".to_owned()),
        },
    )
    .await
    .expect("label person");
    label_capture(
        &pool,
        LabelCaptureInput {
            capture_item_id: scene_item.id.clone(),
            character_id: None,
            classification: Some("scene".to_owned()),
        },
    )
    .await
    .expect("label scene");

    // Person is first in the queue but must be skipped without vision.
    let skipped = peek_next_queued(&pool, true)
        .await
        .expect("peek skipping person")
        .expect("scene is next");
    assert_eq!(skipped.id, scene_item.id);

    let claimed = claim_next_queued(&pool, true)
        .await
        .expect("claim skipping person")
        .expect("scene claimed");
    assert_eq!(claimed.id, scene_item.id);
    assert_eq!(claimed.status, "processing");

    let person_still = get_item(&pool, &person_item.id)
        .await
        .expect("person still queued");
    assert_eq!(person_still.status, "queued");

    // With vision configured the person item becomes eligible again.
    let person_next = peek_next_queued(&pool, false)
        .await
        .expect("peek including person")
        .expect("person is next");
    assert_eq!(person_next.id, person_item.id);
}

#[tokio::test]
async fn start_session_snapshots_project_directories() {
    let pool = db::test_pool().await;
    let project = project(&pool).await;
    let (_workspace, session, source) = session_with_source(&pool, &project.id).await;

    let dirs = session_source_directories(&pool, &session.id)
        .await
        .expect("session directories");
    assert_eq!(dirs.len(), 1);
    assert_eq!(
        dirs[0].directory,
        path_to_string(
            &tokio::fs::canonicalize(&source)
                .await
                .expect("canonical source")
        )
    );
    assert_eq!(dirs[0].baseline_initialized, 1);

    let updated = crate::services::project_service::list(&pool)
        .await
        .expect("list projects")
        .into_iter()
        .find(|value| value.id == project.id)
        .expect("project exists");
    assert!(updated.destination_directory.is_some());
}

#[tokio::test]
async fn popup_context_returns_all_unclassified_items_oldest_first() {
    let pool = db::test_pool().await;
    let project = project(&pool).await;
    let (_workspace, session, source) = session_with_source(&pool, &project.id).await;

    let first_path = source.join("first.png");
    let second_path = source.join("second.png");
    tokio::fs::write(&first_path, b"first")
        .await
        .expect("first");
    tokio::fs::write(&second_path, b"second")
        .await
        .expect("second");
    let first = register_capture(
        &pool,
        RegisterCaptureInput {
            session_id: session.id.clone(),
            source_path: path_to_string(&first_path),
        },
    )
    .await
    .expect("first capture");
    let second = register_capture(
        &pool,
        RegisterCaptureInput {
            session_id: session.id.clone(),
            source_path: path_to_string(&second_path),
        },
    )
    .await
    .expect("second capture");

    let context = classify_popup_context(&pool).await.expect("popup context");
    assert_eq!(context.items.len(), 2);
    assert_eq!(context.items[0].id, first.id);
    assert_eq!(context.items[1].id, second.id);
    assert_eq!(context.project_id.as_deref(), Some(project.id.as_str()));
    assert_eq!(context.project_name.as_deref(), Some(project.name.as_str()));

    // Classifying one item removes it from the popup list.
    let character = character_service::create(
        &pool,
        CreateCharacterInput {
            project_id: project.id.clone(),
            name: "Aurora".to_owned(),
            aliases_json: None,
        },
    )
    .await
    .expect("character");
    label_capture(
        &pool,
        LabelCaptureInput {
            capture_item_id: first.id,
            character_id: Some(character.id),
            classification: Some("person".to_owned()),
        },
    )
    .await
    .expect("label first");
    let context = classify_popup_context(&pool)
        .await
        .expect("popup context after label");
    assert_eq!(context.items.len(), 1);
    assert_eq!(context.items[0].id, second.id);
}

#[tokio::test]
async fn popup_context_only_returns_items_from_the_active_project() {
    let pool = db::test_pool().await;
    let old_project = project(&pool).await;
    let (_old_workspace, old_session, old_source) =
        session_with_source(&pool, &old_project.id).await;
    let old_path = old_source.join("old-project.png");
    tokio::fs::write(&old_path, b"old project")
        .await
        .expect("old project image");
    let old_item = register_capture(
        &pool,
        RegisterCaptureInput {
            session_id: old_session.id,
            source_path: path_to_string(&old_path),
        },
    )
    .await
    .expect("old project capture");

    let active_project = project(&pool).await;
    let (_active_workspace, active_session, active_source) =
        session_with_source(&pool, &active_project.id).await;
    let active_path = active_source.join("active-project.png");
    tokio::fs::write(&active_path, b"active project")
        .await
        .expect("active project image");
    let active_item = register_capture(
        &pool,
        RegisterCaptureInput {
            session_id: active_session.id,
            source_path: path_to_string(&active_path),
        },
    )
    .await
    .expect("active project capture");

    let context = classify_popup_context(&pool).await.expect("popup context");
    assert_eq!(
        context.project_id.as_deref(),
        Some(active_project.id.as_str())
    );
    assert_eq!(context.items.len(), 1);
    assert_eq!(context.items[0].id, active_item.id);
    assert_ne!(context.items[0].id, old_item.id);
}

#[tokio::test]
async fn popup_context_is_empty_without_an_active_session() {
    let pool = db::test_pool().await;
    let project = project(&pool).await;
    let (_workspace, session, source) = session_with_source(&pool, &project.id).await;
    let path = source.join("waiting.png");
    tokio::fs::write(&path, b"waiting")
        .await
        .expect("waiting image");
    register_capture(
        &pool,
        RegisterCaptureInput {
            session_id: session.id.clone(),
            source_path: path_to_string(&path),
        },
    )
    .await
    .expect("waiting capture");
    end_session(
        &pool,
        EndCaptureSessionInput {
            session_id: session.id,
            status: Some("completed".to_owned()),
        },
    )
    .await
    .expect("end session");

    let context = classify_popup_context(&pool).await.expect("popup context");
    assert!(context.items.is_empty());
    assert!(context.project_id.is_none());
    assert!(context.characters.is_empty());
}

#[tokio::test]
async fn registration_enforces_session_directory_type_stability_and_lifecycle() {
    let pool = db::test_pool().await;
    let project = project(&pool).await;
    let (workspace, session, source) = session_with_source(&pool, &project.id).await;
    let screenshot = source.join("capture.png");
    let text_file = source.join("capture.txt");
    let outside = workspace.path().join("outside.png");
    tokio::fs::write(&text_file, b"text").await.expect("text");
    tokio::fs::write(&outside, b"outside")
        .await
        .expect("outside");
    tokio::fs::write(&screenshot, b"stable image")
        .await
        .expect("screenshot");

    let outside_result = register_capture(
        &pool,
        RegisterCaptureInput {
            session_id: session.id.clone(),
            source_path: path_to_string(&outside),
        },
    )
    .await;
    assert!(matches!(outside_result, Err(AppError::Validation(_))));
    let type_result = register_capture(
        &pool,
        RegisterCaptureInput {
            session_id: session.id.clone(),
            source_path: path_to_string(&text_file),
        },
    )
    .await;
    assert!(matches!(type_result, Err(AppError::Validation(_))));

    let registered = register_capture(
        &pool,
        RegisterCaptureInput {
            session_id: session.id.clone(),
            source_path: path_to_string(&screenshot),
        },
    )
    .await
    .expect("register");
    let duplicate = register_capture(
        &pool,
        RegisterCaptureInput {
            session_id: session.id.clone(),
            source_path: path_to_string(&screenshot),
        },
    )
    .await
    .expect("duplicate is idempotent");
    assert_eq!(registered.id, duplicate.id);

    end_session(
        &pool,
        EndCaptureSessionInput {
            session_id: session.id.clone(),
            status: None,
        },
    )
    .await
    .expect("end");
    let ended_result = register_capture(
        &pool,
        RegisterCaptureInput {
            session_id: session.id,
            source_path: path_to_string(&screenshot),
        },
    )
    .await;
    assert!(matches!(ended_result, Err(AppError::Conflict(_))));
}

#[tokio::test]
async fn startup_recovery_only_fails_in_flight_processing_jobs() {
    let pool = db::test_pool().await;
    let project = project(&pool).await;
    let (_workspace, session, source) = session_with_source(&pool, &project.id).await;
    let first_path = source.join("processing.png");
    let second_path = source.join("archive-pending.png");
    tokio::fs::write(&first_path, b"first")
        .await
        .expect("first");
    tokio::fs::write(&second_path, b"second")
        .await
        .expect("second");
    let processing = register_capture(
        &pool,
        RegisterCaptureInput {
            session_id: session.id.clone(),
            source_path: path_to_string(&first_path),
        },
    )
    .await
    .expect("processing item");
    let pending = register_capture(
        &pool,
        RegisterCaptureInput {
            session_id: session.id,
            source_path: path_to_string(&second_path),
        },
    )
    .await
    .expect("pending item");
    sqlx::query("UPDATE capture_items SET status = 'processing' WHERE id = ?")
        .bind(&processing.id)
        .execute(&pool)
        .await
        .expect("arrange processing");
    sqlx::query("UPDATE capture_items SET status = 'archive_pending' WHERE id = ?")
        .bind(&pending.id)
        .execute(&pool)
        .await
        .expect("arrange pending");

    assert_eq!(
        recover_interrupted_processing(&pool)
            .await
            .expect("recover"),
        1
    );
    let recovered = get_item(&pool, &processing.id).await.expect("recovered");
    let untouched = get_item(&pool, &pending.id).await.expect("untouched");
    assert_eq!(recovered.status, "failed");
    assert!(recovered
        .error_message
        .as_deref()
        .is_some_and(|message| message.contains("restart")));
    assert_eq!(untouched.status, "archive_pending");
}

#[tokio::test]
async fn archive_retries_back_off_three_times_then_require_manual_retry() {
    let pool = db::test_pool().await;
    let project = project(&pool).await;
    let (_workspace, session, source) = session_with_source(&pool, &project.id).await;
    let screenshot = source.join("retry.png");
    tokio::fs::write(&screenshot, b"source")
        .await
        .expect("source");
    let item = register_capture(
        &pool,
        RegisterCaptureInput {
            session_id: session.id,
            source_path: path_to_string(&screenshot),
        },
    )
    .await
    .expect("item");
    sqlx::query(
        "UPDATE capture_items SET status = 'failed', failure_stage = 'archive' WHERE id = ?",
    )
    .bind(&item.id)
    .execute(&pool)
    .await
    .expect("arrange failure");

    let first = schedule_archive_retry(&pool, &item.id, "NAS offline")
        .await
        .expect("first retry");
    assert_eq!(first.status, "archive_pending");
    assert_eq!(first.attempt_count, 1);
    assert!(first.next_retry_at.is_some());

    sqlx::query("UPDATE capture_items SET status = 'failed' WHERE id = ?")
        .bind(&item.id)
        .execute(&pool)
        .await
        .expect("second failure");
    let second = schedule_archive_retry(&pool, &item.id, "NAS offline")
        .await
        .expect("second retry");
    assert_eq!(second.status, "archive_pending");
    assert_eq!(second.attempt_count, 2);

    sqlx::query("UPDATE capture_items SET status = 'failed' WHERE id = ?")
        .bind(&item.id)
        .execute(&pool)
        .await
        .expect("third failure");
    let third = schedule_archive_retry(&pool, &item.id, "NAS offline")
        .await
        .expect("third retry");
    assert_eq!(third.status, "archive_pending");
    assert_eq!(third.attempt_count, 3);
    assert!(third.next_retry_at.is_some());

    sqlx::query("UPDATE capture_items SET status = 'failed' WHERE id = ?")
        .bind(&item.id)
        .execute(&pool)
        .await
        .expect("fourth failure");
    let terminal = schedule_archive_retry(&pool, &item.id, "NAS offline")
        .await
        .expect("terminal failure");
    assert_eq!(terminal.status, "failed");
    assert_eq!(terminal.attempt_count, 4);
    assert_eq!(terminal.failure_stage.as_deref(), Some("archive"));
    assert!(terminal.next_retry_at.is_none());
}

#[tokio::test]
async fn without_engine_marks_person_archive_pending_with_warning() {
    let pool = db::test_pool().await;
    let project = project(&pool).await;
    let (_workspace, session, source) = session_with_source(&pool, &project.id).await;
    let screenshot = source.join("shot.png");
    tokio::fs::write(&screenshot, b"raw")
        .await
        .expect("source screenshot");
    let item = register_capture(
        &pool,
        RegisterCaptureInput {
            session_id: session.id.clone(),
            source_path: path_to_string(&screenshot),
        },
    )
    .await
    .expect("register capture");
    let character = character_service::create(
        &pool,
        CreateCharacterInput {
            project_id: project.id.clone(),
            name: "Aurora".to_owned(),
            aliases_json: None,
        },
    )
    .await
    .expect("create character");
    let labelled = label_capture(
        &pool,
        LabelCaptureInput {
            capture_item_id: item.id.clone(),
            character_id: Some(character.id),
            classification: Some("person".to_owned()),
        },
    )
    .await
    .expect("label capture");
    let processing = mark_processing(
        &pool,
        CaptureItemIdInput {
            capture_item_id: labelled.id.clone(),
        },
    )
    .await
    .expect("claim processing");
    assert_eq!(processing.status, "processing");

    let pending = complete_processing_without_engine(&pool, &item.id)
        .await
        .expect("degraded complete");
    assert_eq!(pending.status, "archive_pending");
    assert_eq!(pending.classification, "person");
    assert!(pending.annotated_path.is_none());
    assert!(pending.avatar_path.is_none());
    let warnings: serde_json::Value =
        serde_json::from_str(&pending.processing_warnings_json).expect("warnings json");
    assert!(warnings.as_array().is_some_and(|list| !list.is_empty()));
}

#[tokio::test]
async fn retry_requeues_degraded_completed_person_item() {
    let pool = db::test_pool().await;
    let project = project(&pool).await;
    let (_workspace, session, source) = session_with_source(&pool, &project.id).await;
    let screenshot = source.join("shot.png");
    tokio::fs::write(&screenshot, b"raw")
        .await
        .expect("source screenshot");
    let item = register_capture(
        &pool,
        RegisterCaptureInput {
            session_id: session.id.clone(),
            source_path: path_to_string(&screenshot),
        },
    )
    .await
    .expect("register capture");
    let character = character_service::create(
        &pool,
        CreateCharacterInput {
            project_id: project.id.clone(),
            name: "Aurora".to_owned(),
            aliases_json: None,
        },
    )
    .await
    .expect("create character");
    label_capture(
        &pool,
        LabelCaptureInput {
            capture_item_id: item.id.clone(),
            character_id: Some(character.id.clone()),
            classification: Some("person".to_owned()),
        },
    )
    .await
    .expect("label capture");

    // Simulate the terminal state of the degraded fallback: completed with
    // no annotated output and a warning.
    sqlx::query(
        r#"
            UPDATE capture_items
            SET status = 'completed', processing_warnings_json = ?
            WHERE id = ?
            "#,
    )
    .bind(serde_json::json!(["AI 引擎未配置"]).to_string())
    .bind(&item.id)
    .execute(&pool)
    .await
    .expect("mark completed degraded");

    let requeued = retry_capture(
        &pool,
        RetryCaptureInput {
            capture_item_id: item.id.clone(),
        },
    )
    .await
    .expect("retry degraded");
    assert_eq!(requeued.status, "queued");
    assert_eq!(requeued.processing_warnings_json, "[]");
    assert!(requeued.annotated_path.is_none());
    assert_eq!(
        requeued.character_id.as_deref(),
        Some(character.id.as_str())
    );
}

#[tokio::test]
async fn relabel_updates_queued_item_directly_and_clears_suggestions() {
    let pool = db::test_pool().await;
    let proj = project(&pool).await;
    let (workspace, session, source) = session_with_source(&pool, &proj.id).await;
    let character = character_service::create(
        &pool,
        CreateCharacterInput {
            project_id: proj.id.clone(),
            name: "Ava".to_owned(),
            aliases_json: None,
        },
    )
    .await
    .expect("character");
    let other = character_service::create(
        &pool,
        CreateCharacterInput {
            project_id: proj.id.clone(),
            name: "Bella".to_owned(),
            aliases_json: None,
        },
    )
    .await
    .expect("other character");
    let screenshot = source.join("capture.png");
    tokio::fs::write(&screenshot, b"source")
        .await
        .expect("screenshot");
    let item = register_capture(
        &pool,
        RegisterCaptureInput {
            session_id: session.id,
            source_path: path_to_string(&screenshot),
        },
    )
    .await
    .expect("register");
    let item = label_capture(
        &pool,
        LabelCaptureInput {
            capture_item_id: item.id.clone(),
            character_id: Some(character.id.clone()),
            classification: Some("person".to_owned()),
        },
    )
    .await
    .expect("label");
    assert_eq!(item.status, "queued");
    recognition_service::set_suggestion(
        &pool,
        SetRecognitionSuggestionInput {
            capture_item_id: item.id.clone(),
            suggested_character_id: Some(other.id.clone()),
            confidence: Some(0.9),
            source: Some("vision".to_owned()),
        },
    )
    .await
    .expect("suggest");
    sqlx::query(
        r#"
            INSERT INTO character_face_samples (
                id, character_id, capture_item_id, feature_json, status
            )
            VALUES ('relabel-sample', ?, ?, '[1.0]', 'active')
            "#,
    )
    .bind(&character.id)
    .bind(&item.id)
    .execute(&pool)
    .await
    .expect("face sample");

    let relabeled = relabel_capture_item(
        &pool,
        RelabelCaptureInput {
            capture_item_id: item.id.clone(),
            character_id: Some(character.id.clone()),
            classification: Some("scene".to_owned()),
        },
    )
    .await
    .expect("relabel");
    assert_eq!(relabeled.status, "queued");
    assert_eq!(relabeled.classification, "scene");
    // Scene captures are outside the person dimension: even a provided
    // character is cleared on relabel.
    assert!(relabeled.character_id.is_none());
    assert!(relabeled.suggested_character_id.is_none());
    assert!(relabeled.recognition_confidence.is_none());
    assert_eq!(relabeled.review_status, "none");
    let sample_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM character_face_samples WHERE capture_item_id = ?")
            .bind(&item.id)
            .fetch_one(&pool)
            .await
            .expect("sample count");
    assert_eq!(sample_count, 0, "relabel away from person drops the sample");
    let _ = workspace;
}

#[tokio::test]
async fn private_classification_clears_a_pinned_project_cover() {
    let pool = db::test_pool().await;
    let proj = project(&pool).await;
    let (workspace, session, source) = session_with_source(&pool, &proj.id).await;

    let pending_path = source.join("pending-cover.png");
    tokio::fs::write(&pending_path, b"pending")
        .await
        .expect("pending source");
    let pending = register_capture(
        &pool,
        RegisterCaptureInput {
            session_id: session.id.clone(),
            source_path: path_to_string(&pending_path),
        },
    )
    .await
    .expect("register pending cover");
    project_service::set_cover(
        &pool,
        SetProjectCoverInput {
            project_id: proj.id.clone(),
            capture_item_id: Some(pending.id.clone()),
        },
    )
    .await
    .expect("pin pending cover");
    label_capture(
        &pool,
        LabelCaptureInput {
            capture_item_id: pending.id,
            character_id: None,
            classification: Some("private".to_owned()),
        },
    )
    .await
    .expect("label private");
    let cover_after_label: Option<String> =
        sqlx::query_scalar("SELECT cover_capture_item_id FROM projects WHERE id = ?")
            .bind(&proj.id)
            .fetch_one(&pool)
            .await
            .expect("cover after label");
    assert!(cover_after_label.is_none());

    let completed_path = source.join("completed-cover.png");
    tokio::fs::write(&completed_path, b"completed")
        .await
        .expect("completed source");
    let completed = register_capture(
        &pool,
        RegisterCaptureInput {
            session_id: session.id,
            source_path: path_to_string(&completed_path),
        },
    )
    .await
    .expect("register completed cover");
    sqlx::query(
        "UPDATE capture_items SET classification = 'scene', status = 'completed' WHERE id = ?",
    )
    .bind(&completed.id)
    .execute(&pool)
    .await
    .expect("complete capture");
    project_service::set_cover(
        &pool,
        SetProjectCoverInput {
            project_id: proj.id.clone(),
            capture_item_id: Some(completed.id.clone()),
        },
    )
    .await
    .expect("pin completed cover");
    relabel_capture_item(
        &pool,
        RelabelCaptureInput {
            capture_item_id: completed.id,
            character_id: None,
            classification: Some("private".to_owned()),
        },
    )
    .await
    .expect("relabel completed cover private");
    let cover_after_relabel: Option<String> =
        sqlx::query_scalar("SELECT cover_capture_item_id FROM projects WHERE id = ?")
            .bind(proj.id)
            .fetch_one(&pool)
            .await
            .expect("cover after relabel");
    assert!(cover_after_relabel.is_none());
    let _ = workspace;
}

#[tokio::test]
async fn relabel_rejects_in_flight_cross_project_and_missing_character() {
    let pool = db::test_pool().await;
    let proj = project(&pool).await;
    let (workspace, session, source) = session_with_source(&pool, &proj.id).await;
    let character = character_service::create(
        &pool,
        CreateCharacterInput {
            project_id: proj.id.clone(),
            name: "Ava".to_owned(),
            aliases_json: None,
        },
    )
    .await
    .expect("character");
    let screenshot = source.join("capture.png");
    tokio::fs::write(&screenshot, b"source")
        .await
        .expect("screenshot");
    let item = register_capture(
        &pool,
        RegisterCaptureInput {
            session_id: session.id,
            source_path: path_to_string(&screenshot),
        },
    )
    .await
    .expect("register");

    let error = relabel_capture_item(
        &pool,
        RelabelCaptureInput {
            capture_item_id: item.id.clone(),
            character_id: None,
            classification: Some("person".to_owned()),
        },
    )
    .await
    .expect_err("person without character");
    assert!(matches!(error, AppError::Validation(_)));

    let other_project = project(&pool).await;
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
    let error = relabel_capture_item(
        &pool,
        RelabelCaptureInput {
            capture_item_id: item.id.clone(),
            character_id: Some(stranger.id),
            classification: Some("person".to_owned()),
        },
    )
    .await
    .expect_err("cross-project character");
    assert!(matches!(error, AppError::Validation(_)));

    let item = label_capture(
        &pool,
        LabelCaptureInput {
            capture_item_id: item.id.clone(),
            character_id: Some(character.id.clone()),
            classification: Some("person".to_owned()),
        },
    )
    .await
    .expect("label");
    let item = mark_processing(
        &pool,
        CaptureItemIdInput {
            capture_item_id: item.id.clone(),
        },
    )
    .await
    .expect("processing");
    let error = relabel_capture_item(
        &pool,
        RelabelCaptureInput {
            capture_item_id: item.id.clone(),
            character_id: None,
            classification: Some("scene".to_owned()),
        },
    )
    .await
    .expect_err("processing cannot relabel");
    assert!(matches!(error, AppError::Conflict(_)));

    let item = mark_archive_pending_from_processing(&pool, &item.id)
        .await
        .expect("archive pending");
    assert_eq!(item.status, "archive_pending");
    let error = relabel_capture_item(
        &pool,
        RelabelCaptureInput {
            capture_item_id: item.id,
            character_id: None,
            classification: Some("scene".to_owned()),
        },
    )
    .await
    .expect_err("archive pending cannot relabel");
    assert!(matches!(error, AppError::Conflict(_)));
    let _ = workspace;
}

#[tokio::test]
async fn relabel_noop_keeps_current_state() {
    let pool = db::test_pool().await;
    let proj = project(&pool).await;
    let (workspace, session, source) = session_with_source(&pool, &proj.id).await;
    let character = character_service::create(
        &pool,
        CreateCharacterInput {
            project_id: proj.id.clone(),
            name: "Ava".to_owned(),
            aliases_json: None,
        },
    )
    .await
    .expect("character");
    let screenshot = source.join("capture.png");
    tokio::fs::write(&screenshot, b"source")
        .await
        .expect("screenshot");
    let item = register_capture(
        &pool,
        RegisterCaptureInput {
            session_id: session.id,
            source_path: path_to_string(&screenshot),
        },
    )
    .await
    .expect("register");
    let item = label_capture(
        &pool,
        LabelCaptureInput {
            capture_item_id: item.id.clone(),
            character_id: Some(character.id.clone()),
            classification: Some("person".to_owned()),
        },
    )
    .await
    .expect("label");

    let unchanged = relabel_capture_item(
        &pool,
        RelabelCaptureInput {
            capture_item_id: item.id.clone(),
            character_id: Some(character.id.clone()),
            classification: Some("person".to_owned()),
        },
    )
    .await
    .expect("no-op relabel");
    assert_eq!(unchanged.id, item.id);
    assert_eq!(unchanged.status, "queued");
    assert_eq!(unchanged.classification, "person");
    let _ = workspace;
}

#[tokio::test]
async fn awaiting_label_feature_pass_picks_each_item_once() {
    let pool = db::test_pool().await;
    let proj = project(&pool).await;
    let (workspace, session, source) = session_with_source(&pool, &proj.id).await;
    let screenshot = source.join("capture.png");
    tokio::fs::write(&screenshot, b"source")
        .await
        .expect("screenshot");
    let item = register_capture(
        &pool,
        RegisterCaptureInput {
            session_id: session.id,
            source_path: path_to_string(&screenshot),
        },
    )
    .await
    .expect("register");
    assert_eq!(item.status, "awaiting_label");

    let picked = next_awaiting_label_without_feature(&pool)
        .await
        .expect("query")
        .expect("picked");
    assert_eq!(picked.id, item.id);

    store_face_feature(
        &pool,
        &item.id,
        FaceFeatureWrite {
            feature_json: Some("[]".to_owned()),
            face_box_json: None,
            model_id: None,
            model_version: None,
            face_count: None,
            face_sharpness: None,
            face_area_ratio: None,
        },
    )
    .await
    .expect("store marker");
    let again = next_awaiting_label_without_feature(&pool)
        .await
        .expect("query");
    assert!(again.is_none(), "attempted items must not be re-picked");
    let _ = workspace;
}

#[tokio::test]
async fn batch_retry_requeues_degraded_captures_all_and_per_character() {
    let pool = db::test_pool().await;
    let proj = project(&pool).await;
    let (workspace, session, source) = session_with_source(&pool, &proj.id).await;
    let ava = character_service::create(
        &pool,
        CreateCharacterInput {
            project_id: proj.id.clone(),
            name: "Ava".to_owned(),
            aliases_json: None,
        },
    )
    .await
    .expect("ava");
    let bella = character_service::create(
        &pool,
        CreateCharacterInput {
            project_id: proj.id.clone(),
            name: "Bella".to_owned(),
            aliases_json: None,
        },
    )
    .await
    .expect("bella");
    let mut ids = Vec::new();
    for (name, character) in [("one.png", &ava), ("two.png", &ava), ("three.png", &bella)] {
        let path = source.join(name);
        tokio::fs::write(&path, name).await.expect("source");
        let item = register_capture(
            &pool,
            RegisterCaptureInput {
                session_id: session.id.clone(),
                source_path: path_to_string(&path),
            },
        )
        .await
        .expect("register");
        let item = label_capture(
            &pool,
            LabelCaptureInput {
                capture_item_id: item.id,
                character_id: Some(character.id.clone()),
                classification: Some("person".to_owned()),
            },
        )
        .await
        .expect("label");
        // Simulate the degraded terminal state (no Python engine).
        sqlx::query(
            r#"
                UPDATE capture_items
                SET status = 'completed', processing_warnings_json = '[]'
                WHERE id = ? AND annotated_path IS NULL
                "#,
        )
        .bind(&item.id)
        .execute(&pool)
        .await
        .expect("degrade");
        ids.push(item.id);
    }

    let requeued = retry_degraded_captures(&pool, &proj.id, Some(&ava.id))
        .await
        .expect("per-character batch");
    assert_eq!(requeued, 2);
    for id in &ids[..2] {
        assert_eq!(get_item(&pool, id).await.expect("item").status, "queued");
    }
    assert_eq!(
        get_item(&pool, &ids[2]).await.expect("bella").status,
        "completed"
    );

    let requeued = retry_degraded_captures(&pool, &proj.id, None)
        .await
        .expect("all batch");
    assert_eq!(requeued, 1);
    let _ = workspace;
}

#[tokio::test]
async fn category_items_list_unclassified_scene_and_private() {
    let pool = db::test_pool().await;
    let proj = project(&pool).await;
    let (workspace, session, source) = session_with_source(&pool, &proj.id).await;
    let mut ids = Vec::new();
    for name in ["u.png", "s.png", "p.png"] {
        let path = source.join(name);
        tokio::fs::write(&path, name).await.expect("source");
        let item = register_capture(
            &pool,
            RegisterCaptureInput {
                session_id: session.id.clone(),
                source_path: path_to_string(&path),
            },
        )
        .await
        .expect("register");
        ids.push(item.id);
    }
    label_capture(
        &pool,
        LabelCaptureInput {
            capture_item_id: ids[1].clone(),
            character_id: None,
            classification: Some("scene".to_owned()),
        },
    )
    .await
    .expect("label scene");
    label_capture(
        &pool,
        LabelCaptureInput {
            capture_item_id: ids[2].clone(),
            character_id: None,
            classification: Some("private".to_owned()),
        },
    )
    .await
    .expect("label private");

    let unclassified = list_category_items(
        &pool,
        ListCategoryItemsInput {
            project_id: proj.id.clone(),
            category: "unclassified".to_owned(),
            page: None,
            page_size: None,
        },
    )
    .await
    .expect("unclassified");
    assert_eq!(unclassified.len(), 1);
    assert_eq!(unclassified[0].id, ids[0]);

    let scene = list_category_items(
        &pool,
        ListCategoryItemsInput {
            project_id: proj.id.clone(),
            category: "scene".to_owned(),
            page: None,
            page_size: None,
        },
    )
    .await
    .expect("scene");
    assert_eq!(scene.len(), 1);
    assert_eq!(scene[0].id, ids[1]);

    let private = list_category_items(
        &pool,
        ListCategoryItemsInput {
            project_id: proj.id.clone(),
            category: "private".to_owned(),
            page: None,
            page_size: None,
        },
    )
    .await
    .expect("private");
    assert_eq!(private.len(), 1);
    assert_eq!(private[0].id, ids[2]);

    let error = list_category_items(
        &pool,
        ListCategoryItemsInput {
            project_id: proj.id,
            category: "everything".to_owned(),
            page: None,
            page_size: None,
        },
    )
    .await
    .expect_err("invalid category");
    assert!(matches!(error, AppError::Validation(_)));
    let _ = workspace;
}

#[tokio::test]
async fn list_items_paged_returns_pages_with_total_and_deterministic_order() {
    let pool = db::test_pool().await;
    let proj = project(&pool).await;
    let (workspace, session, _source) = session_with_source(&pool, &proj.id).await;
    for index in 0..25 {
        insert_item(
            &pool,
            &session.id,
            &proj.id,
            "unclassified",
            &format!("{index:02}.png"),
        )
        .await;
    }

    let legacy = list_items(&pool, &session.id).await.expect("legacy list");
    assert_eq!(legacy.len(), 25);

    let page1 = list_items_paged(&pool, &session.id, 1, 10)
        .await
        .expect("page 1");
    assert_eq!(page1.total, 25);
    assert_eq!(page1.page, 1);
    assert_eq!(page1.page_size, 10);
    assert_eq!(page1.items.len(), 10);
    assert_eq!(page1.items[0].id, legacy[0].id);

    let page2 = list_items_paged(&pool, &session.id, 2, 10)
        .await
        .expect("page 2");
    assert_eq!(page2.items.len(), 10);
    assert_eq!(page2.items[0].id, legacy[10].id);

    let page3 = list_items_paged(&pool, &session.id, 3, 10)
        .await
        .expect("page 3");
    assert_eq!(page3.items.len(), 5);
    assert_eq!(page3.items[0].id, legacy[20].id);

    let mut paged_ids: Vec<&str> = page1
        .items
        .iter()
        .chain(&page2.items)
        .chain(&page3.items)
        .map(|item| item.id.as_str())
        .collect();
    paged_ids.sort_unstable();
    let mut legacy_ids: Vec<&str> = legacy.iter().map(|item| item.id.as_str()).collect();
    legacy_ids.sort_unstable();
    assert_eq!(paged_ids, legacy_ids, "pages cover every item exactly once");

    let deep = list_items_paged(&pool, &session.id, 100, 10)
        .await
        .expect("deep page");
    assert!(deep.items.is_empty());
    assert_eq!(deep.total, 25);
    let _ = workspace;
}

#[tokio::test]
async fn list_items_paged_rejects_invalid_bounds_and_missing_session() {
    let pool = db::test_pool().await;
    let proj = project(&pool).await;
    let (workspace, session, _source) = session_with_source(&pool, &proj.id).await;
    insert_item(&pool, &session.id, &proj.id, "unclassified", "one.png").await;

    let zero_page = list_items_paged(&pool, &session.id, 0, 10)
        .await
        .expect_err("page zero");
    assert!(matches!(zero_page, AppError::Validation(_)));

    let zero_size = list_items_paged(&pool, &session.id, 1, 0)
        .await
        .expect_err("page size zero");
    assert!(matches!(zero_size, AppError::Validation(_)));

    let huge_size = list_items_paged(&pool, &session.id, 1, 501)
        .await
        .expect_err("page size above max");
    assert!(matches!(huge_size, AppError::Validation(_)));

    let huge_offset = list_items_paged(&pool, &session.id, 1_000_002, 1)
        .await
        .expect_err("offset above max");
    assert!(matches!(huge_offset, AppError::Validation(_)));

    let missing = list_items_paged(&pool, "no-such-session", 1, 10)
        .await
        .expect_err("missing session");
    assert!(matches!(missing, AppError::NotFound(_)));
    let _ = workspace;
}

#[tokio::test]
async fn list_category_items_paged_returns_pages_with_total() {
    let pool = db::test_pool().await;
    let proj = project(&pool).await;
    let (workspace, session, _source) = session_with_source(&pool, &proj.id).await;
    for index in 0..12 {
        insert_item(
            &pool,
            &session.id,
            &proj.id,
            "scene",
            &format!("scene-{index:02}.png"),
        )
        .await;
    }
    insert_item(&pool, &session.id, &proj.id, "private", "private.png").await;

    let legacy = list_category_items(
        &pool,
        ListCategoryItemsInput {
            project_id: proj.id.clone(),
            category: "scene".to_owned(),
            page: None,
            page_size: None,
        },
    )
    .await
    .expect("legacy scene list");
    assert_eq!(legacy.len(), 12);

    let page1 = list_category_items_paged(
        &pool,
        ListCategoryItemsInput {
            project_id: proj.id.clone(),
            category: "scene".to_owned(),
            page: None,
            page_size: None,
        },
        1,
        5,
    )
    .await
    .expect("page 1");
    assert_eq!(page1.total, 12);
    assert_eq!(page1.items.len(), 5);
    assert_eq!(page1.items[0].id, legacy[0].id);

    let page3 = list_category_items_paged(
        &pool,
        ListCategoryItemsInput {
            project_id: proj.id.clone(),
            category: "scene".to_owned(),
            page: None,
            page_size: None,
        },
        3,
        5,
    )
    .await
    .expect("page 3");
    assert_eq!(page3.items.len(), 2);
    assert_eq!(page3.items[0].id, legacy[10].id);

    let private = list_category_items_paged(
        &pool,
        ListCategoryItemsInput {
            project_id: proj.id.clone(),
            category: "private".to_owned(),
            page: None,
            page_size: None,
        },
        1,
        5,
    )
    .await
    .expect("private page");
    assert_eq!(private.total, 1);
    assert_eq!(private.items.len(), 1);

    let error = list_category_items_paged(
        &pool,
        ListCategoryItemsInput {
            project_id: proj.id.clone(),
            category: "everything".to_owned(),
            page: None,
            page_size: None,
        },
        1,
        5,
    )
    .await
    .expect_err("invalid category");
    assert!(matches!(error, AppError::Validation(_)));
    let _ = workspace;
}

#[tokio::test]
async fn lists_and_imports_pre_existing_directory_images() {
    let pool = db::test_pool().await;
    let proj = project(&pool).await;
    let (workspace, session, source) = session_with_source(&pool, &proj.id).await;
    for (index, name) in ["a.png", "b.jpg", "note.txt"].iter().enumerate() {
        tokio::fs::write(source.join(name), format!("image data {index}"))
            .await
            .expect("write");
    }

    let listed = list_unimported_captures(&pool, &session.id)
        .await
        .expect("list");
    assert_eq!(listed.len(), 2);
    assert!(
        listed
            .iter()
            .all(|capture| !capture.path.ends_with("note.txt")),
        "non-image files are excluded"
    );

    let paths: Vec<String> = listed.iter().map(|capture| capture.path.clone()).collect();
    let imported = import_directory_captures(
        &pool,
        ImportDirectoryCapturesInput {
            session_id: session.id.clone(),
            paths,
        },
    )
    .await
    .expect("import");
    assert_eq!(imported.len(), 2);

    let items = list_items(&pool, &session.id).await.expect("items");
    assert_eq!(items.len(), 2);
    assert!(
        items.iter().all(|item| item.status == "awaiting_label"),
        "imported captures enter the awaiting-label queue"
    );
    assert_eq!(
        deferred_import_recognition_count(&pool, &session.id)
            .await
            .expect("deferred count"),
        2
    );
    assert!(
        next_awaiting_label_without_feature(&pool)
            .await
            .expect("worker pick")
            .is_none(),
        "the worker must not recognize imported captures before approval"
    );
    assert_eq!(
        start_imported_recognition(&pool, &session.id)
            .await
            .expect("start imported recognition"),
        2
    );
    assert!(
        next_awaiting_label_without_feature(&pool)
            .await
            .expect("worker pick after start")
            .is_some(),
        "the worker may take one imported capture after approval"
    );
    assert_eq!(
        deferred_import_recognition_count(&pool, &session.id)
            .await
            .expect("deferred count after start"),
        0
    );
    assert!(
        list_unimported_captures(&pool, &session.id)
            .await
            .expect("list again")
            .is_empty(),
        "imported images are no longer listed"
    );

    let again = import_directory_captures(
        &pool,
        ImportDirectoryCapturesInput {
            session_id: session.id,
            paths: items.iter().map(|item| item.source_path.clone()).collect(),
        },
    )
    .await
    .expect("re-import");
    assert_eq!(again.len(), 0, "already-registered paths are skipped");
    let _ = workspace;
}

#[tokio::test]
async fn import_is_deduplicated_across_sessions() {
    let pool = db::test_pool().await;
    let proj = project(&pool).await;
    let (workspace, session, source) = session_with_source(&pool, &proj.id).await;
    for (index, name) in ["a.png", "b.jpg"].iter().enumerate() {
        tokio::fs::write(source.join(name), format!("image data {index}"))
            .await
            .expect("write");
    }
    let paths: Vec<String> = list_unimported_captures(&pool, &session.id)
        .await
        .expect("list")
        .iter()
        .map(|capture| capture.path.clone())
        .collect();
    assert_eq!(paths.len(), 2);
    assert_eq!(
        import_directory_captures(
            &pool,
            ImportDirectoryCapturesInput {
                session_id: session.id.clone(),
                paths: paths.clone(),
            },
        )
        .await
        .expect("import")
        .len(),
        2
    );

    // A second session pointed at the same directory must not re-offer or
    // re-import the already-registered files.
    let session2 = test_support::start_session(&pool, &proj.id)
        .await
        .expect("second session");
    assert!(
        list_unimported_captures(&pool, &session2.id)
            .await
            .expect("list second session")
            .is_empty(),
        "already-registered files must not be offered again"
    );
    assert_eq!(
        import_directory_captures(
            &pool,
            ImportDirectoryCapturesInput {
                session_id: session2.id,
                paths,
            },
        )
        .await
        .expect("import second session")
        .len(),
        0,
        "cross-session re-import must be skipped"
    );
    let _ = workspace;
}

#[tokio::test]
async fn identical_content_in_another_directory_registers_only_once() {
    let pool = db::test_pool().await;
    let proj = project(&pool).await;
    let (workspace, session, source) = session_with_source(&pool, &proj.id).await;
    let first = source.join("same.png");
    tokio::fs::write(&first, b"identical content")
        .await
        .expect("first file");
    let first_item = register_capture(
        &pool,
        RegisterCaptureInput {
            session_id: session.id.clone(),
            source_path: path_to_string(&first),
        },
    )
    .await
    .expect("first register");
    assert_eq!(
        first_item.content_hash.as_deref().map(str::len),
        Some(64),
        "sha256 fingerprint is stored"
    );

    // A second project directory holding an identical file must not
    // create a second item: the fingerprint dedup maps it to the first.
    let second_dir = workspace.path().join("version2");
    tokio::fs::create_dir(&second_dir)
        .await
        .expect("second dir");
    project_service::add_source_directory(
        &pool,
        crate::models::project::AddProjectSourceDirectoryInput {
            project_id: proj.id.clone(),
            directory: path_to_string(&second_dir),
        },
    )
    .await
    .expect("add second source directory");
    let second_session = test_support::start_session(&pool, &proj.id)
        .await
        .expect("second session");
    let second = second_dir.join("same.png");
    tokio::fs::write(&second, b"identical content")
        .await
        .expect("second file");
    let duplicate = register_capture(
        &pool,
        RegisterCaptureInput {
            session_id: second_session.id.clone(),
            source_path: path_to_string(&second),
        },
    )
    .await
    .expect("duplicate register");
    assert_eq!(
        duplicate.id, first_item.id,
        "identical content maps to the already-registered item"
    );
    assert_eq!(duplicate.session_id, session.id);

    let total =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM capture_items WHERE project_id = ?")
            .bind(&proj.id)
            .fetch_one(&pool)
            .await
            .expect("count");
    assert_eq!(total, 1);
}

#[tokio::test]
async fn imports_baselined_images_from_a_chinese_named_directory() {
    let pool = db::test_pool().await;
    let project = project(&pool).await;
    let workspace = tempdir().expect("tempdir");
    let source = workspace.path().join("source");
    let chinese = workspace.path().join("测试目录");
    let destination = workspace.path().join("archive");
    tokio::fs::create_dir(&source).await.expect("source dir");
    tokio::fs::create_dir(&chinese).await.expect("chinese dir");
    tokio::fs::create_dir(&destination).await.expect("destination");
    project_service::set_destination_directory(
        &pool,
        SetProjectDestinationInput {
            project_id: project.id.clone(),
            directory: path_to_string(&destination),
        },
    )
    .await
    .expect("destination");
    project_service::add_source_directory(
        &pool,
        AddProjectSourceDirectoryInput {
            project_id: project.id.clone(),
            directory: path_to_string(&source),
        },
    )
    .await
    .expect("add source");
    project_service::add_source_directory(
        &pool,
        AddProjectSourceDirectoryInput {
            project_id: project.id.clone(),
            directory: path_to_string(&chinese),
        },
    )
    .await
    .expect("add chinese source");
    for (index, name) in ["a.png", "b.png", "c.png"].iter().enumerate() {
        tokio::fs::write(chinese.join(name), format!("distinct image {index}"))
            .await
            .expect("image");
    }
    let session = start_session(
        &pool,
        StartCaptureSessionInput {
            project_id: project.id,
        },
    )
    .await
    .expect("start session")
    .session;

    let unimported = list_unimported_captures(&pool, &session.id)
        .await
        .expect("list unimported");
    assert_eq!(
        unimported.len(),
        3,
        "baselined images in a Chinese-named directory must appear in the import dialog"
    );
    let paths: Vec<String> = unimported
        .iter()
        .map(|candidate| candidate.path.clone())
        .collect();
    let imported = import_directory_captures(
        &pool,
        ImportDirectoryCapturesInput {
            session_id: session.id.clone(),
            paths,
        },
    )
    .await
    .expect("import");
    assert_eq!(
        imported.len(),
        3,
        "baselined images must import from a Chinese-named directory"
    );
    let registered: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM capture_items WHERE session_id = ?")
            .bind(&session.id)
            .fetch_one(&pool)
            .await
            .expect("registered count");
    assert_eq!(registered, 3);
}

#[tokio::test]
async fn recent_items_surface_awaiting_label_captures_from_ended_sessions() {
    let pool = db::test_pool().await;
    let project = project(&pool).await;
    let (_workspace, first_session, _source) = session_with_source(&pool, &project.id).await;

    // One capture stays awaiting-label in the ended session; one is archived.
    let pending = insert_item(&pool, &first_session.id, &project.id, "person", "pending.png").await;
    let done = insert_item(&pool, &first_session.id, &project.id, "person", "done.png").await;
    transition_item(&pool, &done, "awaiting_label", "completed", false)
        .await
        .expect("complete");
    end_session(
        &pool,
        EndCaptureSessionInput {
            session_id: first_session.id.clone(),
            status: None,
        },
    )
    .await
    .expect("end session");

    // A second session starts and gets its own capture.
    let second = start_session(
        &pool,
        StartCaptureSessionInput {
            project_id: project.id.clone(),
        },
    )
    .await
    .expect("second session")
    .session;
    let live = insert_item(&pool, &second.id, &project.id, "person", "live.png").await;

    let recent = list_project_recent_items(&pool, &project.id, 100)
        .await
        .expect("recent");
    let ids: Vec<String> = recent.iter().map(|item| item.id.clone()).collect();
    assert!(
        ids.contains(&pending),
        "awaiting-label capture from the ended session must stay visible"
    );
    assert!(
        !ids.contains(&done),
        "completed capture from the ended session must not reappear"
    );
    assert!(ids.contains(&live), "active-session capture must be visible");
}
