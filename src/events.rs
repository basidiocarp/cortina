mod adapter_events;
mod normalized_lifecycle;
mod outcome_events;

#[allow(
    unused_imports,
    reason = "Re-exported event types are used by tests, adapter modules, and downstream contract work"
)]
pub use adapter_events::{
    BashToolEvent, CommandRewriteRequest, FileEditEvent, PreCompactEvent, SessionStopEvent,
    ToolResultEvent, UserPromptSubmitEvent, VolvaBackendKind, VolvaHookEvent, VolvaHookPhase,
};
#[allow(
    unused_imports,
    reason = "Re-exported lifecycle vocabulary is part of the shared event surface"
)]
pub use normalized_lifecycle::{
    LifecycleCategory, LifecycleHost, LifecycleStatus, NORMALIZED_LIFECYCLE_EVENT_SCHEMA_VERSION,
    NormalizedLifecycleEvent, is_council_prompt,
};
pub use outcome_events::{OutcomeEvent, OutcomeKind};
