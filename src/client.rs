//! Multiplexing client over any [`Transport`].
//!
//! Source: pi `f1b2e77`
//! `packages/coding-agent/src/modes/rpc/rpc-client.ts:50` (listener type),
//! `:118-225` (start, stop, stdin errors, exit handling),
//! `:471-527` (`waitForIdle`, `collectEvents`, `promptAndWait`),
//! `:556-640` (`send` with id map and 30s timeout).
//!
//! IDs are `req_N` counters. Responses match by id, never by order.
//! `prompt` success means accepted, not done; wait helpers end at
//! `agent_settled`. Subscribe before send so fast completions never drop.

use std::collections::{BTreeMap, HashMap};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};
use std::time::Duration;

use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::command::{
    Bash, Command, Common, Compact, ExportHtml, FollowUp, Fork, GetEntries, NewSession, Prompt,
    SetFlag, SetModel, SetQueueMode, SetSessionName, SetThinkingLevel, Steer, SwitchSession,
};
use crate::error::{Error, ErrorKind, Result};
use crate::event::{AssistantUpdate, Event, MessageUpdate};
use crate::extui::{UiAnswer, UiHandler, UiOutcome, UiRequest, encode_ui_response};
use crate::frame::DEFAULT_MAX_LINE;
use crate::message::Message;
use crate::response::{
    ClearQueueData, CompactionResult, EntryList, ExportPath, ForkData, ForkMessage, Response,
    SessionState, SessionStats, SessionSwitch, TreeList,
};
use crate::transport::{Transport, TransportRecv, TransportSend};
use crate::types::{
    ImageContent, Model, PromptDisposition, QueueMode, QueuedInputDisposition, SlashCommand,
    StreamingBehavior, ThinkingLevel,
};

/// Timeout for one request/response round trip unless overridden per call.
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Client tuning. Per-call timeout overrides apply on wait helpers only.
///
/// New fields may arrive in later versions; build with `..Default::default()`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Config {
    /// Deadline for one request/response round trip.
    pub request_timeout: Duration,
    /// Deadline for run waits.
    pub wait_timeout: Duration,
    /// Broadcast channel depth for session events.
    pub event_capacity: usize,
    /// Max accepted record size in bytes. Larger lines skip with a count;
    /// the stream stays alive and framing stays synced.
    pub max_line: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
            wait_timeout: Duration::from_secs(60),
            event_capacity: 1024,
            max_line: DEFAULT_MAX_LINE,
        }
    }
}

impl Config {
    /// Set the request round-trip deadline.
    pub fn with_request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }

    /// Set the run-wait deadline.
    pub fn with_wait_timeout(mut self, timeout: Duration) -> Self {
        self.wait_timeout = timeout;
        self
    }

    /// Set the broadcast channel depth (clamped to at least 1 on use).
    pub fn with_event_capacity(mut self, capacity: usize) -> Self {
        self.event_capacity = capacity;
        self
    }

    /// Set the max accepted record size in bytes.
    pub fn with_max_line(mut self, max_line: usize) -> Self {
        self.max_line = max_line;
        self
    }
}

/// Full prompt arguments for [`Client::prompt_with`].
#[derive(Debug, Clone, Default)]
pub struct PromptReq {
    pub message: String,
    pub images: Option<Vec<ImageContent>>,
    pub streaming_behavior: Option<StreamingBehavior>,
}

/// Full bash arguments for [`Client::bash_with`].
#[derive(Debug, Clone, Default)]
pub struct BashReq {
    pub command: String,
    pub exclude_from_context: Option<bool>,
}

/// Full compact arguments for [`Client::compact_with`].
#[derive(Debug, Clone, Default)]
pub struct CompactReq {
    pub custom_instructions: Option<String>,
}

type Callback = Arc<dyn Fn(Arc<Event>) + Send + Sync + 'static>;

/// Counters for records the read loop drops.
///
/// Drops keep flowing: malformed lines, stray responses, bad events, and
/// bad UI requests never fail the stream. Read counts in tests to spot
/// protocol drift early.
///
/// New counters may arrive in later versions; read fields, never build it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct DropCounts {
    /// Lines dropped before dispatch: unparsable bytes, over-cap lines,
    /// and undecodable records with no usable waiter id.
    pub malformed: u64,
    /// Responses with no id or an id no request waits for.
    pub stray_response: u64,
    /// Lines tagged as events that fail to parse.
    pub bad_event: u64,
    /// UI requests that fail to parse.
    pub bad_ui: u64,
    /// Event callbacks that panicked instead of returning.
    pub callback_panic: u64,
}

/// One outbound job for the loop that owns the transport.
enum TxJob {
    Send {
        line: String,
        done: oneshot::Sender<Result<()>>,
    },
    Close {
        done: oneshot::Sender<Result<()>>,
    },
}

struct Inner {
    tx: mpsc::UnboundedSender<TxJob>,
    pending: Mutex<HashMap<String, oneshot::Sender<Result<Response>>>>,
    events: broadcast::Sender<Arc<Event>>,
    callbacks: Mutex<Vec<(u64, Callback)>>,
    ui: tokio::sync::Mutex<Option<Box<dyn UiHandler + Send>>>,
    drops: Mutex<DropCounts>,
    next_id: AtomicU64,
    next_cb: AtomicU64,
    handles: AtomicUsize,
    closed: AtomicBool,
    shutdown: watch::Sender<bool>,
    config: Config,
    tasks: Mutex<Vec<JoinHandle<()>>>,
}

/// Typed client for one pi session. All methods take `&self`.
///
/// Cloning shares the session: one id counter, one event bus, one pair of
/// tasks. The session ends when the last handle drops or [`Client::close`]
/// runs, whichever comes first.
pub struct Client {
    inner: Arc<Inner>,
}

impl Clone for Client {
    fn clone(&self) -> Self {
        self.inner.handles.fetch_add(1, Ordering::SeqCst);
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = &self.inner;
        f.debug_struct("Client")
            .field("closed", &inner.closed.load(Ordering::SeqCst))
            .field("next_id", &inner.next_id.load(Ordering::SeqCst))
            .field("pending", &inner.pending.lock().unwrap().len())
            .field("drops", &inner.drops.lock().unwrap())
            .finish_non_exhaustive()
    }
}

impl Client {
    /// Spawn the read loop and return the client.
    ///
    /// # Panics
    ///
    /// Panics without a running Tokio runtime, like `tokio::spawn`.
    pub fn new<T: Transport>(transport: T) -> Self {
        Self::with_config(transport, Config::default())
    }

    /// Spawn the read loop with explicit tuning.
    ///
    /// Zero `event_capacity` clamps to 1: the channel panics on zero.
    ///
    /// # Panics
    ///
    /// Panics without a running Tokio runtime, like `tokio::spawn`.
    pub fn with_config<T: Transport>(transport: T, config: Config) -> Self {
        let capacity = config.event_capacity.max(1);
        let (events, _) = broadcast::channel(capacity);
        let (tx, rx) = mpsc::unbounded_channel();
        let inner = Arc::new(Inner {
            tx,
            pending: Mutex::new(HashMap::new()),
            events,
            callbacks: Mutex::new(Vec::new()),
            ui: tokio::sync::Mutex::new(None),
            drops: Mutex::new(DropCounts::default()),
            next_id: AtomicU64::new(1),
            next_cb: AtomicU64::new(1),
            handles: AtomicUsize::new(1),
            closed: AtomicBool::new(false),
            shutdown: watch::Sender::new(false),
            config,
            tasks: Mutex::new(Vec::new()),
        });
        let (tx_half, rx_half) = transport.split();
        let reader = tokio::spawn(read_lines(rx_half, inner.clone()));
        let looper = tokio::spawn(run_loop(tx_half, rx, inner.clone()));
        inner.tasks.lock().unwrap().extend([reader, looper]);
        Self { inner }
    }

    /// Read-only view of the tuning this client runs with.
    pub fn config(&self) -> Config {
        self.inner.config.clone()
    }

    /// Low-level send for any command. Prefer the typed methods.
    pub async fn send_raw(&self, cmd: Command) -> Result<Response> {
        self.send_cmd(cmd).await
    }

    /// Send a prompt. Success means accepted, queued, or handled.
    pub async fn prompt(&self, message: &str) -> Result<PromptDisposition> {
        self.prompt_with(PromptReq {
            message: message.to_string(),
            ..PromptReq::default()
        })
        .await
    }

    /// Send a prompt with images or mid-run queue behavior.
    pub async fn prompt_with(&self, req: PromptReq) -> Result<PromptDisposition> {
        let body = self
            .send_cmd(Command::Prompt(Prompt {
                id: None,
                message: req.message,
                images: req.images,
                streaming_behavior: req.streaming_behavior,
            }))
            .await?;
        match body {
            Response::Prompt(data) => Ok(data.disposition),
            Response::Error(failed) => Err(command_failed(&failed)),
            other => Err(unexpected("prompt", &other)),
        }
    }

    /// Queue a steering message for the running turn.
    pub async fn steer(&self, message: &str) -> Result<QueuedInputDisposition> {
        self.steer_with(message, None).await
    }

    /// Queue a steering message with optional images.
    pub async fn steer_with(
        &self,
        message: &str,
        images: Option<Vec<ImageContent>>,
    ) -> Result<QueuedInputDisposition> {
        let body = self
            .send_cmd(Command::Steer(Steer {
                id: None,
                message: message.to_string(),
                images,
            }))
            .await?;
        match body {
            Response::Steer(data) => Ok(data.disposition),
            Response::Error(failed) => Err(command_failed(&failed)),
            other => Err(unexpected("steer", &other)),
        }
    }

    /// Queue a follow-up message for after the run settles.
    pub async fn follow_up(&self, message: &str) -> Result<QueuedInputDisposition> {
        self.follow_up_with(message, None).await
    }

    /// Queue a follow-up message with optional images.
    pub async fn follow_up_with(
        &self,
        message: &str,
        images: Option<Vec<ImageContent>>,
    ) -> Result<QueuedInputDisposition> {
        let body = self
            .send_cmd(Command::FollowUp(FollowUp {
                id: None,
                message: message.to_string(),
                images,
            }))
            .await?;
        match body {
            Response::FollowUp(data) => Ok(data.disposition),
            Response::Error(failed) => Err(command_failed(&failed)),
            other => Err(unexpected("follow_up", &other)),
        }
    }

    /// Abort the current operation and wait for idle.
    pub async fn abort(&self) -> Result<()> {
        match self.send_cmd(Command::Abort(Common { id: None })).await? {
            Response::Abort => Ok(()),
            Response::Error(failed) => Err(command_failed(&failed)),
            other => Err(unexpected("abort", &other)),
        }
    }

    /// Clear queued inputs and get their texts back for editor restore.
    pub async fn clear_queue(&self) -> Result<ClearQueueData> {
        match self
            .send_cmd(Command::ClearQueue(Common { id: None }))
            .await?
        {
            Response::ClearQueue(data) => Ok(data),
            Response::Error(failed) => Err(command_failed(&failed)),
            other => Err(unexpected("clear_queue", &other)),
        }
    }

    /// Start a fresh session, optionally tracking its parent file.
    pub async fn new_session(&self, parent_session: Option<&str>) -> Result<SessionSwitch> {
        let body = self
            .send_cmd(Command::NewSession(NewSession {
                id: None,
                parent_session: parent_session.map(str::to_string),
            }))
            .await?;
        match body {
            Response::NewSession(data) => Ok(data),
            Response::Error(failed) => Err(command_failed(&failed)),
            other => Err(unexpected("new_session", &other)),
        }
    }

    /// Read the live session snapshot.
    pub async fn get_state(&self) -> Result<SessionState> {
        match self
            .send_cmd(Command::GetState(Common { id: None }))
            .await?
        {
            Response::GetState(data) => Ok(data),
            Response::Error(failed) => Err(command_failed(&failed)),
            other => Err(unexpected("get_state", &other)),
        }
    }

    /// Switch to a model by provider and id.
    pub async fn set_model(&self, provider: &str, model_id: &str) -> Result<Model> {
        let body = self
            .send_cmd(Command::SetModel(SetModel {
                id: None,
                provider: provider.to_string(),
                model_id: model_id.to_string(),
            }))
            .await?;
        match body {
            Response::SetModel(data) => Ok(data),
            Response::Error(failed) => Err(command_failed(&failed)),
            other => Err(unexpected("set_model", &other)),
        }
    }

    /// Cycle to the next model. Null when only one model exists.
    pub async fn cycle_model(&self) -> Result<Option<crate::response::CycleModel>> {
        match self
            .send_cmd(Command::CycleModel(Common { id: None }))
            .await?
        {
            Response::CycleModel(data) => Ok(data),
            Response::Error(failed) => Err(command_failed(&failed)),
            other => Err(unexpected("cycle_model", &other)),
        }
    }

    /// List configured models with full definitions.
    ///
    /// The wire already carries full models (`getAvailableSnapshot`).
    pub async fn get_available_models(&self) -> Result<Vec<Model>> {
        let body = self
            .send_cmd(Command::GetAvailableModels(Common { id: None }))
            .await?;
        match body {
            Response::GetAvailableModels(models) => Ok(models),
            Response::Error(failed) => Err(command_failed(&failed)),
            other => Err(unexpected("get_available_models", &other)),
        }
    }

    /// Set the reasoning level for models that support it.
    pub async fn set_thinking_level(&self, level: ThinkingLevel) -> Result<()> {
        match self
            .send_cmd(Command::SetThinkingLevel(SetThinkingLevel {
                id: None,
                level,
            }))
            .await?
        {
            Response::SetThinkingLevel => Ok(()),
            Response::Error(failed) => Err(command_failed(&failed)),
            other => Err(unexpected("set_thinking_level", &other)),
        }
    }

    /// Cycle the thinking level. Null when the model lacks support.
    pub async fn cycle_thinking_level(&self) -> Result<Option<crate::response::ThinkingLevelData>> {
        match self
            .send_cmd(Command::CycleThinkingLevel(Common { id: None }))
            .await?
        {
            Response::CycleThinkingLevel(data) => Ok(data),
            Response::Error(failed) => Err(command_failed(&failed)),
            other => Err(unexpected("cycle_thinking_level", &other)),
        }
    }

    /// List thinking levels for the current model.
    pub async fn get_available_thinking_levels(&self) -> Result<Vec<ThinkingLevel>> {
        match self
            .send_cmd(Command::GetAvailableThinkingLevels(Common { id: None }))
            .await?
        {
            Response::GetAvailableThinkingLevels(data) => Ok(data),
            Response::Error(failed) => Err(command_failed(&failed)),
            other => Err(unexpected("get_available_thinking_levels", &other)),
        }
    }

    /// Control how steering messages flush.
    pub async fn set_steering_mode(&self, mode: QueueMode) -> Result<()> {
        match self
            .send_cmd(Command::SetSteeringMode(SetQueueMode { id: None, mode }))
            .await?
        {
            Response::SetSteeringMode => Ok(()),
            Response::Error(failed) => Err(command_failed(&failed)),
            other => Err(unexpected("set_steering_mode", &other)),
        }
    }

    /// Control how follow-up messages flush.
    pub async fn set_follow_up_mode(&self, mode: QueueMode) -> Result<()> {
        match self
            .send_cmd(Command::SetFollowUpMode(SetQueueMode { id: None, mode }))
            .await?
        {
            Response::SetFollowUpMode => Ok(()),
            Response::Error(failed) => Err(command_failed(&failed)),
            other => Err(unexpected("set_follow_up_mode", &other)),
        }
    }

    /// Compact context with optional focus instructions.
    pub async fn compact(&self, custom_instructions: Option<&str>) -> Result<CompactionResult> {
        self.compact_with(CompactReq {
            custom_instructions: custom_instructions.map(str::to_string),
        })
        .await
    }

    /// Compact context with the full argument set.
    pub async fn compact_with(&self, req: CompactReq) -> Result<CompactionResult> {
        let body = self
            .send_cmd(Command::Compact(Compact {
                id: None,
                custom_instructions: req.custom_instructions,
            }))
            .await?;
        match body {
            Response::Compact(data) => Ok(data),
            Response::Error(failed) => Err(command_failed(&failed)),
            other => Err(unexpected("compact", &other)),
        }
    }

    /// Toggle automatic compaction near context end.
    pub async fn set_auto_compaction(&self, enabled: bool) -> Result<()> {
        match self
            .send_cmd(Command::SetAutoCompaction(SetFlag { id: None, enabled }))
            .await?
        {
            Response::SetAutoCompaction => Ok(()),
            Response::Error(failed) => Err(command_failed(&failed)),
            other => Err(unexpected("set_auto_compaction", &other)),
        }
    }

    /// Toggle automatic retry on transient provider errors.
    pub async fn set_auto_retry(&self, enabled: bool) -> Result<()> {
        match self
            .send_cmd(Command::SetAutoRetry(SetFlag { id: None, enabled }))
            .await?
        {
            Response::SetAutoRetry => Ok(()),
            Response::Error(failed) => Err(command_failed(&failed)),
            other => Err(unexpected("set_auto_retry", &other)),
        }
    }

    /// Cancel the delay and stop an in-progress retry.
    pub async fn abort_retry(&self) -> Result<()> {
        match self
            .send_cmd(Command::AbortRetry(Common { id: None }))
            .await?
        {
            Response::AbortRetry => Ok(()),
            Response::Error(failed) => Err(command_failed(&failed)),
            other => Err(unexpected("abort_retry", &other)),
        }
    }

    /// Run a shell command inside the session and get its final result.
    pub async fn bash(&self, command: &str) -> Result<crate::response::BashResult> {
        self.bash_with(BashReq {
            command: command.to_string(),
            exclude_from_context: None,
        })
        .await
    }

    /// Run a shell command with the full argument set.
    pub async fn bash_with(&self, req: BashReq) -> Result<crate::response::BashResult> {
        let body = self
            .send_cmd(Command::Bash(Bash {
                id: None,
                command: req.command,
                exclude_from_context: req.exclude_from_context,
            }))
            .await?;
        match body {
            Response::Bash(data) => Ok(data),
            Response::Error(failed) => Err(command_failed(&failed)),
            other => Err(unexpected("bash", &other)),
        }
    }

    /// Abort the running bash command.
    pub async fn abort_bash(&self) -> Result<()> {
        match self
            .send_cmd(Command::AbortBash(Common { id: None }))
            .await?
        {
            Response::AbortBash => Ok(()),
            Response::Error(failed) => Err(command_failed(&failed)),
            other => Err(unexpected("abort_bash", &other)),
        }
    }

    /// Read token use, cost, and context-window use.
    pub async fn get_session_stats(&self) -> Result<SessionStats> {
        match self
            .send_cmd(Command::GetSessionStats(Common { id: None }))
            .await?
        {
            Response::GetSessionStats(data) => Ok(data),
            Response::Error(failed) => Err(command_failed(&failed)),
            other => Err(unexpected("get_session_stats", &other)),
        }
    }

    /// Export the session transcript to HTML.
    pub async fn export_html(&self, output_path: Option<&str>) -> Result<ExportPath> {
        let body = self
            .send_cmd(Command::ExportHtml(ExportHtml {
                id: None,
                output_path: output_path.map(str::to_string),
            }))
            .await?;
        match body {
            Response::ExportHtml(data) => Ok(data),
            Response::Error(failed) => Err(command_failed(&failed)),
            other => Err(unexpected("export_html", &other)),
        }
    }

    /// Load a different session file.
    pub async fn switch_session(&self, session_path: &str) -> Result<SessionSwitch> {
        let body = self
            .send_cmd(Command::SwitchSession(SwitchSession {
                id: None,
                session_path: session_path.to_string(),
            }))
            .await?;
        match body {
            Response::SwitchSession(data) => Ok(data),
            Response::Error(failed) => Err(command_failed(&failed)),
            other => Err(unexpected("switch_session", &other)),
        }
    }

    /// Fork from a previous user message entry.
    pub async fn fork(&self, entry_id: &str) -> Result<ForkData> {
        let body = self
            .send_cmd(Command::Fork(Fork {
                id: None,
                entry_id: entry_id.to_string(),
            }))
            .await?;
        match body {
            Response::Fork(data) => Ok(data),
            Response::Error(failed) => Err(command_failed(&failed)),
            other => Err(unexpected("fork", &other)),
        }
    }

    /// Duplicate the active branch into a new session.
    pub async fn clone_session(&self) -> Result<SessionSwitch> {
        match self.send_cmd(Command::Clone(Common { id: None })).await? {
            Response::Clone(data) => Ok(data),
            Response::Error(failed) => Err(command_failed(&failed)),
            other => Err(unexpected("clone", &other)),
        }
    }

    /// List user messages available for forking.
    pub async fn get_fork_messages(&self) -> Result<Vec<ForkMessage>> {
        match self
            .send_cmd(Command::GetForkMessages(Common { id: None }))
            .await?
        {
            Response::GetForkMessages(data) => Ok(data),
            Response::Error(failed) => Err(command_failed(&failed)),
            other => Err(unexpected("get_fork_messages", &other)),
        }
    }

    /// Read entries in append order. `since` is a durable cursor.
    pub async fn get_entries(&self, since: Option<&str>) -> Result<EntryList> {
        let body = self
            .send_cmd(Command::GetEntries(GetEntries {
                id: None,
                since: since.map(str::to_string),
            }))
            .await?;
        match body {
            Response::GetEntries(data) => Ok(data),
            Response::Error(failed) => Err(command_failed(&failed)),
            other => Err(unexpected("get_entries", &other)),
        }
    }

    /// Read the session as a branch tree.
    pub async fn get_tree(&self) -> Result<TreeList> {
        match self.send_cmd(Command::GetTree(Common { id: None })).await? {
            Response::GetTree(data) => Ok(data),
            Response::Error(failed) => Err(command_failed(&failed)),
            other => Err(unexpected("get_tree", &other)),
        }
    }

    /// Read the last assistant text. Null when no assistant text exists.
    pub async fn get_last_assistant_text(&self) -> Result<Option<String>> {
        match self
            .send_cmd(Command::GetLastAssistantText(Common { id: None }))
            .await?
        {
            Response::GetLastAssistantText(data) => Ok(data),
            Response::Error(failed) => Err(command_failed(&failed)),
            other => Err(unexpected("get_last_assistant_text", &other)),
        }
    }

    /// Set the display name of the current session.
    pub async fn set_session_name(&self, name: &str) -> Result<()> {
        match self
            .send_cmd(Command::SetSessionName(SetSessionName {
                id: None,
                name: name.to_string(),
            }))
            .await?
        {
            Response::SetSessionName => Ok(()),
            Response::Error(failed) => Err(command_failed(&failed)),
            other => Err(unexpected("set_session_name", &other)),
        }
    }

    /// Read all conversation messages.
    pub async fn get_messages(&self) -> Result<Vec<Message>> {
        match self
            .send_cmd(Command::GetMessages(Common { id: None }))
            .await?
        {
            Response::GetMessages(data) => Ok(data),
            Response::Error(failed) => Err(command_failed(&failed)),
            other => Err(unexpected("get_messages", &other)),
        }
    }

    /// List commands runnable through `prompt` with a `/` prefix.
    pub async fn get_commands(&self) -> Result<Vec<SlashCommand>> {
        match self
            .send_cmd(Command::GetCommands(Common { id: None }))
            .await?
        {
            Response::GetCommands(data) => Ok(data),
            Response::Error(failed) => Err(command_failed(&failed)),
            other => Err(unexpected("get_commands", &other)),
        }
    }

    /// Open another event stream. Slow readers lag; lag errors on read.
    pub fn subscribe(&self) -> EventStream {
        EventStream {
            rx: self.inner.events.subscribe(),
            shutdown: self.inner.shutdown.subscribe(),
        }
    }

    /// Dropped-record counters since creation. All zeros means a clean stream.
    ///
    /// # Panics
    ///
    /// Panics on a poisoned lock. Locks guard plain data and never cross
    /// an await, so this only fires after a bug elsewhere in the crate.
    pub fn drop_counts(&self) -> DropCounts {
        *self.inner.drops.lock().unwrap()
    }

    /// Add a callback listener. Keep it fast; it runs on the read loop.
    /// Dropping the guard unsubscribes.
    pub fn on_event(&self, callback: impl Fn(Arc<Event>) + Send + Sync + 'static) -> Subscription {
        let id = self.inner.next_cb.fetch_add(1, Ordering::SeqCst);
        self.inner
            .callbacks
            .lock()
            .unwrap()
            .push((id, Arc::new(callback)));
        Subscription {
            inner: self.inner.clone(),
            id,
        }
    }

    /// Wait until `agent_settled`. `None` means the 60s wait default.
    ///
    /// Sees only events that arrive after this call subscribes. A settle
    /// that already fired is missed, like upstream `waitForIdle`.
    pub async fn wait_for_idle(&self, timeout: Option<Duration>) -> Result<()> {
        let deadline = Instant::now() + timeout.unwrap_or(self.inner.config.wait_timeout);
        let mut rx = self.subscribe();
        loop {
            if matches!(
                &*recv_before(&mut rx, deadline).await?,
                Event::AgentSettled(_)
            ) {
                return Ok(());
            }
        }
    }

    /// Gather events until `agent_settled`. `None` means the 60s wait default.
    ///
    /// Starts from subscribe time; earlier events are not replayed. Use
    /// `prompt_and_wait` to cover a full run from its first event.
    /// Events arrive shared (`Arc`): clone the `Arc`, not the payload.
    pub async fn collect_events(&self, timeout: Option<Duration>) -> Result<Vec<Arc<Event>>> {
        let deadline = Instant::now() + timeout.unwrap_or(self.inner.config.wait_timeout);
        let mut rx = self.subscribe();
        let mut out = Vec::new();
        loop {
            let event = recv_before(&mut rx, deadline).await?;
            let settled = matches!(&*event, Event::AgentSettled(_));
            out.push(event);
            if settled {
                return Ok(out);
            }
        }
    }

    /// Prompt, then gather events until `agent_settled`.
    /// Subscribes before send so fast completions never drop.
    /// A `handled` disposition starts no run and returns empty.
    pub async fn prompt_and_wait(
        &self,
        message: &str,
        timeout: Option<Duration>,
    ) -> Result<Vec<Arc<Event>>> {
        self.prompt_and_wait_with(
            PromptReq {
                message: message.to_string(),
                ..PromptReq::default()
            },
            timeout,
        )
        .await
    }

    /// Prompt with full arguments, then gather events until settled.
    /// Subscribes before send so fast completions never drop.
    /// A `handled` disposition starts no run and returns empty.
    pub async fn prompt_and_wait_with(
        &self,
        req: PromptReq,
        timeout: Option<Duration>,
    ) -> Result<Vec<Arc<Event>>> {
        let deadline = Instant::now() + timeout.unwrap_or(self.inner.config.wait_timeout);
        let mut rx = self.subscribe();
        match self.prompt_with(req).await? {
            PromptDisposition::Handled => Ok(Vec::new()),
            _ => {
                let mut out = Vec::new();
                loop {
                    let event = recv_before(&mut rx, deadline).await?;
                    let settled = matches!(&*event, Event::AgentSettled(_));
                    out.push(event);
                    if settled {
                        return Ok(out);
                    }
                }
            }
        }
    }

    /// Prompt, wait, and fold text deltas into one string.
    ///
    /// Folds `text_start`, `text_delta`, and `text_end` in block order.
    /// Thinking and tool-call deltas never join the output.
    /// Text from every assistant message in the run concatenates with no
    /// separator and no per-message grouping.
    pub async fn prompt_text(&self, message: &str, timeout: Option<Duration>) -> Result<String> {
        let events = self.prompt_and_wait(message, timeout).await?;
        Ok(fold_text(&events))
    }

    /// Install the dialog handler. One slot; a later call replaces it.
    pub async fn set_ui_handler(&self, handler: impl UiHandler + 'static) {
        *self.inner.ui.lock().await = Some(Box::new(handler));
    }

    /// Close the write side, fail pending work, stop the read loop.
    ///
    /// Prefer this over dropping: `Drop` aborts the loop without the
    /// close handshake, so the peer sees a cut stream.
    ///
    /// The drain waits up to the request timeout. Past it, the loop is
    /// aborted, pending work fails, and `close` reports `Timeout`.
    /// A second call returns `Ok` at once, even if the first one failed.
    pub async fn close(&self) -> Result<()> {
        if self.inner.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        self.inner.shutdown.send_replace(true);
        let (done_tx, done_rx) = oneshot::channel();
        if self.inner.tx.send(TxJob::Close { done: done_tx }).is_err() {
            self.fail_all(Error::new(ErrorKind::Closed, "read loop gone"));
            return Ok(());
        }
        let drained = tokio::time::timeout(self.inner.config.request_timeout, done_rx).await;
        let result = match drained {
            Ok(result) => result.unwrap_or(Err(Error::new(ErrorKind::Closed, "read loop gone"))),
            Err(_) => Err(Error::new(ErrorKind::Timeout, "close stalled")),
        };
        self.fail_all(Error::new(ErrorKind::Closed, "client closed"));
        if let Ok(mut tasks) = self.inner.tasks.lock() {
            for task in tasks.drain(..) {
                task.abort();
            }
        }
        result
    }

    async fn send_cmd(&self, cmd: Command) -> Result<Response> {
        if self.inner.closed.load(Ordering::SeqCst) {
            return Err(Error::new(ErrorKind::Closed, "transport closed"));
        }
        // One deadline covers the send leg and the response wait, like the
        // single upstream timer. Worst case stays one timeout, not two.
        let deadline = Instant::now() + self.inner.config.request_timeout;
        let tag = cmd.tag();
        let mut value = serde_json::to_value(&cmd)
            .map_err(|err| Error::new(ErrorKind::Protocol, format!("bad command: {err}")))?;
        // Always stamp a fresh id like upstream (`rpc-client.ts:577-578`).
        // A caller id would collide with the counter and cross requests.
        let id = format!("req_{}", self.inner.next_id.fetch_add(1, Ordering::SeqCst));
        value["id"] = id.clone().into();
        let line = serde_json::to_string(&value)
            .map_err(|err| Error::new(ErrorKind::Protocol, format!("bad command: {err}")))?;
        let (tx, rx) = oneshot::channel();
        self.inner.pending.lock().unwrap().insert(id.clone(), tx);
        let (done_tx, done_rx) = oneshot::channel();
        if self
            .inner
            .tx
            .send(TxJob::Send {
                line,
                done: done_tx,
            })
            .is_err()
        {
            self.inner.pending.lock().unwrap().remove(&id);
            return Err(Error::new(ErrorKind::Closed, "read loop gone"));
        }
        // The send leg shares the deadline: a wedged transport surfaces
        // as Timeout, never hangs the caller.
        let send_done = tokio::time::timeout_at(deadline, done_rx).await;
        match send_done {
            Err(_) => {
                self.inner.pending.lock().unwrap().remove(&id);
                Err(Error::new(
                    ErrorKind::Timeout,
                    format!("send stalled for {tag} ({id})"),
                ))
            }
            Ok(Err(_)) => {
                self.inner.pending.lock().unwrap().remove(&id);
                Err(Error::new(ErrorKind::Closed, "read loop gone"))
            }
            Ok(Ok(Err(err))) => {
                self.inner.pending.lock().unwrap().remove(&id);
                Err(err)
            }
            Ok(Ok(Ok(()))) => Ok(()),
        }?;
        match tokio::time::timeout_at(deadline, rx).await {
            Err(_) => {
                self.inner.pending.lock().unwrap().remove(&id);
                if self.inner.closed.load(Ordering::SeqCst) {
                    Err(Error::new(ErrorKind::Closed, "read loop ended"))
                } else {
                    Err(Error::new(
                        ErrorKind::Timeout,
                        format!("no response for {tag} ({id})"),
                    ))
                }
            }
            Ok(Err(_)) => Err(Error::new(ErrorKind::Closed, "read loop ended")),
            Ok(Ok(response)) => response,
        }
    }

    fn fail_all(&self, err: Error) {
        fail_shared(&self.inner, &err);
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        // Only the last handle ends the session. Survivors keep full use.
        if self.inner.handles.fetch_sub(1, Ordering::SeqCst) != 1 {
            return;
        }
        self.fail_all(Error::new(ErrorKind::Closed, "client dropped"));
        if let Ok(mut tasks) = self.inner.tasks.lock() {
            for task in tasks.drain(..) {
                task.abort();
            }
        }
    }
}

/// Owned event stream, decoupled from Tokio channel types.
///
/// Slow readers lag past the channel capacity and read a `Protocol` error
/// instead of the skipped events. Close wakes every waiter at once with
/// `Closed`, even with no further lines.
#[derive(Debug)]
pub struct EventStream {
    rx: broadcast::Receiver<Arc<Event>>,
    shutdown: watch::Receiver<bool>,
}

impl EventStream {
    /// Wait for the next event. Pending lines drain before close reports.
    pub async fn recv(&mut self) -> Result<Arc<Event>> {
        tokio::select! {
            biased;
            res = self.rx.recv() => res.map_err(map_broadcast),
            closed = self.shutdown.wait_for(|c| *c) => {
                let _ = closed;
                Err(Error::new(ErrorKind::Closed, "client closed"))
            }
        }
    }

    /// Take the next event without waiting. Empty reads `Ok(None)`.
    pub fn try_recv(&mut self) -> Result<Option<Arc<Event>>> {
        match self.rx.try_recv() {
            Ok(event) => Ok(Some(event)),
            Err(broadcast::error::TryRecvError::Empty) => Ok(None),
            Err(broadcast::error::TryRecvError::Lagged(skipped)) => Err(Error::new(
                ErrorKind::Protocol,
                format!("event receiver lagged by {skipped}"),
            )),
            Err(broadcast::error::TryRecvError::Closed) => {
                Err(Error::new(ErrorKind::Closed, "event stream closed"))
            }
        }
    }
}

fn map_broadcast(err: broadcast::error::RecvError) -> Error {
    match err {
        broadcast::error::RecvError::Lagged(skipped) => Error::new(
            ErrorKind::Protocol,
            format!("event receiver lagged by {skipped}"),
        ),
        broadcast::error::RecvError::Closed => Error::new(ErrorKind::Closed, "event stream closed"),
    }
}
/// Callback guard. Dropping removes the listener.
#[must_use = "dropping the guard at once unsubscribes"]
pub struct Subscription {
    inner: Arc<Inner>,
    id: u64,
}

impl std::fmt::Debug for Subscription {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Subscription")
            .field("id", &self.id)
            .finish()
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        if let Ok(mut callbacks) = self.inner.callbacks.lock() {
            callbacks.retain(|(id, _)| *id != self.id);
        }
    }
}

/// Reader task. Owns the read half and does nothing else, so a slow
/// send never stalls reads and no `recv` is ever cancelled mid-frame.
async fn read_lines<Rx: TransportRecv>(mut rx: Rx, inner: Arc<Inner>) {
    // The guard covers every exit path including panic unwind.
    let _guard = LoopEnd(inner.clone());
    loop {
        match rx.recv().await {
            Err(err) => {
                fail_shared(&inner, &err);
                break;
            }
            Ok(None) => {
                fail_shared(&inner, &Error::new(ErrorKind::Closed, "transport ended"));
                break;
            }
            Ok(Some(line)) => {
                // Oversized lines skip with a count. The full line is
                // already in hand, so dropping it keeps framing synced
                // without killing the stream.
                if line.len() > inner.config.max_line {
                    bump(&inner, |c| c.malformed += 1);
                    continue;
                }
                dispatch(&inner, &line).await;
            }
        }
    }
}

async fn run_loop<Tx: TransportSend>(
    mut tx: Tx,
    mut jobs: mpsc::UnboundedReceiver<TxJob>,
    inner: Arc<Inner>,
) {
    // Every exit path fails pending work and marks closed, including a
    // panic unwind past this frame. Normal breaks already drained the map,
    // so the guard is a no-op for them.
    let _guard = LoopEnd(inner.clone());
    while let Some(job) = jobs.recv().await {
        match job {
            TxJob::Send { line, done } => {
                let _ = done.send(tx.send(line).await);
            }
            TxJob::Close { done } => {
                let _ = done.send(tx.close().await);
                break;
            }
        }
    }
}

/// Best-effort id read for a broken response value.
fn response_id_value(value: &serde_json::Value) -> Option<String> {
    value.get("id")?.as_str().map(str::to_string)
}

fn bump(inner: &Arc<Inner>, f: impl FnOnce(&mut DropCounts)) {
    f(&mut inner.drops.lock().unwrap());
}

/// End-of-loop guard. Fires on every exit, panic unwind included.
struct LoopEnd(Arc<Inner>);

impl Drop for LoopEnd {
    fn drop(&mut self) {
        fail_shared(&self.0, &Error::new(ErrorKind::Closed, "loop ended"));
        self.0.closed.store(true, Ordering::SeqCst);
        self.0.shutdown.send_replace(true);
    }
}

fn fail_shared(inner: &Arc<Inner>, err: &Error) {
    let senders: Vec<oneshot::Sender<Result<Response>>> = inner
        .pending
        .lock()
        .unwrap()
        .drain()
        .map(|(_, tx)| tx)
        .collect();
    for tx in senders {
        let _ = tx.send(Err(err.clone()));
    }
}

async fn dispatch(inner: &Arc<Inner>, line: &str) {
    let value: serde_json::Value = match serde_json::from_str(line) {
        Ok(value) => value,
        Err(_) => {
            bump(inner, |c| c.malformed += 1);
            return;
        }
    };
    match value.get("type").and_then(|v| v.as_str()) {
        Some("response") => dispatch_response_value(inner, value),
        Some("extension_ui_request") => dispatch_ui_value(inner, value).await,
        Some(_) => dispatch_event_value(inner, value),
        None => bump(inner, |c| c.malformed += 1),
    }
}

fn dispatch_response_value(inner: &Arc<Inner>, value: serde_json::Value) {
    let id_hint = response_id_value(&value);
    let envelope = match crate::response::decode_envelope_value(value) {
        Ok(envelope) => envelope,
        Err(err) => {
            // The waiter can still use the real cause when the id parses.
            // Only id-less or foreign lines count as malformed drift.
            if let Some(id) = id_hint {
                if let Some(sender) = inner.pending.lock().unwrap().remove(&id) {
                    let _ = sender.send(Err(err));
                    return;
                }
            }
            bump(inner, |c| c.malformed += 1);
            return;
        }
    };
    let Some(id) = envelope.id.clone() else {
        bump(inner, |c| c.stray_response += 1);
        return;
    };
    let sender = inner.pending.lock().unwrap().remove(&id);
    if sender.is_none() {
        bump(inner, |c| c.stray_response += 1);
    }
    if let Some(sender) = sender {
        let _ = sender.send(Ok(envelope.body));
    }
}

async fn dispatch_ui_value(inner: &Arc<Inner>, value: serde_json::Value) {
    let id = match value.get("id").and_then(|v| v.as_str()) {
        Some(id) => id.to_string(),
        None => {
            bump(inner, |c| c.bad_ui += 1);
            return;
        }
    };
    let request = match crate::extui::decode_ui_request_value(value) {
        Ok(request) => request,
        Err(_) => {
            bump(inner, |c| c.bad_ui += 1);
            return;
        }
    };
    // The handler runs spawned: dialogs never stall the reader, so the
    // stream keeps flowing and reentrant client calls resolve. The answer
    // carries the request id, routing it back to its dialog.
    let worker = inner.clone();
    tokio::spawn(async move {
        // The handler answers under the wait timeout, like pi-side timeouts.
        // Expiry auto-dismisses.
        let answer = match tokio::time::timeout(
            worker.config.wait_timeout,
            answer_dialog(&worker, &request),
        )
        .await
        {
            Ok(answer) => answer,
            Err(_) => Some(UiAnswer::Cancelled),
        };
        if let Some(answer) = answer {
            if let Ok(line) = encode_ui_response(&id, &answer) {
                let (done_tx, _) = oneshot::channel();
                let _ = worker.tx.send(TxJob::Send {
                    line,
                    done: done_tx,
                });
            }
        }
    });
}

async fn answer_dialog(inner: &Arc<Inner>, request: &UiRequest) -> Option<UiAnswer> {
    let mut ui = inner.ui.lock().await;
    let Some(handler) = ui.as_mut() else {
        return Some(UiAnswer::Cancelled);
    };
    match request {
        UiRequest::Select(dialog) => match handler.select(dialog.clone()).await {
            UiOutcome::Value(value) => Some(UiAnswer::Value(value)),
            UiOutcome::Dismissed => Some(UiAnswer::Cancelled),
        },
        UiRequest::Confirm(dialog) => {
            Some(UiAnswer::Confirmed(handler.confirm(dialog.clone()).await))
        }
        UiRequest::Input(dialog) => match handler.input(dialog.clone()).await {
            UiOutcome::Value(value) => Some(UiAnswer::Value(value)),
            UiOutcome::Dismissed => Some(UiAnswer::Cancelled),
        },
        UiRequest::Editor(dialog) => match handler.editor(dialog.clone()).await {
            UiOutcome::Value(value) => Some(UiAnswer::Value(value)),
            UiOutcome::Dismissed => Some(UiAnswer::Cancelled),
        },
        UiRequest::Notify(notice) => {
            handler.notify(notice.clone()).await;
            None
        }
        UiRequest::SetStatus(entry) => {
            handler.set_status(entry.clone()).await;
            None
        }
        UiRequest::SetWidget(entry) => {
            handler.set_widget(entry.clone()).await;
            None
        }
        UiRequest::SetTitle(title) => {
            handler.set_title(title.clone()).await;
            None
        }
        UiRequest::SetEditorText(text) => {
            handler.set_editor_text(text.clone()).await;
            None
        }
        UiRequest::Unknown(_) => Some(UiAnswer::Cancelled),
    }
}

fn dispatch_event_value(inner: &Arc<Inner>, value: serde_json::Value) {
    let event = match crate::event::decode_event_value(value) {
        Ok(event) => std::sync::Arc::new(event),
        Err(_) => {
            bump(inner, |c| c.bad_event += 1);
            return;
        }
    };
    let has_broadcast = inner.events.receiver_count() > 0;
    let callbacks: Vec<Callback> = inner
        .callbacks
        .lock()
        .unwrap()
        .iter()
        .map(|(_, cb)| cb.clone())
        .collect();
    if !has_broadcast && callbacks.is_empty() {
        return;
    }
    // Callbacks run before broadcast sends, so a test that waits for a
    // later event observes every earlier callback effect deterministically.
    for callback in callbacks {
        // A panicking listener must not kill the loop. Upstream swallows
        // listener faults the same way; the count keeps it visible.
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            callback(event.clone());
        }));
        if caught.is_err() {
            bump(inner, |c| c.callback_panic += 1);
        }
    }
    if has_broadcast {
        let _ = inner.events.send(event);
    }
}

async fn recv_before(stream: &mut EventStream, deadline: Instant) -> Result<Arc<Event>> {
    match tokio::time::timeout_at(deadline, stream.recv()).await {
        Err(_) => Err(Error::new(ErrorKind::Timeout, "wait timed out")),
        Ok(result) => result,
    }
}

/// Fold text deltas of all blocks into one string, in block order.
///
/// Buffers key on message sequence plus block index, so a later assistant
/// message never clears an earlier one sharing the index.
fn fold_text(events: &[Arc<Event>]) -> String {
    let mut buffers: BTreeMap<(u64, u64), String> = BTreeMap::new();
    let mut seq = 0u64;
    for event in events {
        match &**event {
            Event::MessageEnd(message) => {
                if matches!(message.message, Message::Assistant(_)) {
                    seq += 1;
                }
            }
            Event::MessageUpdate(MessageUpdate { update, .. }) => match update {
                AssistantUpdate::TextStart(block) => {
                    buffers.insert((seq, block.content_index), String::new());
                }
                AssistantUpdate::TextDelta(delta) => {
                    buffers
                        .entry((seq, delta.content_index))
                        .or_default()
                        .push_str(&delta.delta);
                }
                AssistantUpdate::TextEnd(done) => {
                    buffers.insert((seq, done.content_index), done.content.clone());
                }
                _ => {}
            },
            _ => {}
        }
    }
    buffers.into_values().collect()
}

fn unexpected(command: &str, body: &Response) -> Error {
    Error::new(
        ErrorKind::Protocol,
        format!("wrong response for {command}: {body:?}"),
    )
}

/// Map a failed command to its error, keeping the pi message verbatim.
fn command_failed(failed: &crate::response::Failed) -> Error {
    Error::new(ErrorKind::Command, failed.error.clone())
}
