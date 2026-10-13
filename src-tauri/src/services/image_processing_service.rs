//! Shared thread budget for CPU-heavy Rust image work and Python requests.
//! A private Rayon pool keeps codec parallelism independent of the process-wide
//! RAYON_NUM_THREADS environment variable and follows settings on the next job.

use std::sync::{
    atomic::{AtomicU32, Ordering},
    Arc, Mutex,
};

use crate::{
    error::AppError,
    models::app_settings::{
        DEFAULT_IMAGE_PROCESSING_CORE_LIMIT, MAX_IMAGE_PROCESSING_CORE_LIMIT,
        MIN_IMAGE_PROCESSING_CORE_LIMIT,
    },
};

static CORE_LIMIT: AtomicU32 = AtomicU32::new(DEFAULT_IMAGE_PROCESSING_CORE_LIMIT);
static POOL: Mutex<Option<(u32, Arc<rayon::ThreadPool>)>> = Mutex::new(None);

pub fn core_limit() -> u32 {
    CORE_LIMIT.load(Ordering::SeqCst)
}

pub fn set_core_limit(limit: u32) {
    CORE_LIMIT.store(
        limit.clamp(
            MIN_IMAGE_PROCESSING_CORE_LIMIT,
            MAX_IMAGE_PROCESSING_CORE_LIMIT,
        ),
        Ordering::SeqCst,
    );
}

/// All Rust image jobs share one pool for the current setting, rather than
/// creating a pool per thumbnail. Jobs already running may finish on the old
/// pool after a setting change; they retain ownership of that pool until done.
pub fn run<T: Send>(job: impl FnOnce() -> Result<T, AppError> + Send) -> Result<T, AppError> {
    run_with_limit(core_limit(), job)
}

fn run_with_limit<T: Send>(
    limit: u32,
    job: impl FnOnce() -> Result<T, AppError> + Send,
) -> Result<T, AppError> {
    let pool = {
        let mut cached = POOL.lock().unwrap_or_else(|error| error.into_inner());
        match cached.as_ref() {
            Some((configured, pool)) if *configured == limit => Arc::clone(pool),
            _ => {
                let pool = Arc::new(
                    rayon::ThreadPoolBuilder::new()
                        .num_threads(limit as usize)
                        .thread_name(|index| format!("image-processing-{index}"))
                        .build()
                        .map_err(|error| {
                            AppError::Image(format!(
                                "cannot start image processing threads: {error}"
                            ))
                        })?,
                );
                *cached = Some((limit, Arc::clone(&pool)));
                pool
            }
        }
    };
    pool.install(job)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_jobs_use_the_configured_budget_and_propagate_image_failures() {
        assert_eq!(
            run(|| Ok(rayon::current_num_threads())).unwrap(),
            core_limit() as usize
        );
        // Exercise setting changes without changing the process-wide limit
        // concurrently observed by the Python worker tests.
        for limit in [8, 1] {
            let actual = run_with_limit(limit, || Ok(rayon::current_num_threads())).unwrap();
            assert_eq!(actual, limit as usize);
        }
        let error = run::<()>(|| Err(AppError::Image("invalid image".to_owned()))).unwrap_err();
        assert!(matches!(error, AppError::Image(message) if message == "invalid image"));
    }
}
