use std::time::Instant;

/// Millisecond wall-clock delta for diagnostics.
pub(crate) fn elapsed_ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1_000.0
}
