use super::*;
use crate::{db, models::capture::{RegisterCaptureInput, LabelCaptureInput}, services::test_support};
use std::sync::{atomic::{AtomicUsize, Ordering}, Mutex};

#[derive(Default)]
struct Recycler { calls: AtomicUsize, fail_on: AtomicUsize, deleted: Mutex<Vec<String>> }
impl FileRecycler for Recycler {
    async fn recycle(&self, path: &Path) -> Result<(), AppError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        // ERROR_SHARING_VIOLATION: the failure a real "file is in use" reset hits.
        if self.fail_on.load(Ordering::SeqCst) == call { return Err(AppError::Io(std::io::Error::from_raw_os_error(32))); }
        self.deleted.lock().unwrap().push(path.to_string_lossy().into());
        tokio::fs::remove_file(path).await?;
        Ok(())
    }
}
async fn fixture() -> (SqlitePool, test_support::ProjectFixture, String, String) {
    let pool = db::test_pool().await;
    let f = test_support::project_with_directories(&pool, "reset").await.unwrap();
    let session = test_support::start_session(&pool, &f.project_id).await.unwrap();
    let id = add(&pool, &f, &session.id, "one.png").await;
    (pool, f, session.id, id)
}
async fn add(pool: &SqlitePool, f: &test_support::ProjectFixture, session: &str, name: &str) -> String {
    let source = f.source_directory.join(name);
    tokio::fs::write(&source, format!("original-{name}")).await.unwrap();
    let item = capture_service::register_capture(pool, RegisterCaptureInput { session_id:session.into(), source_path:source.to_string_lossy().into() }).await.unwrap();
    let dest = f.destination_directory.join(name);
    tokio::fs::write(&dest, format!("archive-{name}")).await.unwrap();
    sqlx::query("UPDATE capture_items SET classification='scene',status='completed',destination_path=?,archived_at='2026-09-01T00:00:00Z',processed_at='2026-09-01T00:00:00Z' WHERE id=?")
        .bind(dest.to_string_lossy().as_ref()).bind(&item.id).execute(pool).await.unwrap();
    item.id
}
fn selection(project: &str) -> ResetPreviewInput { ResetPreviewInput { project_id:project.into(),capture_item_ids:None,session_id:None,character_id:None,status:None,include_private:Some(false) } }
fn request(id: &str, delete: bool) -> ResetExecuteInput { ResetExecuteInput { job_id:id.into(),delete_destination_files:delete,allow_permanent_network_delete:false } }

#[tokio::test]
async fn preview_reports_ineligible_pictures_without_a_job_row() {
    let (pool,f,session,id) = fixture().await;
    let missing=add(&pool,&f,&session,"missing.png").await;
    tokio::fs::remove_file(f.source_directory.join("missing.png")).await.unwrap();
    let job = preview(&pool, selection(&f.project_id)).await.unwrap();
    assert_eq!(job.status, "preview");
    assert_eq!(job.items.len(), 2);
    let skipped = job.items.iter().find(|item| item.capture_item_id == missing).unwrap();
    assert_eq!(skipped.status, "skipped");
    assert_eq!(skipped.reason.as_deref(), Some("source_gone"));
    // Nothing about a reset is persisted any more.
    let tables: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name LIKE 'capture_reset%'").fetch_one(&pool).await.unwrap();
    assert_eq!(tables, 0);
    let ready = job.items.iter().find(|item| item.capture_item_id == id).unwrap();
    assert_eq!(ready.status, "ready");
    assert_eq!(job.destination_file_count, 1);
}

#[tokio::test]
async fn execute_releases_the_claim_and_bumps_the_reset_generation() {
    let (pool,f,_,id) = fixture().await;
    let job = preview(&pool, selection(&f.project_id)).await.unwrap();
    let recycler = Recycler::default();
    let result = execute(&pool,&f.output_directory,&f.output_directory,request(&job.id,true),&recycler).await.unwrap();
    assert_eq!(result.status,"completed"); assert!(!result.executing);
    assert_eq!(result.items[0].status,"succeeded");
    let item = capture_service::get_item(&pool,&id).await.unwrap();
    assert_eq!(item.classification,"unclassified"); assert_eq!(item.status,"awaiting_label");
    assert!(item.archived_at.is_none() && item.destination_path.is_none());
    assert!(Path::new(&item.source_path).exists());
    let state:(Option<String>,i64,i64) = sqlx::query_as("SELECT operation_owner,reset_generation,recognition_deferred FROM capture_items WHERE id=?")
        .bind(&id).fetch_one(&pool).await.unwrap();
    // The claim is released in the same run, and the generation feeds archive naming.
    assert_eq!(state,(None,1,0));
    assert_eq!(capture_service::next_awaiting_label_without_feature(&pool).await.unwrap().unwrap().id,id);
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM ignored_capture_contents").fetch_one(&pool).await.unwrap(),0);
    assert_eq!(recycler.calls.load(Ordering::SeqCst),1);
}

#[tokio::test]
async fn a_failed_picture_is_reported_released_and_retryable() {
    let (pool,f,_,id) = fixture().await;
    let job = preview(&pool, selection(&f.project_id)).await.unwrap();
    let recycler = Recycler::default();
    recycler.fail_on.store(1,Ordering::SeqCst);
    let result = execute(&pool,&f.output_directory,&f.output_directory,request(&job.id,true),&recycler).await.unwrap();
    // Failed items end the job instead of leaving a queue behind.
    assert_eq!(result.status,"completed");
    assert_eq!(result.items[0].status,"failed");
    assert_eq!(result.items[0].reason.as_deref(),Some("locked"));
    assert!(!result.items[0].error.as_deref().unwrap_or_default().is_empty());
    assert_eq!(capture_service::get_item(&pool,&id).await.unwrap().classification,"scene");
    // The picture must not stay locked out of labeling after a failure.
    assert!(capture_operation_service::ensure_available(&pool,&id).await.is_ok());
    let state:(Option<String>,i64) = sqlx::query_as("SELECT operation_owner,reset_generation FROM capture_items WHERE id=?").bind(&id).fetch_one(&pool).await.unwrap();
    assert_eq!(state,(None,0));
    // Retrying the same preview runs the unfinished picture again.
    let result = execute(&pool,&f.output_directory,&f.output_directory,request(&job.id,true),&Recycler::default()).await.unwrap();
    assert_eq!(result.items[0].status,"succeeded");
    assert_eq!(capture_service::get_item(&pool,&id).await.unwrap().classification,"unclassified");
}

#[tokio::test]
async fn interrupted_claims_are_released_at_startup() {
    let (pool,_,_,id) = fixture().await;
    sqlx::query("UPDATE capture_items SET operation_owner='reset:killed' WHERE id=?").bind(&id).execute(&pool).await.unwrap();
    assert!(capture_operation_service::ensure_available(&pool,&id).await.is_err());
    assert_eq!(recover_interrupted(&pool).await.unwrap(),1);
    assert!(capture_operation_service::ensure_available(&pool,&id).await.is_ok());
    // Other owners are left alone.
    sqlx::query("UPDATE capture_items SET operation_owner='prelabel:other' WHERE id=?").bind(&id).execute(&pool).await.unwrap();
    assert_eq!(recover_interrupted(&pool).await.unwrap(),0);
    assert!(capture_operation_service::ensure_available(&pool,&id).await.is_err());
    sqlx::query("UPDATE capture_items SET operation_owner=NULL WHERE id=?").bind(&id).execute(&pool).await.unwrap();
}

#[tokio::test]
async fn fixed_preview_does_not_include_new_items_and_retains_archives() {
    let (pool,f,session,id) = fixture().await;
    let job = preview(&pool,selection(&f.project_id)).await.unwrap();
    let later = add(&pool,&f,&session,"later.png").await;
    let result = execute(&pool,&f.output_directory,&f.output_directory,request(&job.id,false),&Recycler::default()).await.unwrap();
    assert_eq!(result.items.len(),1); assert_eq!(result.delete_destination_files,Some(false));
    assert!(f.destination_directory.join("one.png").exists());
    assert_eq!(capture_service::get_item(&pool,&later).await.unwrap().classification,"scene");
    assert_eq!(capture_service::get_item(&pool,&id).await.unwrap().classification,"unclassified");
    // Choices are frozen for the preview and cannot be flipped afterwards.
    assert!(execute(&pool,&f.output_directory,&f.output_directory,request(&job.id,true),&Recycler::default()).await.is_err());
}

#[tokio::test]
async fn missing_archive_is_skipped_and_partial_cleanup_marks_the_state() {
    let (pool,f,_,id) = fixture().await;
    let second = f.destination_directory.join("z-avatar.png"); tokio::fs::write(&second,b"avatar").await.unwrap();
    sqlx::query("UPDATE capture_items SET destination_avatar_path=?,destination_avatar_file_state='available' WHERE id=?").bind(second.to_string_lossy().as_ref()).bind(&id).execute(&pool).await.unwrap();
    let job = preview(&pool,selection(&f.project_id)).await.unwrap();
    let recycler = Recycler::default(); recycler.fail_on.store(2,Ordering::SeqCst);
    let result = execute(&pool,&f.output_directory,&f.output_directory,request(&job.id,true),&recycler).await.unwrap();
    assert_eq!(result.items[0].status,"failed");
    assert_eq!(capture_service::get_item(&pool,&id).await.unwrap().classification,"scene");
    // The already removed archive must not keep reporting as available.
    let state:(String,String) = sqlx::query_as("SELECT destination_file_state,destination_avatar_file_state FROM capture_items WHERE id=?").bind(&id).fetch_one(&pool).await.unwrap();
    assert_eq!(state,("missing".to_owned(),"available".to_owned()));
    // A retry treats the file that is gone as already cleaned and finishes the picture.
    let result = execute(&pool,&f.output_directory,&f.output_directory,request(&job.id,true),&Recycler::default()).await.unwrap();
    assert_eq!(result.items[0].status,"succeeded");
}

#[tokio::test]
async fn skips_missing_changed_and_busy_sources_but_processes_eligible_items() {
    let (pool,f,session,id)=fixture().await;
    let missing=add(&pool,&f,&session,"missing.png").await;
    let changed=add(&pool,&f,&session,"changed.png").await;
    let busy=add(&pool,&f,&session,"busy.png").await;
    tokio::fs::remove_file(f.source_directory.join("missing.png")).await.unwrap();
    tokio::fs::write(f.source_directory.join("changed.png"),b"different").await.unwrap();
    sqlx::query("UPDATE capture_items SET status='processing' WHERE id=?").bind(&busy).execute(&pool).await.unwrap();
    let job=preview(&pool,selection(&f.project_id)).await.unwrap();
    assert_eq!(job.items.iter().filter(|i|i.status=="skipped").count(),3);
    let result=execute(&pool,&f.output_directory,&f.output_directory,request(&job.id,true),&Recycler::default()).await.unwrap();
    assert_eq!(result.items.iter().find(|i|i.capture_item_id==id).unwrap().status,"succeeded");
    for id in [missing,changed,busy] { assert_eq!(capture_service::get_item(&pool,&id).await.unwrap().classification,"scene"); }
}

#[tokio::test]
async fn changed_target_is_not_deleted_or_reported_as_success() {
    let (pool,f,_,id)=fixture().await;
    let job=preview(&pool,selection(&f.project_id)).await.unwrap();
    tokio::fs::write(f.destination_directory.join("one.png"),b"new occupant").await.unwrap();
    let recycler=Recycler::default();
    let result=execute(&pool,&f.output_directory,&f.output_directory,request(&job.id,true),&recycler).await.unwrap();
    assert_eq!(result.items[0].status,"failed"); assert_eq!(recycler.calls.load(Ordering::SeqCst),0);
    assert_eq!(capture_service::get_item(&pool,&id).await.unwrap().classification,"scene");
}

#[tokio::test]
async fn original_alias_is_protected_and_shared_output_is_retained() {
    let (pool,f,session,id)=fixture().await;
    let other=add(&pool,&f,&session,"other.png").await;
    let own=capture_service::get_item(&pool,&id).await.unwrap();
    sqlx::query("UPDATE capture_items SET destination_path=? WHERE id=?").bind(&own.source_path).bind(&id).execute(&pool).await.unwrap();
    let job=preview(&pool,ResetPreviewInput{capture_item_ids:Some(vec![id.clone()]),..selection(&f.project_id)}).await.unwrap();
    let result=execute(&pool,&f.output_directory,&f.output_directory,request(&job.id,true),&Recycler::default()).await.unwrap();
    assert_eq!(result.items[0].status,"failed"); assert!(Path::new(&own.source_path).exists());
    let shared=f.destination_directory.join("other.png");
    // Another capture references the second capture's archive.
    sqlx::query("UPDATE capture_items SET avatar_path=? WHERE id=?").bind(shared.to_string_lossy().as_ref()).bind(&id).execute(&pool).await.unwrap();
    let job=preview(&pool,ResetPreviewInput{capture_item_ids:Some(vec![other]),..selection(&f.project_id)}).await.unwrap();
    let result=execute(&pool,&f.output_directory,&f.output_directory,request(&job.id,true),&Recycler::default()).await.unwrap();
    assert_eq!(result.items[0].status,"succeeded"); assert!(shared.exists());
}

#[tokio::test]
async fn persisted_reset_owner_blocks_label_and_stale_feature_validation() {
    let (pool,f,_,id)=fixture().await;
    let old=capture_operation_service::version(&pool,&id).await.unwrap();
    sqlx::query("UPDATE capture_items SET classification='unclassified',status='awaiting_label',operation_owner='reset:test',processing_version=processing_version+1 WHERE id=?").bind(&id).execute(&pool).await.unwrap();
    assert!(capture_service::label_capture(&pool,LabelCaptureInput{capture_item_id:id.clone(),character_id:None,classification:Some("scene".into())}).await.is_err());
    assert!(capture_operation_service::validate_result(&pool,&id,old).await.is_err());
    assert!(capture_service::next_awaiting_label_without_feature(&pool).await.unwrap().is_none());
    assert!(f.source_directory.join("one.png").exists());
}

#[tokio::test]
async fn alias_scan_resolves_each_reference_path_once_per_run() {
    let (pool, f, session, id) = fixture().await;
    add(&pool, &f, &session, "second.png").await;
    let row = load(&pool, &id).await.unwrap();
    let index = ReferenceIndex::load(&pool, &row).await.unwrap();
    let references: BTreeSet<String> = index.sources.iter().chain(index.shared.iter())
        .chain(index.assets.iter().flat_map(|(path, thumbnail)| std::iter::once(path).chain(thumbnail.iter())))
        .map(|path| key(Path::new(path))).collect();
    assert!(references.len() >= 3, "fixture must offer several references, got {}", references.len());

    let mut cache = CanonicalCache::new();
    let first = f.destination_directory.join("one.png");
    assert_eq!(protection(&index, &mut cache, &first).await.unwrap(), Protection::None);
    assert_eq!(cache.len(), references.len(), "one target must resolve every reference path exactly once");

    let second = f.destination_directory.join("second.png");
    assert_eq!(protection(&index, &mut cache, &second).await.unwrap(), Protection::Shared);
    assert_eq!(cache.len(), references.len(), "a second target must reuse the resolutions of the first");
}

#[tokio::test]
async fn failure_reasons_are_classified_for_the_interface() {
    assert_eq!(reason_of(&conflict("capture is unclassified or busy")), "busy");
    assert_eq!(reason_of(&conflict("reset target aliases a protected source")), "source_gone");
    assert_eq!(reason_of(&conflict("reset file changed since preview: x")), "changed");
    assert_eq!(reason_of(&conflict("UNC deletion requires explicit permanent-delete authorization")), "network");
    assert_eq!(reason_of(&conflict("reset cleanup did not remove the file")), "locked");
    assert_eq!(reason_of(&AppError::Io(std::io::Error::from_raw_os_error(53))), "network");
    assert_eq!(reason_of(&AppError::Io(std::io::Error::from_raw_os_error(32))), "locked");
    assert_eq!(reason_of(&AppError::Io(std::io::Error::from_raw_os_error(5))), "denied");
    assert_eq!(reason_of(&AppError::Io(std::io::Error::from_raw_os_error(2))), "source_gone");
}

#[tokio::test]
async fn closing_a_preview_discards_it_but_a_started_job_survives() {
    let (pool, f, _, _) = fixture().await;
    let previewed = preview(&pool, selection(&f.project_id)).await.unwrap();
    assert!(get(&previewed.id).is_ok());
    discard(&previewed.id).unwrap();
    // No leak: the abandoned preview is gone from the registry.
    assert!(get(&previewed.id).is_err());

    let started = preview(&pool, selection(&f.project_id)).await.unwrap();
    let recycler = Recycler::default();
    recycler.fail_on.store(1, Ordering::SeqCst);
    execute(&pool, &f.output_directory, &f.output_directory, request(&started.id, true), &recycler)
        .await
        .unwrap();
    // A started job keeps its report until it ages out; a stray discard cannot
    // cancel work that already ran.
    discard(&started.id).unwrap();
    assert_eq!(get(&started.id).unwrap().status, "completed");
}

#[test]
fn abandoned_previews_and_finished_jobs_are_pruned() {
    fn job(id: &str, created: Duration, finished: Option<Duration>) -> JobHandle {
        store(JobState {
            id: id.to_owned(),
            delete_destination_files: None,
            allow_permanent_network_delete: None,
            executing: false,
            created: Instant::now() - created,
            finished: finished.map(|age| Instant::now() - age),
            destination_files: BTreeSet::new(),
            network_destination_files: BTreeSet::new(),
            items: Vec::new(),
        })
    }
    let fresh = job("fresh-preview", Duration::from_secs(5), None);
    let stale_preview = job("stale-preview", PREVIEW_TTL + Duration::from_secs(60), None);
    let stale_finished = job("stale-finished", Duration::from_secs(5), Some(JOB_TTL + Duration::from_secs(60)));

    prune(&mut lock(jobs()));

    assert!(lock(&fresh).id == "fresh-preview");
    assert!(get("fresh-preview").is_ok(), "a preview inside its window must survive");
    assert!(get("stale-preview").is_err(), "an abandoned preview must be reclaimed");
    assert!(get("stale-finished").is_err(), "a finished job ages out as before");
    let _ = stale_preview;
    let _ = stale_finished;
}

#[test]
fn the_registry_cap_evicts_inactive_jobs_only() {
    fn preview_job(id: String, age_secs: u64, executing: bool) -> JobState {
        JobState {
            id,
            delete_destination_files: None,
            allow_permanent_network_delete: None,
            executing,
            created: Instant::now() - Duration::from_secs(age_secs),
            finished: None,
            destination_files: BTreeSet::new(),
            network_destination_files: BTreeSet::new(),
            items: Vec::new(),
        }
    }
    // The oldest job is running: the cap has to skip it and evict an idle one.
    store(preview_job("running-oldest".to_owned(), 10_000, true));
    for index in 0..MAX_JOBS + 4 {
        // cap-0 is the oldest idle preview, the last one inserted is the newest.
        let age = (MAX_JOBS + 4 - index) as u64;
        store(preview_job(format!("cap-{index}"), age, false));
    }
    assert!(get("running-oldest").is_ok(), "an executing job must never be evicted");
    assert!(jobs().lock().unwrap_or_else(|error| error.into_inner()).len() <= MAX_JOBS);
    // The newest preview always survives its own insertion.
    assert!(get(&format!("cap-{}", MAX_JOBS + 3)).is_ok());
    // Eviction took idle jobs instead: the oldest preview went first.
    assert!(get("cap-0").is_err(), "the oldest idle preview should have been evicted");
    discard("running-oldest").unwrap();
    assert!(get("running-oldest").is_ok(), "a running job ignores discard too");
}

#[tokio::test]
async fn a_relocated_source_no_longer_blocks_the_retry() {
    let (pool, f, _, id) = fixture().await;
    let job = preview(&pool, selection(&f.project_id)).await.unwrap();
    let recycler = Recycler::default();
    recycler.fail_on.store(1, Ordering::SeqCst);
    let result = execute(&pool, &f.output_directory, &f.output_directory, request(&job.id, true), &recycler)
        .await
        .unwrap();
    assert_eq!(result.items[0].status, "failed");
    // The same content is moved and reconciliation records the new location.
    let moved = f.source_directory.join("moved-one.png");
    tokio::fs::rename(f.source_directory.join("one.png"), &moved).await.unwrap();
    sqlx::query("UPDATE capture_items SET source_path = ?, updated_at = '2026-09-15T00:00:00Z' WHERE id = ?")
        .bind(moved.to_string_lossy().as_ref())
        .bind(&id)
        .execute(&pool)
        .await
        .unwrap();

    let result = execute(&pool, &f.output_directory, &f.output_directory, request(&job.id, true), &Recycler::default())
        .await
        .unwrap();
    assert_eq!(result.items[0].status, "succeeded");
    assert_eq!(capture_service::get_item(&pool, &id).await.unwrap().classification, "unclassified");
    assert!(moved.exists(), "the relocated original must stay untouched");
}
