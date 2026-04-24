//! Canonical lifecycle pipeline for cortina hook events.
//!
//! Naming the stages explicitly enables composable handlers, testable
//! pipelines, and clear extension points for future tools (hyphae storage,
//! mycelium compression) without requiring changes to cortina's core logic.
//!
//! Event flow: stdin → adapter → `ClaudeCodeEventCommand` → pipeline stages →
//! per-hook handler. The pipeline observes and emits; existing handler logic
//! is unchanged.

use std::fmt;

/// Named stages in the cortina lifecycle pipeline.
/// Each stage has semantic meaning — handlers register at a specific stage
/// and are called only when that stage fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PipelineStage {
    ToolCallReceived,
    #[allow(dead_code)]
    ToolCallValidated,
    #[allow(dead_code)]
    ToolCallDispatched,
    ToolCallCompleted,
    #[allow(dead_code)]
    OutputCaptured,
    #[allow(dead_code)]
    OutputFiltered,
    #[allow(dead_code)]
    OutputStored,
    SessionSignalEmitted,
}

impl fmt::Display for PipelineStage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PipelineStage::ToolCallReceived => write!(f, "ToolCallReceived"),
            PipelineStage::ToolCallValidated => write!(f, "ToolCallValidated"),
            PipelineStage::ToolCallDispatched => write!(f, "ToolCallDispatched"),
            PipelineStage::ToolCallCompleted => write!(f, "ToolCallCompleted"),
            PipelineStage::OutputCaptured => write!(f, "OutputCaptured"),
            PipelineStage::OutputFiltered => write!(f, "OutputFiltered"),
            PipelineStage::OutputStored => write!(f, "OutputStored"),
            PipelineStage::SessionSignalEmitted => write!(f, "SessionSignalEmitted"),
        }
    }
}

/// Context passed to every handler at a given stage.
pub struct PipelineContext<'a> {
    pub stage: PipelineStage,
    pub tool_name: Option<&'a str>,
    pub agent_id: Option<&'a str>,
    #[allow(dead_code)]
    pub payload: &'a serde_json::Value,
}

/// A handler registered at one or more pipeline stages.
/// Returning an error logs a warning and continues (fail-open).
pub trait StageHandler: Send + Sync {
    fn stage(&self) -> PipelineStage;
    fn handle(&self, ctx: &PipelineContext<'_>) -> Result<(), Box<dyn std::error::Error>>;
}

/// Pipeline runner. Handlers are called in registration order for each stage.
/// A handler error never stops subsequent handlers (fail-open invariant).
pub struct Pipeline {
    handlers: Vec<Box<dyn StageHandler>>,
}

impl Pipeline {
    pub fn new() -> Self {
        Pipeline {
            handlers: Vec::new(),
        }
    }

    pub fn register(&mut self, handler: Box<dyn StageHandler>) {
        self.handlers.push(handler);
    }

    /// Run all handlers registered for `ctx.stage`, in registration order.
    /// Errors are logged as warnings; the pipeline always continues.
    pub fn run(&self, ctx: &PipelineContext<'_>) {
        for handler in &self.handlers {
            if handler.stage() == ctx.stage {
                if let Err(e) = handler.handle(ctx) {
                    tracing::warn!(
                        stage = %ctx.stage,
                        error = %e,
                        "pipeline handler failed (continuing)"
                    );
                }
            }
        }
    }
}

impl Default for Pipeline {
    fn default() -> Self {
        Self::new()
    }
}

/// Concrete handler that emits a `tracing::debug!` for each stage it's registered at.
/// Provides a free observability layer for operators who enable DEBUG logging.
pub struct LoggingHandler {
    stage: PipelineStage,
}

impl LoggingHandler {
    pub fn new(stage: PipelineStage) -> Self {
        LoggingHandler { stage }
    }
}

impl StageHandler for LoggingHandler {
    fn stage(&self) -> PipelineStage {
        self.stage
    }

    fn handle(&self, ctx: &PipelineContext<'_>) -> Result<(), Box<dyn std::error::Error>> {
        tracing::debug!(
            stage = %ctx.stage,
            tool = ?ctx.tool_name,
            agent = ?ctx.agent_id,
            "pipeline stage"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    struct CountingHandler {
        stage: PipelineStage,
        count: Arc<AtomicU32>,
    }

    impl CountingHandler {
        fn new(stage: PipelineStage, count: Arc<AtomicU32>) -> Self {
            CountingHandler { stage, count }
        }
    }

    impl StageHandler for CountingHandler {
        fn stage(&self) -> PipelineStage {
            self.stage
        }
        fn handle(&self, _ctx: &PipelineContext<'_>) -> Result<(), Box<dyn std::error::Error>> {
            self.count.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }

    struct FailingHandler {
        stage: PipelineStage,
    }

    impl StageHandler for FailingHandler {
        fn stage(&self) -> PipelineStage {
            self.stage
        }
        fn handle(&self, _ctx: &PipelineContext<'_>) -> Result<(), Box<dyn std::error::Error>> {
            Err("handler error".into())
        }
    }

    fn dummy_payload() -> Value {
        Value::Null
    }

    #[test]
    fn handlers_called_in_registration_order() {
        use std::sync::Mutex;
        struct OrderHandler {
            stage: PipelineStage,
            id: u32,
            log: Arc<Mutex<Vec<u32>>>,
        }

        impl StageHandler for OrderHandler {
            fn stage(&self) -> PipelineStage {
                self.stage
            }
            fn handle(&self, _ctx: &PipelineContext<'_>) -> Result<(), Box<dyn std::error::Error>> {
                self.log.lock().unwrap().push(self.id);
                Ok(())
            }
        }

        let log = Arc::new(Mutex::new(Vec::new()));
        let mut p = Pipeline::new();
        p.register(Box::new(OrderHandler {
            stage: PipelineStage::ToolCallReceived,
            id: 1,
            log: log.clone(),
        }));
        p.register(Box::new(OrderHandler {
            stage: PipelineStage::ToolCallReceived,
            id: 2,
            log: log.clone(),
        }));
        let payload = dummy_payload();
        p.run(&PipelineContext {
            stage: PipelineStage::ToolCallReceived,
            tool_name: None,
            agent_id: None,
            payload: &payload,
        });
        assert_eq!(*log.lock().unwrap(), vec![1, 2]);
    }

    #[test]
    fn fail_open_continues_after_error() {
        struct AfterFailHandler {
            stage: PipelineStage,
            called: Arc<std::sync::atomic::AtomicBool>,
        }

        impl StageHandler for AfterFailHandler {
            fn stage(&self) -> PipelineStage {
                self.stage
            }
            fn handle(&self, _ctx: &PipelineContext<'_>) -> Result<(), Box<dyn std::error::Error>> {
                self.called.store(true, Ordering::Relaxed);
                Ok(())
            }
        }

        let mut p = Pipeline::new();
        p.register(Box::new(FailingHandler {
            stage: PipelineStage::ToolCallReceived,
        }));
        let called = Arc::new(std::sync::atomic::AtomicBool::new(false));
        p.register(Box::new(AfterFailHandler {
            stage: PipelineStage::ToolCallReceived,
            called: called.clone(),
        }));
        let payload = dummy_payload();
        p.run(&PipelineContext {
            stage: PipelineStage::ToolCallReceived,
            tool_name: None,
            agent_id: None,
            payload: &payload,
        });
        assert!(
            called.load(Ordering::Relaxed),
            "handler after failing one must still run"
        );
    }

    #[test]
    fn handlers_only_called_for_their_stage() {
        let count = Arc::new(AtomicU32::new(0));
        let mut p = Pipeline::new();
        p.register(Box::new(CountingHandler::new(
            PipelineStage::ToolCallReceived,
            count.clone(),
        )));

        let payload = dummy_payload();

        // Fire ToolCallCompleted — should NOT call the ToolCallReceived handler
        p.run(&PipelineContext {
            stage: PipelineStage::ToolCallCompleted,
            tool_name: None,
            agent_id: None,
            payload: &payload,
        });
        assert_eq!(
            count.load(Ordering::Relaxed),
            0,
            "handler must not fire for wrong stage"
        );

        // Fire ToolCallReceived — must call it
        p.run(&PipelineContext {
            stage: PipelineStage::ToolCallReceived,
            tool_name: None,
            agent_id: None,
            payload: &payload,
        });
        assert_eq!(
            count.load(Ordering::Relaxed),
            1,
            "handler must fire for correct stage"
        );
    }
}
