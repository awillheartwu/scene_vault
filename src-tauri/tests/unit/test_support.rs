//! Shared test scaffolding: a project with one configured source directory
//! and archive destination, ready for capture-session tests.

use std::path::PathBuf;

use sqlx::SqlitePool;
use tempfile::TempDir;

use crate::error::AppError;
use crate::models::capture::{CaptureSession, StartCaptureSessionInput};
use crate::models::project::{
    AddProjectSourceDirectoryInput, CreateProjectInput, SetProjectDestinationInput,
};
use crate::services::{capture_service, project_service};

pub struct ProjectFixture {
    pub _workspace: TempDir,
    pub project_id: String,
    pub source_directory: PathBuf,
    pub destination_directory: PathBuf,
    pub output_directory: PathBuf,
}

pub async fn project_with_directories(
    pool: &SqlitePool,
    name: &str,
) -> Result<ProjectFixture, AppError> {
    let workspace =
        TempDir::new().map_err(|error| AppError::Io(std::io::Error::other(error.to_string())))?;
    let source_directory = workspace.path().join("source");
    let destination_directory = workspace.path().join("archive");
    let output_directory = workspace.path().join("output");
    std::fs::create_dir_all(&source_directory)?;
    std::fs::create_dir_all(&destination_directory)?;
    std::fs::create_dir_all(&output_directory)?;

    let project = project_service::create(
        pool,
        CreateProjectInput {
            name: name.to_owned(),
            description: None,
            cover_asset_id: None,
        },
    )
    .await?;
    project_service::set_destination_directory(
        pool,
        SetProjectDestinationInput {
            project_id: project.id.clone(),
            directory: capture_service::path_to_string(&destination_directory),
        },
    )
    .await?;
    project_service::add_source_directory(
        pool,
        AddProjectSourceDirectoryInput {
            project_id: project.id.clone(),
            directory: capture_service::path_to_string(&source_directory),
        },
    )
    .await?;

    Ok(ProjectFixture {
        _workspace: workspace,
        project_id: project.id,
        source_directory,
        destination_directory,
        output_directory,
    })
}

pub async fn start_session(
    pool: &SqlitePool,
    project_id: &str,
) -> Result<CaptureSession, AppError> {
    capture_service::start_session(
        pool,
        StartCaptureSessionInput {
            project_id: project_id.to_owned(),
        },
    )
    .await
    .map(|result| result.session)
}
