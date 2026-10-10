//! Stdout command responses with typed data.
//!
//! Source: pi `f1b2e77`
//! `packages/coding-agent/src/modes/rpc/rpc-types.ts:96-115`
//! (`RpcSessionState`), `:116-245` (`RpcResponse`).
//! `SessionStats` detail:
//! `packages/coding-agent/src/core/agent-session.ts:342-359`.
//! `BashResult` detail:
//! `packages/coding-agent/src/core/bash-executor.ts:27-38`.
//! `CompactionResult` detail:
//! `packages/coding-agent/src/core/compaction/compaction.ts:105-114`.

use serde::{Deserialize, Serialize};

use crate::entry::{Entry, TreeNode};
use crate::error::{Error, ErrorKind};
use crate::message::Message;
use crate::types::{
    ContextUsage, Model, PromptDisposition, QueueMode, QueuedInputDisposition, SlashCommand,
    ThinkingLevel, Usage,
};

/// Command tag for the failed command. Kept verbatim for routing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Failed {
    pub command: String,
    pub error: String,
}

/// Typed response for one command. Unknown commands never fail a read.
///
/// Wire conversion goes through [`decode_envelope`] and [`encode_envelope`];
/// this enum carries no serde shape of its own.
#[derive(Debug, Clone, PartialEq)]
pub enum Response {
    Prompt(PromptData),
    Steer(QueuedData),
    FollowUp(QueuedData),
    Abort,
    ClearQueue(ClearQueueData),
    NewSession(SessionSwitch),
    GetState(SessionState),
    SetModel(Model),
    CycleModel(Option<CycleModel>),
    GetAvailableModels(Vec<Model>),
    SetThinkingLevel,
    CycleThinkingLevel(Option<ThinkingLevelData>),
    GetAvailableThinkingLevels(Vec<ThinkingLevel>),
    SetSteeringMode,
    SetFollowUpMode,
    Compact(CompactionResult),
    SetAutoCompaction,
    SetAutoRetry,
    AbortRetry,
    Bash(BashResult),
    AbortBash,
    GetSessionStats(SessionStats),
    ExportHtml(ExportPath),
    SwitchSession(SessionSwitch),
    Fork(ForkData),
    Clone(SessionSwitch),
    GetForkMessages(Vec<ForkMessage>),
    GetEntries(EntryList),
    GetTree(TreeList),
    GetLastAssistantText(Option<String>),
    SetSessionName,
    GetMessages(Vec<Message>),
    GetCommands(Vec<SlashCommand>),
    Error(Failed),
    Unknown(UnknownResponse),
}

/// Raw record for commands this lib does not know yet.
#[derive(Debug, Clone, PartialEq)]
pub struct UnknownResponse {
    pub command: String,
    pub success: bool,
    pub data: Option<serde_json::Value>,
    pub error: Option<String>,
}

/// Outer envelope shared by every stdout response.
#[derive(Debug, Clone, PartialEq)]
pub struct Envelope {
    pub id: Option<String>,
    pub body: Response,
}

/// Acceptance report for a submitted prompt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PromptData {
    pub disposition: PromptDisposition,
}

/// Queue report for steer and follow-up inputs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueuedData {
    pub disposition: QueuedInputDisposition,
}

/// Queued texts returned by `clear_queue`, for Esc-style restore.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClearQueueData {
    pub steering: Vec<String>,
    pub follow_up: Vec<String>,
}

/// Cancel flag for session switch operations.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SessionSwitch {
    pub cancelled: bool,
}

/// Live session snapshot from `get_state`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<Model>,
    pub thinking_level: ThinkingLevel,
    pub is_streaming: bool,
    pub is_compacting: bool,
    pub steering_mode: QueueMode,
    pub follow_up_mode: QueueMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_file: Option<String>,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_name: Option<String>,
    pub auto_compaction_enabled: bool,
    pub message_count: u64,
    pub pending_message_count: u64,
}

/// Model pick plus level and scope flag from `cycle_model`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CycleModel {
    pub model: Model,
    pub thinking_level: ThinkingLevel,
    #[serde(rename = "isScoped")]
    pub is_scoped: bool,
}

/// Level picked by `cycle_thinking_level`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThinkingLevelData {
    pub level: ThinkingLevel,
}

/// Outcome of one compaction pass.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactionResult {
    pub summary: String,
    pub first_kept_entry_id: String,
    pub tokens_before: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_tokens_after: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

/// Final result of one direct shell command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BashResult {
    pub output: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<f64>,
    pub cancelled: bool,
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full_output_path: Option<String>,
}

/// Token and cost totals for the full session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStats {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_file: Option<String>,
    pub session_id: String,
    pub user_messages: u64,
    pub assistant_messages: u64,
    pub tool_calls: u64,
    pub tool_results: u64,
    pub total_messages: u64,
    pub tokens: TokenTotals,
    pub cost: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_usage: Option<ContextUsage>,
}

/// Flat token counters without cost split.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenTotals {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
    pub total: f64,
}

/// HTML export path.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportPath {
    pub path: String,
}

/// Fork source text plus cancel flag.
///
/// Cancelled runs omit `text`: the runtime returns `{ cancelled: true }`
/// with no text (`agent-session-runtime.ts:262-270`), and the wire drops
/// the key (`rpc-mode.ts:616`, doc `rpc-commands.md:628-635`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ForkData {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    pub cancelled: bool,
}

/// One forkable user message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForkMessage {
    pub entry_id: String,
    pub text: String,
}

/// Entries in append order plus current leaf cursor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryList {
    pub entries: Vec<Entry>,
    pub leaf_id: Option<String>,
}

/// Session tree plus current leaf cursor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeList {
    pub tree: Vec<TreeNode>,
    pub leaf_id: Option<String>,
}

/// Parse one stdout response line into envelope plus typed body.
pub fn decode_envelope(line: &str) -> Result<Envelope, Error> {
    let value: serde_json::Value = serde_json::from_str(line)
        .map_err(|err| Error::new(ErrorKind::Protocol, format!("bad response: {err}")))?;
    decode_envelope_value(value)
}

/// Decode one already-parsed response value. Saves a second parse on hot paths.
pub fn decode_envelope_value(value: serde_json::Value) -> Result<Envelope, Error> {
    let wire: WireResponse = serde_json::from_value(value)
        .map_err(|err| Error::new(ErrorKind::Protocol, format!("bad response: {err}")))?;
    if wire.kind != "response" {
        return Err(Error::new(
            ErrorKind::Protocol,
            format!("not a response record: {}", wire.kind),
        ));
    }
    let body = decode_body(&wire.command, wire.success, wire.data, wire.error)?;
    Ok(Envelope { id: wire.id, body })
}

/// Encode one envelope back to its exact wire shape.
pub fn encode_envelope(env: &Envelope) -> Result<String, Error> {
    let (command, success, data, error) = encode_body(&env.body)?;
    let mut obj = serde_json::Map::with_capacity(5);
    if let Some(id) = &env.id {
        obj.insert("id".to_string(), serde_json::Value::String(id.clone()));
    }
    obj.insert(
        "type".to_string(),
        serde_json::Value::String("response".to_string()),
    );
    obj.insert("command".to_string(), serde_json::Value::String(command));
    obj.insert("success".to_string(), serde_json::Value::Bool(success));
    if let Some(data) = data {
        obj.insert("data".to_string(), data);
    }
    if let Some(error) = error {
        obj.insert("error".to_string(), serde_json::Value::String(error));
    }
    crate::frame::encode_line(&serde_json::Value::Object(obj))
}

#[derive(Debug, Deserialize)]
struct WireResponse {
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    command: String,
    #[serde(default)]
    success: bool,
    #[serde(default)]
    data: Option<serde_json::Value>,
    #[serde(default)]
    error: Option<String>,
}

fn decode_body(
    command: &str,
    success: bool,
    data: Option<serde_json::Value>,
    error: Option<String>,
) -> Result<Response, Error> {
    use ErrorKind::Protocol;
    if !success {
        return Ok(Response::Error(Failed {
            command: command.to_string(),
            error: error.unwrap_or_else(|| "command failed".to_string()),
        }));
    }
    let bad = |err: serde_json::Error| Error::new(Protocol, format!("bad {command} data: {err}"));
    // Missing stays missing only on the unknown path below. Known shapes
    // decode from null, which fails on their required fields as before.
    // Each arm moves `data` on its own exclusive path, so no clone runs.
    let body = match command {
        "prompt" => Response::Prompt(
            serde_json::from_value(data.unwrap_or(serde_json::Value::Null)).map_err(bad)?,
        ),
        "steer" => Response::Steer(
            serde_json::from_value(data.unwrap_or(serde_json::Value::Null)).map_err(bad)?,
        ),
        "follow_up" => Response::FollowUp(
            serde_json::from_value(data.unwrap_or(serde_json::Value::Null)).map_err(bad)?,
        ),
        "abort" => Response::Abort,
        "clear_queue" => Response::ClearQueue(
            serde_json::from_value(data.unwrap_or(serde_json::Value::Null)).map_err(bad)?,
        ),
        "new_session" => Response::NewSession(
            serde_json::from_value(data.unwrap_or(serde_json::Value::Null)).map_err(bad)?,
        ),
        "get_state" => Response::GetState(
            serde_json::from_value(data.unwrap_or(serde_json::Value::Null)).map_err(bad)?,
        ),
        "set_model" => Response::SetModel(
            serde_json::from_value(data.unwrap_or(serde_json::Value::Null)).map_err(bad)?,
        ),
        "cycle_model" => Response::CycleModel(
            serde_json::from_value(data.unwrap_or(serde_json::Value::Null)).map_err(bad)?,
        ),
        "get_available_models" => Response::GetAvailableModels(
            serde_json::from_value::<ModelList>(data.unwrap_or(serde_json::Value::Null))
                .map_err(bad)?
                .models,
        ),
        "set_thinking_level" => Response::SetThinkingLevel,
        "cycle_thinking_level" => Response::CycleThinkingLevel(
            serde_json::from_value(data.unwrap_or(serde_json::Value::Null)).map_err(bad)?,
        ),
        "get_available_thinking_levels" => Response::GetAvailableThinkingLevels(
            serde_json::from_value::<LevelList>(data.unwrap_or(serde_json::Value::Null))
                .map_err(bad)?
                .levels,
        ),
        "set_steering_mode" => Response::SetSteeringMode,
        "set_follow_up_mode" => Response::SetFollowUpMode,
        "compact" => Response::Compact(
            serde_json::from_value(data.unwrap_or(serde_json::Value::Null)).map_err(bad)?,
        ),
        "set_auto_compaction" => Response::SetAutoCompaction,
        "set_auto_retry" => Response::SetAutoRetry,
        "abort_retry" => Response::AbortRetry,
        "bash" => Response::Bash(
            serde_json::from_value(data.unwrap_or(serde_json::Value::Null)).map_err(bad)?,
        ),
        "abort_bash" => Response::AbortBash,
        "get_session_stats" => Response::GetSessionStats(
            serde_json::from_value(data.unwrap_or(serde_json::Value::Null)).map_err(bad)?,
        ),
        "export_html" => Response::ExportHtml(
            serde_json::from_value(data.unwrap_or(serde_json::Value::Null)).map_err(bad)?,
        ),
        "switch_session" => Response::SwitchSession(
            serde_json::from_value(data.unwrap_or(serde_json::Value::Null)).map_err(bad)?,
        ),
        "fork" => Response::Fork(
            serde_json::from_value(data.unwrap_or(serde_json::Value::Null)).map_err(bad)?,
        ),
        "clone" => Response::Clone(
            serde_json::from_value(data.unwrap_or(serde_json::Value::Null)).map_err(bad)?,
        ),
        "get_fork_messages" => Response::GetForkMessages(
            serde_json::from_value::<ForkList>(data.unwrap_or(serde_json::Value::Null))
                .map_err(bad)?
                .messages,
        ),
        "get_entries" => Response::GetEntries(
            serde_json::from_value(data.unwrap_or(serde_json::Value::Null)).map_err(bad)?,
        ),
        "get_tree" => Response::GetTree(
            serde_json::from_value(data.unwrap_or(serde_json::Value::Null)).map_err(bad)?,
        ),
        "get_last_assistant_text" => Response::GetLastAssistantText(
            serde_json::from_value::<LastText>(data.unwrap_or(serde_json::Value::Null))
                .map_err(bad)?
                .text,
        ),
        "set_session_name" => Response::SetSessionName,
        "get_messages" => Response::GetMessages(
            serde_json::from_value::<MessageList>(data.unwrap_or(serde_json::Value::Null))
                .map_err(bad)?
                .messages,
        ),
        "get_commands" => Response::GetCommands(
            serde_json::from_value::<CommandList>(data.unwrap_or(serde_json::Value::Null))
                .map_err(bad)?
                .commands,
        ),
        other => Response::Unknown(UnknownResponse {
            command: other.to_string(),
            success: true,
            data,
            error: None,
        }),
    };
    Ok(body)
}

fn to_wire<T: Serialize>(value: &T) -> Result<serde_json::Value, Error> {
    serde_json::to_value(value)
        .map_err(|err| Error::new(ErrorKind::Protocol, format!("bad response data: {err}")))
}

fn encode_body(
    body: &Response,
) -> Result<(String, bool, Option<serde_json::Value>, Option<String>), Error> {
    let pair = match body {
        Response::Prompt(v) => ("prompt", true, Some(to_wire(v)?), None),
        Response::Steer(v) => ("steer", true, Some(to_wire(v)?), None),
        Response::FollowUp(v) => ("follow_up", true, Some(to_wire(v)?), None),
        Response::Abort => ("abort", true, None, None),
        Response::ClearQueue(v) => ("clear_queue", true, Some(to_wire(v)?), None),
        Response::NewSession(v) => ("new_session", true, Some(to_wire(v)?), None),
        Response::GetState(v) => ("get_state", true, Some(to_wire(v)?), None),
        Response::SetModel(v) => ("set_model", true, Some(to_wire(v)?), None),
        Response::CycleModel(v) => ("cycle_model", true, Some(to_wire(v)?), None),
        Response::GetAvailableModels(v) => (
            "get_available_models",
            true,
            Some(to_wire(&ModelList { models: v.clone() })?),
            None,
        ),
        Response::SetThinkingLevel => ("set_thinking_level", true, None, None),
        Response::CycleThinkingLevel(v) => ("cycle_thinking_level", true, Some(to_wire(v)?), None),
        Response::GetAvailableThinkingLevels(v) => (
            "get_available_thinking_levels",
            true,
            Some(to_wire(&LevelList { levels: v.clone() })?),
            None,
        ),
        Response::SetSteeringMode => ("set_steering_mode", true, None, None),
        Response::SetFollowUpMode => ("set_follow_up_mode", true, None, None),
        Response::Compact(v) => ("compact", true, Some(to_wire(v)?), None),
        Response::SetAutoCompaction => ("set_auto_compaction", true, None, None),
        Response::SetAutoRetry => ("set_auto_retry", true, None, None),
        Response::AbortRetry => ("abort_retry", true, None, None),
        Response::Bash(v) => ("bash", true, Some(to_wire(v)?), None),
        Response::AbortBash => ("abort_bash", true, None, None),
        Response::GetSessionStats(v) => ("get_session_stats", true, Some(to_wire(v)?), None),
        Response::ExportHtml(v) => ("export_html", true, Some(to_wire(v)?), None),
        Response::SwitchSession(v) => ("switch_session", true, Some(to_wire(v)?), None),
        Response::Fork(v) => ("fork", true, Some(to_wire(v)?), None),
        Response::Clone(v) => ("clone", true, Some(to_wire(v)?), None),
        Response::GetForkMessages(v) => (
            "get_fork_messages",
            true,
            Some(to_wire(&ForkList {
                messages: v.clone(),
            })?),
            None,
        ),
        Response::GetEntries(v) => {
            let mut entries = Vec::with_capacity(v.entries.len());
            for entry in &v.entries {
                entries.push(crate::entry::encode_entry_value(entry)?);
            }
            let mut data = serde_json::Map::new();
            data.insert("entries".to_string(), serde_json::Value::Array(entries));
            data.insert("leafId".to_string(), v.leaf_id.clone().into());
            (
                "get_entries",
                true,
                Some(serde_json::Value::Object(data)),
                None,
            )
        }
        Response::GetTree(v) => {
            let mut tree = Vec::with_capacity(v.tree.len());
            for node in &v.tree {
                tree.push(crate::entry::encode_tree_node_value(node)?);
            }
            let mut data = serde_json::Map::new();
            data.insert("tree".to_string(), serde_json::Value::Array(tree));
            data.insert("leafId".to_string(), v.leaf_id.clone().into());
            (
                "get_tree",
                true,
                Some(serde_json::Value::Object(data)),
                None,
            )
        }
        Response::GetLastAssistantText(v) => (
            "get_last_assistant_text",
            true,
            Some(to_wire(&LastText { text: v.clone() })?),
            None,
        ),
        Response::SetSessionName => ("set_session_name", true, None, None),
        Response::GetMessages(v) => {
            let mut messages = Vec::with_capacity(v.len());
            for message in v {
                messages.push(crate::message::encode_message_value(message)?);
            }
            let mut data = serde_json::Map::new();
            data.insert("messages".to_string(), serde_json::Value::Array(messages));
            (
                "get_messages",
                true,
                Some(serde_json::Value::Object(data)),
                None,
            )
        }
        Response::GetCommands(v) => (
            "get_commands",
            true,
            Some(to_wire(&CommandList {
                commands: v.clone(),
            })?),
            None,
        ),
        Response::Error(v) => (v.command.as_str(), false, None, Some(v.error.clone())),
        Response::Unknown(v) => (
            v.command.as_str(),
            v.success,
            v.data.clone(),
            v.error.clone(),
        ),
    };
    Ok((pair.0.to_string(), pair.1, pair.2, pair.3))
}

#[derive(Debug, Deserialize, Serialize)]
struct ModelList {
    models: Vec<Model>,
}

#[derive(Debug, Deserialize, Serialize)]
struct LevelList {
    levels: Vec<ThinkingLevel>,
}

#[derive(Debug, Deserialize, Serialize)]
struct ForkList {
    messages: Vec<ForkMessage>,
}

#[derive(Debug, Deserialize, Serialize)]
struct LastText {
    text: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct MessageList {
    messages: Vec<Message>,
}

#[derive(Debug, Deserialize, Serialize)]
struct CommandList {
    commands: Vec<SlashCommand>,
}
