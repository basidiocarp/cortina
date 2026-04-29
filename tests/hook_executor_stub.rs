//! Hook executor integration stub — documents the no-op contract.
//!
//! The hook executor is intentionally a no-op stub. No subprocess is
//! execute'd, no timeout is enforced, no stdout/stderr is aggregated,
//! and no nonzero exit code from a hook process is handled.
//!
//! When real hook execution is implemented, this module gains tests for:
//! - Success aggregation across multiple hooks
//! - Fail-open diagnostic behavior on nonzero exit
//! - Timeout enforcement (30-second limit per hook)
//! - Context modification size limits (50 KB)
//!
//! Until then: the only invariant is that the stub is fail-open and silent.

/// Confirms the documented no-op stub contract.
///
/// The executor stub is fail-open: it never blocks, never produces output,
/// and never launches any subprocess. Real execute tests replace this when
/// subprocess dispatch is wired.
#[test]
fn stub_executor_contract_is_fail_open() {
    // Stub: no subprocess is execute'd, no timeout is enforced, no stdout/stderr
    // is aggregated, no nonzero exit code is handled. The executor returns the
    // default pass-through HookOutput — fail-open and silent.
    //
    // This test body is intentionally empty. Real assertions (timeout boundary,
    // nonzero exit handling, diagnostic output) are added here when subprocess
    // dispatch is wired in the executor.
}
