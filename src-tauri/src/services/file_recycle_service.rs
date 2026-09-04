use std::path::Path;

use crate::error::AppError;

pub trait FileRecycler: Send + Sync {
    async fn recycle(&self, path: &Path) -> Result<(), AppError>;

    async fn delete_permanently(&self, path: &Path) -> Result<(), AppError> {
        tokio::fs::remove_file(path).await?;
        Ok(())
    }
}

pub struct SystemRecycleBin;

impl FileRecycler for SystemRecycleBin {
    async fn recycle(&self, path: &Path) -> Result<(), AppError> {
        recycle(path).await
    }
}

/// Moves one local file to the platform recycle bin. Network targets require
/// a separate, explicit permanent-delete authorization at the service layer.
pub async fn recycle(path: &Path) -> Result<(), AppError> {
    if is_unc_path(path) {
        return Err(AppError::Conflict(
            "UNC and NAS targets do not support guaranteed recoverable deletion".to_owned(),
        ));
    }
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || recycle_blocking(&path))
        .await
        .map_err(|error| AppError::Io(std::io::Error::other(error.to_string())))?
}

#[cfg(windows)]
pub(crate) fn is_unc_path(path: &Path) -> bool {
    use std::path::{Component, Prefix};

    matches!(
        path.components().next(),
        Some(Component::Prefix(prefix))
            if matches!(prefix.kind(), Prefix::UNC(_, _) | Prefix::VerbatimUNC(_, _))
    )
}

#[cfg(not(windows))]
pub(crate) fn is_unc_path(path: &Path) -> bool {
    path.as_os_str().to_string_lossy().starts_with("\\\\")
}

#[cfg(not(windows))]
fn recycle_blocking(_path: &Path) -> Result<(), AppError> {
    Err(AppError::Validation(
        "the recycle bin is only available in the Windows build".to_owned(),
    ))
}

#[cfg(windows)]
fn recycle_blocking(path: &Path) -> Result<(), AppError> {
    use std::{
        os::windows::ffi::OsStrExt,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
    };
    use windows::{
        core::{implement, Error as WindowsError, Result as WindowsResult, HRESULT, PCWSTR},
        Win32::{
            System::Com::{
                CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL,
                COINIT_APARTMENTTHREADED,
            },
            UI::Shell::{
                FileOperation, IFileOperation, IFileOperationProgressSink,
                IFileOperationProgressSink_Impl, IShellItem, SHCreateItemFromParsingName,
                FOFX_RECYCLEONDELETE, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT,
                TSF_DELETE_RECYCLE_IF_POSSIBLE,
            },
        },
    };
    use windows_core::Ref;

    #[implement(IFileOperationProgressSink)]
    struct RecycleOnlySink {
        denied: Arc<AtomicBool>,
    }

    impl IFileOperationProgressSink_Impl for RecycleOnlySink_Impl {
        fn StartOperations(&self) -> WindowsResult<()> {
            Ok(())
        }
        fn FinishOperations(&self, _result: HRESULT) -> WindowsResult<()> {
            Ok(())
        }
        fn PreRenameItem(&self, _: u32, _: Ref<'_, IShellItem>, _: &PCWSTR) -> WindowsResult<()> {
            Ok(())
        }
        fn PostRenameItem(
            &self,
            _: u32,
            _: Ref<'_, IShellItem>,
            _: &PCWSTR,
            _: HRESULT,
            _: Ref<'_, IShellItem>,
        ) -> WindowsResult<()> {
            Ok(())
        }
        fn PreMoveItem(
            &self,
            _: u32,
            _: Ref<'_, IShellItem>,
            _: Ref<'_, IShellItem>,
            _: &PCWSTR,
        ) -> WindowsResult<()> {
            Ok(())
        }
        fn PostMoveItem(
            &self,
            _: u32,
            _: Ref<'_, IShellItem>,
            _: Ref<'_, IShellItem>,
            _: &PCWSTR,
            _: HRESULT,
            _: Ref<'_, IShellItem>,
        ) -> WindowsResult<()> {
            Ok(())
        }
        fn PreCopyItem(
            &self,
            _: u32,
            _: Ref<'_, IShellItem>,
            _: Ref<'_, IShellItem>,
            _: &PCWSTR,
        ) -> WindowsResult<()> {
            Ok(())
        }
        fn PostCopyItem(
            &self,
            _: u32,
            _: Ref<'_, IShellItem>,
            _: Ref<'_, IShellItem>,
            _: &PCWSTR,
            _: HRESULT,
            _: Ref<'_, IShellItem>,
        ) -> WindowsResult<()> {
            Ok(())
        }
        fn PreDeleteItem(&self, flags: u32, _: Ref<'_, IShellItem>) -> WindowsResult<()> {
            if flags & TSF_DELETE_RECYCLE_IF_POSSIBLE.0 as u32 == 0 {
                self.denied.store(true, Ordering::SeqCst);
                return Err(WindowsError::from_hresult(HRESULT(0x80004004_u32 as i32)));
            }
            Ok(())
        }
        fn PostDeleteItem(
            &self,
            _: u32,
            _: Ref<'_, IShellItem>,
            result: HRESULT,
            _: Ref<'_, IShellItem>,
        ) -> WindowsResult<()> {
            result.ok()
        }
        fn PreNewItem(&self, _: u32, _: Ref<'_, IShellItem>, _: &PCWSTR) -> WindowsResult<()> {
            Ok(())
        }
        fn PostNewItem(
            &self,
            _: u32,
            _: Ref<'_, IShellItem>,
            _: &PCWSTR,
            _: &PCWSTR,
            _: u32,
            _: HRESULT,
            _: Ref<'_, IShellItem>,
        ) -> WindowsResult<()> {
            Ok(())
        }
        fn UpdateProgress(&self, _: u32, _: u32) -> WindowsResult<()> {
            Ok(())
        }
        fn ResetTimer(&self) -> WindowsResult<()> {
            Ok(())
        }
        fn PauseTimer(&self) -> WindowsResult<()> {
            Ok(())
        }
        fn ResumeTimer(&self) -> WindowsResult<()> {
            Ok(())
        }
    }

    struct ComGuard;
    impl Drop for ComGuard {
        fn drop(&mut self) {
            unsafe { CoUninitialize() };
        }
    }

    let denied = Arc::new(AtomicBool::new(false));
    let result = unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED)
            .ok()
            .map_err(|error| AppError::Io(std::io::Error::other(error.to_string())))?;
        let _guard = ComGuard;
        let operation: IFileOperation = CoCreateInstance(&FileOperation, None, CLSCTX_ALL)
            .map_err(|error| AppError::Io(std::io::Error::other(error.to_string())))?;
        operation
            .SetOperationFlags(
                FOF_ALLOWUNDO
                    | FOF_NOCONFIRMATION
                    | FOF_SILENT
                    | FOF_NOERRORUI
                    | FOFX_RECYCLEONDELETE,
            )
            .map_err(|error| AppError::Io(std::io::Error::other(error.to_string())))?;
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let item: IShellItem = SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None)
            .map_err(|error| AppError::Io(std::io::Error::other(error.to_string())))?;
        let sink: IFileOperationProgressSink = RecycleOnlySink {
            denied: Arc::clone(&denied),
        }
        .into();
        operation
            .DeleteItem(&item, &sink)
            .map_err(|error| AppError::Io(std::io::Error::other(error.to_string())))?;
        operation
            .PerformOperations()
            .map_err(|error| AppError::Io(std::io::Error::other(error.to_string())))?;
        if operation
            .GetAnyOperationsAborted()
            .map_err(|error| AppError::Io(std::io::Error::other(error.to_string())))?
            .as_bool()
        {
            Err(AppError::Conflict(
                "recycle operation was cancelled".to_owned(),
            ))
        } else {
            Ok(())
        }
    };
    if denied.load(Ordering::SeqCst) {
        return Err(AppError::Conflict(
            "this target location does not support recoverable deletion".to_owned(),
        ));
    }
    result
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn windows_recycles_a_test_file() {
        let directory =
            std::env::temp_dir().join(format!("scene-vault-recycle-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).expect("create test directory");
        let file = directory.join("target.png");
        std::fs::write(&file, b"recycle-bin integration test").expect("write test file");

        recycle(&file).await.expect("move test file to recycle bin");

        assert!(!file.exists(), "the recycled path must no longer exist");
        std::fs::remove_dir(&directory).expect("remove empty test directory");
    }

    #[tokio::test]
    async fn windows_rejects_unc_before_shell_deletion() {
        let error = recycle(Path::new(r"\\server\share\target.png"))
            .await
            .expect_err("UNC must fail closed");
        assert!(matches!(error, AppError::Conflict(_)));
    }
}
