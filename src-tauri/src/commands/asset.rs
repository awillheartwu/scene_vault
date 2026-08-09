use tauri::State;

use crate::{
    db::AppState,
    error::AppError,
    models::asset::{Asset, CreateAssetInput},
    services::asset_service,
};

#[tauri::command]
pub async fn create_asset(
    state: State<'_, AppState>,
    input: CreateAssetInput,
) -> Result<Asset, AppError> {
    asset_service::create(&state.pool, input).await
}
