//! Per-capture serialization for prelabel work, manual edits and reset.
use std::{collections::HashMap, sync::{Arc, Mutex, OnceLock, Weak}};
use sqlx::SqlitePool;
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};
use crate::error::AppError;

pub async fn lock(_pool: &SqlitePool, id: &str) -> OwnedMutexGuard<()> {
    static LOCKS: OnceLock<Mutex<HashMap<String, Weak<AsyncMutex<()>>>>> = OnceLock::new();
    let mutex = {
        let mut locks = LOCKS.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
        locks.retain(|_, value| value.strong_count() > 0);
        match locks.get(id).and_then(Weak::upgrade) {
            Some(lock) => lock,
            None => {
                let lock = Arc::new(AsyncMutex::new(()));
                locks.insert(id.to_owned(), Arc::downgrade(&lock));
                lock
            }
        }
    };
    mutex.lock_owned().await
}

pub async fn ensure_available(pool: &SqlitePool, id: &str) -> Result<(), AppError> {
    let owner: Option<String> = sqlx::query_scalar("SELECT operation_owner FROM capture_items WHERE id = ?")
        .bind(id).fetch_one(pool).await?;
    if owner.is_some() {
        return Err(AppError::Conflict("图片正在撤销分类或等待撤销重试，请先完成该操作".to_owned()));
    }
    Ok(())
}

pub async fn version(pool: &SqlitePool, id: &str) -> Result<i64, AppError> {
    Ok(sqlx::query_scalar("SELECT processing_version FROM capture_items WHERE id = ?")
        .bind(id).fetch_one(pool).await?)
}

pub async fn validate_result(pool: &SqlitePool, id: &str, expected: i64) -> Result<(), AppError> {
    ensure_available(pool, id).await?;
    if version(pool, id).await? != expected {
        return Err(AppError::Conflict("识别结果已经过期，请重新识别".to_owned()));
    }
    let item = super::capture_service::get_item(pool, id).await?;
    super::capture_service::validate_source_identity(pool, &item).await?;
    Ok(())
}
