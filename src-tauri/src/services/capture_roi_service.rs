use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use crate::{error::AppError, models::capture::CaptureItem};
use super::{capture_operation_service as operation, capture_service, recognition_service, vision_settings_service};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FaceRoi { pub x: f64, pub y: f64, pub width: f64, pub height: f64 }

impl FaceRoi {
    pub fn validate(&self) -> Result<(), AppError> {
        if ![self.x, self.y, self.width, self.height].iter().all(|v| v.is_finite())
            || self.x < 0.0 || self.y < 0.0 || self.width <= 0.0 || self.height <= 0.0
            || self.x + self.width > 1.0 || self.y + self.height > 1.0 {
            return Err(AppError::Validation("选区必须位于图片内且宽高大于零".to_owned()));
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetFaceRoiInput { pub capture_item_id: String, pub face_roi: Option<FaceRoi> }

pub async fn get(pool: &SqlitePool, id: &str) -> Result<Option<FaceRoi>, AppError> {
    let raw: Option<String> = sqlx::query_scalar("SELECT manual_face_roi_json FROM capture_items WHERE id = ?")
        .bind(id).fetch_one(pool).await?;
    raw.map(|s| serde_json::from_str(&s).map_err(|e| AppError::Validation(e.to_string()))).transpose()
}

pub async fn set(pool: &SqlitePool, input: SetFaceRoiInput) -> Result<CaptureItem, AppError> {
    if let Some(roi) = &input.face_roi { roi.validate()?; }
    let id = &input.capture_item_id;
    let _guard = operation::lock(pool, id).await;
    operation::ensure_available(pool, id).await?;
    let item = capture_service::get_item(pool, id).await?;
    if item.status != "awaiting_label" || item.classification != "unclassified" {
        return Err(AppError::Conflict("请先撤销分类，再框选主脸".to_owned()));
    }
    capture_service::validate_source_identity(pool, &item).await?;
    let raw = input.face_roi.as_ref().map(serde_json::to_string).transpose()
        .map_err(|e| AppError::Validation(e.to_string()))?;
    let mut tx = pool.begin().await?;
    sqlx::query("UPDATE capture_items SET manual_face_roi_json=?, manual_face_roi_ready=0, processing_version=processing_version+1, recognition_deferred=1, face_box_json=NULL, face_count=NULL, face_feature_json=NULL, suggested_character_id=NULL, recognition_confidence=NULL, recognition_source=NULL, review_status='none', updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=?")
        .bind(raw).bind(id).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM capture_faces WHERE capture_item_id=?").bind(id).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM character_face_samples WHERE capture_item_id=?").bind(id).execute(&mut *tx).await?;
    tx.commit().await?;
    if input.face_roi.is_none() && !vision_settings_service::is_engine_available(&vision_settings_service::get(pool).await?) {
        return capture_service::get_item(pool, id).await;
    }
    let result = recognition_service::refresh_capture_face_feature_locked(pool, id, |_, _| {}).await;
    if result.is_ok() {
        sqlx::query("UPDATE capture_items SET manual_face_roi_ready=1 WHERE id=?").bind(id).execute(pool).await?;
    }
    match result { Ok(_) => capture_service::get_item(pool, id).await, Err(error) => Err(error) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_invalid_rectangles() {
        for roi in [FaceRoi{x:-0.1,y:0.0,width:0.2,height:0.2}, FaceRoi{x:0.9,y:0.0,width:0.2,height:0.2}, FaceRoi{x:0.0,y:0.0,width:0.0,height:0.2}, FaceRoi{x:f64::NAN,y:0.0,width:0.2,height:0.2}] { assert!(roi.validate().is_err()); }
        assert!(FaceRoi{x:0.0,y:0.0,width:1.0,height:1.0}.validate().is_ok());
    }
}
