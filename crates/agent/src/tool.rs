use std::future::Future;
use std::sync::{Arc, Mutex};

use futures::future::{BoxFuture, Shared};
use futures::FutureExt;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::agent_loop::ToolExecutionMode;
use crate::provider::ToolSpec;
use crate::types::ContentPart;

/// Final or partial output of a tool.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolResult {
    /// What the model sees.
    pub content: Vec<ContentPart>,
    /// Structured details for logs or UI rendering.
    #[serde(default)]
    pub details: Value,
    /// Machine-readable output matching the tool's `output_schema`. A codemode script receives
    /// it instead of the text; the model never sees it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured: Option<Value>,
    /// The call failed. A tool returns an error result instead of an `Err` when the failure has
    /// content or structured output to go with it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_error: bool,
    /// Hint that the agent should stop after this batch. Only honored when every tool in the
    /// batch sets it.
    #[serde(default)]
    pub terminate: bool,
}

impl ToolResult {
    pub fn text(text: impl Into<String>) -> Self {
        ToolResult { content: vec![ContentPart::text(text)], ..ToolResult::default() }
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.details = details;
        self
    }

    pub fn terminating(mut self) -> Self {
        self.terminate = true;
        self
    }

    pub fn text_content(&self) -> String {
        self.content.iter().filter_map(ContentPart::as_text).collect::<Vec<_>>().join("\n")
    }
}

/// How a call that a tool made through a [`ToolRunner`] came out.
#[derive(Debug, Clone, Default)]
pub struct ToolOutcome {
    pub result: ToolResult,
    pub is_error: bool,
    /// A `before_tool_call` hook refused the call, so it never ran.
    pub blocked: bool,
}

/// A pipe worker's immutable completion, retained until its execution root is released.
pub type PipeCompletion = Result<ToolResult, String>;
/// Command failure is a completed execution, not evidence of uncertain worker cleanup.
pub enum PipeWorkerCompletion {
    Finished(PipeCompletion),
    Uncertain,
}

impl PipeWorkerCompletion {
    pub fn result(&self) -> Result<&ToolResult, &str> {
        match self {
            Self::Finished(Ok(result)) => Ok(result),
            Self::Finished(Err(error)) => Err(error),
            Self::Uncertain => Err("Pipe worker failed; cleanup completion is uncertain."),
        }
    }

    fn is_uncertain(&self) -> bool { matches!(self, Self::Uncertain) }
}

type Completion = Arc<PipeWorkerCompletion>;
type Joined = Shared<BoxFuture<'static, Completion>>;

struct ScopeNode {
    parent: Option<usize>,
    closed: bool,
    cancel: CancellationToken,
}

struct PipeSlot {
    node: usize,
    call_id: String,
    joined: Joined,
}

struct Arena {
    nodes: Vec<ScopeNode>,
    slots: Vec<PipeSlot>,
}

/// Owns pipe workers across disposal of tool, script, and drain futures.
/// Each top-level call has a separate root; child scopes include only their own subtree.
#[derive(Clone)]
pub struct ExecutionScope {
    arena: Arc<Mutex<Arena>>,
    node: usize,
    #[cfg(test)]
    pub(crate) probe: Option<Arc<LifecycleProbe>>,
}

pub(crate) struct PipeWaiter {
    joined: Joined,
}

impl PipeWaiter {
    pub(crate) async fn completion(&self) -> Completion {
        self.joined.clone().await
    }
}

impl Arena {
    fn includes(&self, root: usize, mut node: usize) -> bool {
        loop {
            if node == root {
                return true;
            }
            match self.nodes[node].parent {
                Some(parent) => node = parent,
                None => return false,
            }
        }
    }

    fn open(&self, mut node: usize) -> bool {
        loop {
            if self.nodes[node].closed {
                return false;
            }
            match self.nodes[node].parent {
                Some(parent) => node = parent,
                None => return true,
            }
        }
    }
}

impl ExecutionScope {
    pub fn root(cancel: &CancellationToken) -> Self {
        Self {
            arena: Arc::new(Mutex::new(Arena {
                nodes: vec![ScopeNode { parent: None, closed: false, cancel: cancel.child_token() }],
                slots: Vec::new(),
            })),
            node: 0,
            #[cfg(test)]
            probe: None,
        }
    }

    pub fn child(&self, cancel: &CancellationToken) -> Result<Self, String> {
        let mut arena = self.arena.lock().expect("execution scope lock");
        if !arena.open(self.node) {
            return Err("Execution scope is closed".into());
        }
        let node = arena.nodes.len();
        arena.nodes.push(ScopeNode { parent: Some(self.node), closed: false, cancel: cancel.child_token() });
        Ok(Self {
            arena: self.arena.clone(),
            node,
            #[cfg(test)]
            probe: self.probe.clone(),
        })
    }

    /// Only the internal bash path registers production workers. `make` constructs an async
    /// future without performing effects; it runs outside the registry lock.
    pub(crate) fn spawn_pipe<F, Make>(&self, call_id: &str, make: Make) -> Result<PipeWaiter, String>
    where
        F: Future<Output = PipeCompletion> + Send + 'static,
        Make: FnOnce(CancellationToken) -> F,
    {
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| "Pipe worker requires a runtime".to_string())?;
        let token = self.arena.lock().expect("execution scope lock").nodes[self.node].cancel.child_token();
        let worker = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| make(token)))
            .map_err(|_| "Pipe worker launch failed".to_string())?;
        let call_id = call_id.to_string();
        let (start, ready) = tokio::sync::oneshot::channel();
        let mut arena = self.arena.lock().expect("execution scope lock");
        if !arena.open(self.node) {
            drop(arena);
            return Err("Execution scope is closed".into());
        }
        arena.slots.reserve(1);
        let launched = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| runtime.spawn(async move {
            if ready.await.is_err() {
                return Err("Pipe worker launch cancelled".into());
            }
            worker.await
        })));
        let handle = match launched {
            Ok(handle) => handle,
            Err(_) => {
                drop(arena);
                return Err("Pipe worker launch failed".into());
            }
        };
        #[cfg(test)]
        let probe = self.probe.clone();
        let joined = async move {
            let completed = Arc::new(match handle.await {
                Ok(result) => PipeWorkerCompletion::Finished(result),
                Err(_) => PipeWorkerCompletion::Uncertain,
            });
            #[cfg(test)]
            if let Some(probe) = &probe {
                probe.completed.lock().expect("fixture completions").push(completed.clone());
                let _ = probe.events.send("worker_completed");
            }
            completed
        }.boxed().shared();
        arena.slots.push(PipeSlot { node: self.node, call_id, joined: joined.clone() });
        let _ = start.send(());
        drop(arena);
        Ok(PipeWaiter { joined })
    }

    pub fn close_and_cancel(&self) {
        let mut arena = self.arena.lock().expect("execution scope lock");
        arena.nodes[self.node].closed = true;
        for (node, state) in arena.nodes.iter().enumerate() {
            if arena.includes(self.node, node) {
                state.cancel.cancel();
            }
        }
    }

    pub async fn drain(&self) -> Vec<(String, Completion)> {
        self.close_and_cancel();
        let slots: Vec<_> = {
            let arena = self.arena.lock().expect("execution scope lock");
            arena.slots.iter().filter(|slot| arena.includes(self.node, slot.node))
                .map(|slot| (slot.call_id.clone(), slot.joined.clone())).collect()
        };
        #[cfg(test)]
        self.event("drain");
        let mut completed = Vec::with_capacity(slots.len());
        for (id, joined) in slots {
            completed.push((id, joined.await));
        }
        completed
    }
}

/// Only status and a bounded call identity cross an owning cutoff boundary, never child output.
pub(crate) fn cleanup_diagnostic(completed: &[(String, Completion)]) -> String {
    let failed = completed.iter().filter(|(_, result)| result.is_uncertain()).count();
    let mut text = format!("Pipe cleanup: completed={}, failed={failed}.", completed.len());
    if let Some((id, _)) = completed.iter().find(|(_, result)| result.is_uncertain()) {
        let id: String = id.chars().take(64).map(|ch| if ch.is_control() { '?' } else { ch }).collect();
        text.push_str(&format!(" Failed child: {id}. Cleanup completion is uncertain."));
    }
    text
}

pub(crate) fn fold_cleanup(result: &mut Result<ToolResult, ToolError>, completed: &[(String, Completion)]) {
    if !completed.iter().any(|(_, completion)| completion.is_uncertain()) {
        return;
    }
    let diagnostic = cleanup_diagnostic(completed);
    match result {
        Ok(result) => {
            result.is_error = true;
            result.content.push(ContentPart::text(diagnostic));
        }
        Err(error) => {
            if !error.0.contains(&diagnostic) {
                error.0.push_str("\n\n");
                error.0.push_str(&diagnostic);
            }
        }
    }
}

#[cfg(test)]
pub(crate) struct LifecycleProbe {
    pub events: tokio::sync::mpsc::UnboundedSender<&'static str>,
    pub release: tokio::sync::Semaphore,
    pub timeout_arm: Option<Arc<tokio::sync::Semaphore>>,
    pub completed: Mutex<Vec<Arc<PipeWorkerCompletion>>>,
}

#[cfg(test)]
impl ExecutionScope {
    pub(crate) fn event(&self, event: &'static str) {
        if let Some(probe) = &self.probe {
            let _ = probe.events.send(event);
        }
    }
}

/// Runs calls for a tool that calls other tools while it runs, as a codemode script does. The
/// loop's runner puts each call through the same pipeline as the model's own: the tool's
/// argument shim and schema check, `before_tool_call`, and `after_tool_call`.
#[async_trait]
pub trait ToolRunner: Send + Sync {
    /// `tool_call_id` is the nested call's own id. Never fails: an error is an outcome.
    async fn run_scoped(&self, tool: Arc<dyn Tool>, tool_call_id: String, args: Value, cancel: CancellationToken, scope: &ExecutionScope) -> ToolOutcome;

    async fn run(&self, tool: Arc<dyn Tool>, tool_call_id: String, args: Value, cancel: CancellationToken) -> ToolOutcome {
        let scope = ExecutionScope::root(&cancel);
        let mut outcome = self.run_scoped(tool, tool_call_id, args, cancel, &scope).await;
        let completed = scope.drain().await;
        if completed.iter().any(|(_, result)| result.is_uncertain()) {
            outcome.is_error = true;
            outcome.result.is_error = true;
            outcome.result.content.push(ContentPart::text(cleanup_diagnostic(&completed)));
        }
        outcome
    }
}

/// Runs calls with argument checks and nothing else: what a tool gets when something other
/// than the loop executes it, so no hook applies.
pub struct DirectRunner;

#[async_trait]
impl ToolRunner for DirectRunner {
    async fn run(&self, tool: Arc<dyn Tool>, tool_call_id: String, args: Value, cancel: CancellationToken) -> ToolOutcome {
        let root = ExecutionScope::root(&cancel);
        let outcome = self.run_scoped(tool, tool_call_id, args, cancel, &root).await;
        root.drain().await;
        outcome
    }

    async fn run_scoped(&self, tool: Arc<dyn Tool>, tool_call_id: String, args: Value, cancel: CancellationToken, scope: &ExecutionScope) -> ToolOutcome {
        let args = match crate::agent_loop::checked_arguments(tool.as_ref(), &args) {
            Ok(args) => args,
            Err(message) => return ToolOutcome { result: ToolResult { is_error: true, ..ToolResult::text(message) }, is_error: true, blocked: false },
        };
        let scope = match scope.child(&cancel) {
            Ok(scope) => scope,
            Err(error) => return ToolOutcome { result: ToolResult { is_error: true, ..ToolResult::text(error) }, is_error: true, blocked: true },
        };
        let mut result = tool.execute_scoped(&tool_call_id, args, cancel, Arc::new(|_| {}), self, &scope).await;
        let completed = scope.drain().await;
        fold_cleanup(&mut result, &completed);
        match result {
            Ok(result) => ToolOutcome { is_error: result.is_error, result, blocked: false },
            Err(error) => ToolOutcome { result: ToolResult { is_error: true, ..ToolResult::text(error.0) }, is_error: true, blocked: false },
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct ToolError(pub String);

impl From<String> for ToolError {
    fn from(value: String) -> Self {
        ToolError(value)
    }
}

impl From<&str> for ToolError {
    fn from(value: &str) -> Self {
        ToolError(value.to_string())
    }
}

impl From<serde_json::Error> for ToolError {
    fn from(value: serde_json::Error) -> Self {
        ToolError(format!("Invalid arguments: {value}"))
    }
}

/// Callback for streaming partial results while a tool runs.
pub type ToolUpdateFn = Arc<dyn Fn(ToolResult) + Send + Sync>;

/// Something the model can call. Throw (`Err`) on failure instead of encoding errors in
/// content, or return a result with `is_error` when the failure comes with output.
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;

    /// Human-readable label for UI display.
    fn label(&self) -> &str {
        self.name()
    }

    fn description(&self) -> &str;

    /// JSON Schema for the arguments object.
    fn parameters(&self) -> Value;

    /// JSON Schema of `structured` in this tool's results, for a tool that always sets it. A
    /// codemode script then receives the structured output instead of the text.
    fn output_schema(&self) -> Option<Value> {
        None
    }

    /// Per-tool execution override. `Sequential` forces the whole batch to run one at a time.
    fn execution_mode(&self) -> Option<ToolExecutionMode> {
        None
    }

    /// A shim over the raw arguments before they are checked against `parameters`, for a tool
    /// that accepts an older or looser shape. Returns the arguments to validate.
    fn prepare_arguments(&self, args: Value) -> Value {
        args
    }

    async fn execute(
        &self,
        tool_call_id: &str,
        args: Value,
        cancel: CancellationToken,
        on_update: ToolUpdateFn,
    ) -> Result<ToolResult, ToolError>;

    /// `execute`, with `tools` for calling other tools through the loop's tool pipeline. The
    /// loop calls this one; the default ignores `tools`. A tool that calls other tools, such
    /// as codemode, overrides it.
    async fn execute_with(
        &self,
        tool_call_id: &str,
        args: Value,
        cancel: CancellationToken,
        on_update: ToolUpdateFn,
        _tools: &dyn ToolRunner,
    ) -> Result<ToolResult, ToolError> {
        self.execute(tool_call_id, args, cancel, on_update).await
    }

    /// Execute with explicit ownership for pipe work that must survive a disposed caller.
    async fn execute_scoped(
        &self,
        tool_call_id: &str,
        args: Value,
        cancel: CancellationToken,
        on_update: ToolUpdateFn,
        tools: &dyn ToolRunner,
        scope: &ExecutionScope,
    ) -> Result<ToolResult, ToolError> {
        let _ = scope;
        self.execute_with(tool_call_id, args, cancel, on_update, tools).await
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name().to_string(),
            description: self.description().to_string(),
            parameters: self.parameters(),
        }
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    #[tokio::test]
    async fn ordinary_command_error_is_not_uncertain_cleanup() {
        let root = ExecutionScope::root(&CancellationToken::new());
        let waiter = root.spawn_pipe("command", |_| async { Err("outputbefore\nCommand terminated by signal 9".into()) }).unwrap();
        let completed = root.drain().await;
        assert_eq!(waiter.completion().await.result().unwrap_err(), "outputbefore\nCommand terminated by signal 9");
        let mut outcome = Err(ToolError("outputbefore\nCommand terminated by signal 9".into()));
        fold_cleanup(&mut outcome, &completed);
        assert_eq!(outcome.unwrap_err().0, "outputbefore\nCommand terminated by signal 9");
        assert_eq!(cleanup_diagnostic(&completed), "Pipe cleanup: completed=1, failed=0.");
    }


    #[tokio::test]
    async fn inclusive_drain_survives_waiter_disposal_and_excludes_siblings() {
        let root = ExecutionScope::root(&CancellationToken::new());
        let child = root.child(&CancellationToken::new()).unwrap();
        let sibling = root.child(&CancellationToken::new()).unwrap();
        let (release, released) = tokio::sync::oneshot::channel();
        let waiter = child.spawn_pipe("own", |_| async move {
            released.await.unwrap();
            Ok(ToolResult::text("private"))
        }).unwrap();
        sibling.spawn_pipe("sibling", |cancel| async move { cancel.cancelled().await; Ok(ToolResult::text("sibling")) }).unwrap();
        let first = child.drain();
        tokio::pin!(first);
        assert!(futures::poll!(&mut first).is_pending());
        // A separately owned waiter can disappear without consuming the retained join.
        {
            let abandoned = child.drain();
            tokio::pin!(abandoned);
            assert!(futures::poll!(&mut abandoned).is_pending());
        }
        release.send(()).unwrap();
        let completed = first.await;
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].0, "own");
        let normal = waiter.completion().await;
        assert!(Arc::ptr_eq(&normal, &completed[0].1));
        assert_eq!(normal.result().unwrap().text_content(), "private");
        assert!(child.child(&CancellationToken::new()).is_err());
        assert!(child.spawn_pipe("late", |_| async { panic!("closed worker polled") }).is_err());
        assert!(sibling.child(&CancellationToken::new()).is_ok());
        assert_eq!(sibling.drain().await.len(), 1);
        root.drain().await;
    }

    #[tokio::test]
    async fn ancestor_closure_during_factory_rejects_without_polling() {
        let root = ExecutionScope::root(&CancellationToken::new());
        let child = root.child(&CancellationToken::new()).unwrap();
        let result = child.spawn_pipe("race", |_| {
            root.close_and_cancel();
            async { panic!("rejected worker was polled") }
        });
        assert!(result.is_err());
        assert!(child.child(&CancellationToken::new()).is_err());
        assert!(root.drain().await.is_empty());
    }

    #[tokio::test]
    async fn worker_panic_and_factory_failure_have_stable_outcomes() {
        let root = ExecutionScope::root(&CancellationToken::new());
        let failure = root.spawn_pipe("factory", |_| -> std::future::Ready<PipeCompletion> { panic!("factory") });
        assert!(failure.is_err());
        let waiter = root.spawn_pipe("panic\nprivate", |_| async { panic!("secret panic payload") }).unwrap();
        let completed = root.drain().await;
        assert_eq!(completed.len(), 1);
        assert_eq!(waiter.completion().await.result().unwrap_err(), "Pipe worker failed; cleanup completion is uncertain.");
        let diagnostic = cleanup_diagnostic(&completed);
        assert!(diagnostic.contains("panic?private"));
        assert!(!diagnostic.contains("secret"));
        let mut error = Err(ToolError("Original failure".into()));
        fold_cleanup(&mut error, &completed);
        let error = error.unwrap_err().0;
        assert!(error.starts_with("Original failure\n\nPipe cleanup:"));
        assert!(error.contains("Cleanup completion is uncertain."));
        assert!(!error.contains("secret panic payload"));
        let mut success = Ok(ToolResult::text("original"));
        fold_cleanup(&mut success, &completed);
        let success = success.unwrap();
        assert!(success.is_error);
        assert!(success.text_content().contains("Pipe cleanup:"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_close_and_registration_never_lose_an_admitted_worker() {
        for _ in 0..16 {
            let root = ExecutionScope::root(&CancellationToken::new());
            let child = root.child(&CancellationToken::new()).unwrap();
            let barrier = Arc::new(std::sync::Barrier::new(2));
            let closing = root.clone();
            let start = barrier.clone();
            let thread = std::thread::spawn(move || { start.wait(); closing.close_and_cancel(); });
            let admitted = child.spawn_pipe("race", |cancel| {
                barrier.wait();
                async move { cancel.cancelled().await; Ok(ToolResult::text("ack")) }
            });
            thread.join().unwrap();
            let completed = root.drain().await;
            assert_eq!(completed.len(), usize::from(admitted.is_ok()));
            if let Ok(waiter) = admitted {
                assert_eq!(waiter.completion().await.result().unwrap().text_content(), "ack");
            }
        }
    }

    #[tokio::test]
    async fn parent_drain_retains_child_worker_and_other_roots_finish_independently() {
        let root = ExecutionScope::root(&CancellationToken::new());
        let child = root.child(&CancellationToken::new()).unwrap();
        let other = ExecutionScope::root(&CancellationToken::new());
        let (release, released) = tokio::sync::oneshot::channel();
        let waiter = child.spawn_pipe("private", |_| async move { released.await.unwrap(); Ok(ToolResult::text("private")) }).unwrap();
        {
            let drain = child.drain();
            tokio::pin!(drain);
            assert!(futures::poll!(&mut drain).is_pending());
        }
        other.spawn_pipe("public", |_| async { Ok(ToolResult::text("public")) }).unwrap();
        let independent = other.drain().await;
        assert_eq!(independent.len(), 1);
        assert_eq!(independent[0].1.result().unwrap().text_content(), "public");
        let parent = root.drain();
        tokio::pin!(parent);
        assert!(futures::poll!(&mut parent).is_pending());
        release.send(()).unwrap();
        let completed = parent.await;
        assert_eq!(completed.len(), 1);
        assert!(Arc::ptr_eq(&completed[0].1, &waiter.completion().await));
        assert!(!cleanup_diagnostic(&independent).contains("private"));
    }

    #[test]
    fn runtime_failure_never_constructs_worker() {
        let root = ExecutionScope::root(&CancellationToken::new());
        assert!(root.spawn_pipe("no runtime", |_| -> std::future::Ready<PipeCompletion> { panic!("factory invoked") }).is_err());
    }
}
