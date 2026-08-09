use sqlx::SqlitePool;
use uuid::Uuid;

use crate::{
    error::AppError,
    models::asset::{Asset, CreateAssetInput},
};

const SUPPORTED_ASSET_TYPES: [&str; 4] = ["image", "video", "audio", "text"];

pub async fn create(pool: &SqlitePool, input: CreateAssetInput) -> Result<Asset, AppError> {
    let name = input.name.trim();
    let path = input.path.trim();
    let asset_type = input.asset_type.trim().to_ascii_lowercase();

    if name.is_empty() {
        return Err(AppError::Validation(
            "asset name cannot be empty".to_owned(),
        ));
    }
    if path.is_empty() {
        return Err(AppError::Validation(
            "asset path cannot be empty".to_owned(),
        ));
    }
    if !SUPPORTED_ASSET_TYPES.contains(&asset_type.as_str()) {
        return Err(AppError::Validation(format!(
            "unsupported asset type: {asset_type}"
        )));
    }
    if input.file_size.is_some_and(|size| size < 0) {
        return Err(AppError::Validation(
            "asset file size cannot be negative".to_owned(),
        ));
    }

    let metadata_json = input.metadata_json.unwrap_or_else(|| "{}".to_owned());
    serde_json::from_str::<serde_json::Value>(&metadata_json)
        .map_err(|error| AppError::Validation(format!("invalid metadata JSON: {error}")))?;

    let asset = sqlx::query_as::<_, Asset>(
        r#"
        INSERT INTO assets (
            id, name, asset_type, path, mime_type, file_size,
            file_modified_at, metadata_json
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?)
        RETURNING
            id, name, asset_type, path, thumbnail_path, mime_type, file_size,
            file_modified_at, content_hash, metadata_json, ai_description,
            embedding_id, status, created_at, updated_at
        "#,
    )
    .bind(Uuid::new_v4().to_string())
    .bind(name)
    .bind(asset_type)
    .bind(path)
    .bind(input.mime_type)
    .bind(input.file_size)
    .bind(input.file_modified_at)
    .bind(metadata_json)
    .fetch_one(pool)
    .await?;

    Ok(asset)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    #[tokio::test]
    async fn creates_an_asset() {
        let pool = db::test_pool().await;
        let asset = create(
            &pool,
            CreateAssetInput {
                name: "Rainy street".to_owned(),
                asset_type: "IMAGE".to_owned(),
                path: "D:/library/rain.png".to_owned(),
                mime_type: Some("image/png".to_owned()),
                file_size: Some(1024),
                file_modified_at: None,
                metadata_json: None,
            },
        )
        .await
        .expect("create asset");

        assert_eq!(asset.asset_type, "image");
        assert_eq!(asset.metadata_json, "{}");
        assert_eq!(asset.status, "available");
    }
}
