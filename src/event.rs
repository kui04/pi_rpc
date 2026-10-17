//! Stdout session events with delta-only streaming updates.
//!
//! Source: pi `v1.3.0`
//! `packages/agent/src/types.ts:516-539` (`AgentEvent`);
//! `packages/coding-agent/src/core/agent-session.ts:188-193`
//! (parent tool id), `:196-245` (session events);
//! `packages/coding-agent/src/modes/json-event.ts:18` (wire shape),
//! `:48-62` (`toJsonEvent` delta strip);
//! `packages/ai/src/types.ts:801-817` (assistant update variants).
//! Tables: `packages/coding-agent/docs/json.md:50-110` (core events),
//! `:182-197` (RPC-only events).

use serde::{Deserialize, Serialize};

use crate::entry::Entry;
use crate::error::{Error, ErrorKind};
use crate::message::{AssistantMessage, Message, ToolCall, ToolResultMessage};
use crate::response::CompactionResult;
use crate::types::{RawRecord, StopReason, ThinkingLevel, Usage};

/// One stdout session event.
///
/// Deltas rebuild live text: buffer `delta` fields by `content_index`,
/// replace with `*_end` content, then trust `message_end`. A run is done
/// only at `agent_settled`, not at `agent_end`.
///
/// Unknown kinds keep their raw record and replay it verbatim everywhere,
/// including inside message-carrying payloads.
///
/// The derived `Serialize` is lossy for unknowns (it renames the tag).
/// The `encode_event_value` twin is the faithful path; use it to replay.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    AgentStart,
    AgentEnd(AgentEnd),
    AgentSettled(Settled),
    TurnStart,
    TurnEnd(TurnEnd),
    MessageStart(BoundedMessage),
    MessageUpdate(MessageUpdate),
    MessageEnd(BoundedMessage),
    ToolExecutionStart(ToolBound),
    ToolExecutionUpdate(ToolUpdate),
    ToolExecutionEnd(ToolEnd),
    QueueUpdate(QueueState),
    EntryAppended(AppendedEntry),
    SessionInfoChanged(SessionName),
    ThinkingLevelChanged(LevelChanged),
    CompactionStart(CompactionBound),
    CompactionEnd(CompactionDone),
    AutoRetryStart(RetryStart),
    AutoRetryEnd(RetryEnd),
    SummarizationRetryScheduled(RetryStart),
    SummarizationRetryAttemptStart(RetryAttempt),
    SummarizationRetryFinished,
    BashExecutionUpdate(BashChunk),
    ExtensionError(ExtensionFault),
    Unknown(RawRecord),
}

impl Event {
    /// Short tag for log fields. Known variants use snake_case; the
    /// catch-all carries the wire string verbatim, so a new pi event
    /// stays visible instead of collapsing to one bucket. Mirrors the
    /// decode table in [`decode_event_value`]; keep both in one edit.
    pub fn tag(&self) -> &str {
        match self {
            Event::AgentStart => "agent_start",
            Event::AgentEnd(_) => "agent_end",
            Event::AgentSettled(_) => "agent_settled",
            Event::TurnStart => "turn_start",
            Event::TurnEnd(_) => "turn_end",
            Event::MessageStart(_) => "message_start",
            Event::MessageUpdate(_) => "message_update",
            Event::MessageEnd(_) => "message_end",
            Event::ToolExecutionStart(_) => "tool_execution_start",
            Event::ToolExecutionUpdate(_) => "tool_execution_update",
            Event::ToolExecutionEnd(_) => "tool_execution_end",
            Event::QueueUpdate(_) => "queue_update",
            Event::EntryAppended(_) => "entry_appended",
            Event::SessionInfoChanged(_) => "session_info_changed",
            Event::ThinkingLevelChanged(_) => "thinking_level_changed",
            Event::CompactionStart(_) => "compaction_start",
            Event::CompactionEnd(_) => "compaction_end",
            Event::AutoRetryStart(_) => "auto_retry_start",
            Event::AutoRetryEnd(_) => "auto_retry_end",
            Event::SummarizationRetryScheduled(_) => "summarization_retry_scheduled",
            Event::SummarizationRetryAttemptStart(_) => "summarization_retry_attempt_start",
            Event::SummarizationRetryFinished => "summarization_retry_finished",
            Event::BashExecutionUpdate(_) => "bash_execution_update",
            Event::ExtensionError(_) => "extension_error",
            Event::Unknown(record) => record.kind.as_str(),
        }
    }
}

impl<'de> Deserialize<'de> for Event {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        decode_event_value(value).map_err(serde::de::Error::custom)
    }
}

/// Decode one already-parsed event value. Unknown tags keep raw payload.
pub fn decode_event_value(value: serde_json::Value) -> Result<Event, Error> {
    let tag = value
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if tag.is_empty() {
        return Err(Error::new(ErrorKind::Protocol, "event lacks type"));
    }
    let bad =
        |err: serde_json::Error| Error::new(ErrorKind::Protocol, format!("bad {tag} event: {err}"));
    let event = match tag.as_str() {
        "agent_start" => Event::AgentStart,
        "agent_end" => Event::AgentEnd(serde_json::from_value(value).map_err(bad)?),
        "agent_settled" => Event::AgentSettled(serde_json::from_value(value).map_err(bad)?),
        "turn_start" => Event::TurnStart,
        "turn_end" => Event::TurnEnd(serde_json::from_value(value).map_err(bad)?),
        "message_start" => Event::MessageStart(serde_json::from_value(value).map_err(bad)?),
        "message_update" => Event::MessageUpdate(serde_json::from_value(value).map_err(bad)?),
        "message_end" => Event::MessageEnd(serde_json::from_value(value).map_err(bad)?),
        "tool_execution_start" => {
            Event::ToolExecutionStart(serde_json::from_value(value).map_err(bad)?)
        }
        "tool_execution_update" => {
            Event::ToolExecutionUpdate(serde_json::from_value(value).map_err(bad)?)
        }
        "tool_execution_end" => {
            Event::ToolExecutionEnd(serde_json::from_value(value).map_err(bad)?)
        }
        "queue_update" => Event::QueueUpdate(serde_json::from_value(value).map_err(bad)?),
        "entry_appended" => Event::EntryAppended(serde_json::from_value(value).map_err(bad)?),
        "session_info_changed" => {
            Event::SessionInfoChanged(serde_json::from_value(value).map_err(bad)?)
        }
        "thinking_level_changed" => {
            Event::ThinkingLevelChanged(serde_json::from_value(value).map_err(bad)?)
        }
        "compaction_start" => Event::CompactionStart(serde_json::from_value(value).map_err(bad)?),
        "compaction_end" => Event::CompactionEnd(serde_json::from_value(value).map_err(bad)?),
        "auto_retry_start" => Event::AutoRetryStart(serde_json::from_value(value).map_err(bad)?),
        "auto_retry_end" => Event::AutoRetryEnd(serde_json::from_value(value).map_err(bad)?),
        "summarization_retry_scheduled" => {
            Event::SummarizationRetryScheduled(serde_json::from_value(value).map_err(bad)?)
        }
        "summarization_retry_attempt_start" => {
            Event::SummarizationRetryAttemptStart(serde_json::from_value(value).map_err(bad)?)
        }
        "summarization_retry_finished" => Event::SummarizationRetryFinished,
        "bash_execution_update" => {
            Event::BashExecutionUpdate(serde_json::from_value(value).map_err(bad)?)
        }
        "extension_error" => Event::ExtensionError(serde_json::from_value(value).map_err(bad)?),
        _ => Event::Unknown(RawRecord::new("type", value)),
    };
    Ok(event)
}

/// Messages produced by one low-level run plus retry flag.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentEnd {
    pub messages: Vec<Message>,
    pub will_retry: bool,
}

/// No automatic work remains for this run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settled {
    pub aborted: bool,
}

/// One assistant turn plus its tool results.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnEnd {
    pub message: Message,
    pub tool_results: Vec<ToolResultMessage>,
}

/// Single authoritative message carrier.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoundedMessage {
    pub message: Message,
}

/// Delta-only assistant update with latest use.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageUpdate {
    pub usage: Usage,
    #[serde(rename = "assistantMessageEvent")]
    pub update: AssistantUpdate,
}

/// Streaming update for one content block.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AssistantUpdate {
    Start,
    TextStart(BlockRef),
    TextDelta(TextDelta),
    TextEnd(TextDone),
    ThinkingStart(BlockRef),
    ThinkingDelta(TextDelta),
    ThinkingEnd(TextDone),
    ToolcallStart(ToolcallStart),
    ToolcallDelta(ToolcallDelta),
    ToolcallEnd(ToolcallEnd),
    Done(StreamDone),
    Error(StreamError),
}

/// Content block index for delta routing.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockRef {
    pub content_index: u64,
}

/// Text appended to one block.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextDelta {
    pub content_index: u64,
    pub delta: String,
}

/// Authoritative block content replacing buffered deltas.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextDone {
    pub content_index: u64,
    pub content: String,
}

/// New tool call with id and name up front.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolcallStart {
    pub content_index: u64,
    pub id: String,
    pub tool_name: String,
}

/// Serialized argument bytes appended to one call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolcallDelta {
    pub content_index: u64,
    pub delta: String,
}

/// Complete tool call replacing buffered argument bytes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolcallEnd {
    pub content_index: u64,
    pub tool_call: ToolCall,
}

/// Provider stream end with final message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StreamDone {
    pub reason: StopReason,
    pub message: AssistantMessage,
}

/// Provider stream failure with final message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StreamError {
    pub reason: StopReason,
    pub error: AssistantMessage,
}

/// Tool call start with opaque arguments.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolBound {
    #[serde(rename = "toolCallId")]
    pub tool_call_id: String,
    #[serde(rename = "toolName")]
    pub tool_name: String,
    pub args: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(rename = "parentToolCallId")]
    pub parent_tool_call_id: Option<String>,
}

/// Partial tool result. Replace or extend per tool contract.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolUpdate {
    #[serde(rename = "toolCallId")]
    pub tool_call_id: String,
    #[serde(rename = "toolName")]
    pub tool_name: String,
    pub args: serde_json::Value,
    #[serde(rename = "partialResult")]
    pub partial_result: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(rename = "parentToolCallId")]
    pub parent_tool_call_id: Option<String>,
}

/// Final tool result with error flag and own duration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolEnd {
    #[serde(rename = "toolCallId")]
    pub tool_call_id: String,
    #[serde(rename = "toolName")]
    pub tool_name: String,
    pub result: serde_json::Value,
    pub is_error: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(rename = "parentToolCallId")]
    pub parent_tool_call_id: Option<String>,
}

/// Full pending steering and follow-up queues.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueState {
    pub steering: Vec<String>,
    pub follow_up: Vec<String>,
}

/// Custom session entry appended by an extension.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppendedEntry {
    pub entry: Entry,
}

/// Display name change. Absent name means cleared.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionName {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Active thinking level change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LevelChanged {
    pub level: ThinkingLevel,
}

/// Why compaction began.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompactionReason {
    Manual,
    Threshold,
    Overflow,
    /// Future reason. Reads widen; writes send the string back verbatim.
    Other(String),
}

crate::wide_string_enum!(CompactionReason {
    "manual" => Manual,
    "threshold" => Threshold,
    "overflow" => Overflow,
});

/// Compaction start with its reason.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompactionBound {
    pub reason: CompactionReason,
}

/// Compaction outcome. No result means aborted or failed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactionDone {
    pub reason: CompactionReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<CompactionResult>,
    pub aborted: bool,
    pub will_retry: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

/// Retry pause with attempt counters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetryStart {
    pub attempt: u64,
    #[serde(rename = "maxAttempts")]
    pub max_attempts: u64,
    #[serde(rename = "delayMs")]
    pub delay_ms: u64,
    #[serde(rename = "errorMessage")]
    pub error_message: String,
}

/// Retry finish with final error on failure.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetryEnd {
    pub success: bool,
    pub attempt: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(rename = "finalError")]
    pub final_error: Option<String>,
}

/// Which summarization path a retry belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetrySource {
    BranchSummary,
    Compaction,
    /// Future source. Reads widen; writes send the string back verbatim.
    Other(String),
}

crate::wide_string_enum!(RetrySource {
    "branchSummary" => BranchSummary,
    "compaction" => Compaction,
});

/// Summarization retry attempt with source and reason.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetryAttempt {
    pub source: RetrySource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<CompactionReason>,
}

/// One direct-bash output chunk. Id matches the command id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BashChunk {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub delta: String,
}

/// Extension handler fault with origin and event name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionFault {
    pub extension_path: String,
    pub event: String,
    pub error: String,
}

/// Parse one stdout event line. Unknown tags keep their raw record.
pub fn decode_event(line: &str) -> Result<Event, Error> {
    let value: serde_json::Value = serde_json::from_str(line)
        .map_err(|err| Error::new(ErrorKind::Protocol, format!("bad event: {err}")))?;
    decode_event_value(value)
}

/// Encode one event back to its exact wire shape.
///
/// Unknown kinds replay their stored record verbatim. Message-carrying
/// structs encode each message through its twin, so nested unknown roles
/// replay verbatim too.
pub fn encode_event(event: &Event) -> Result<String, Error> {
    crate::frame::encode_line(&encode_event_value(event)?)
}

/// Encode one already-split event value with verbatim unknowns.
pub fn encode_event_value(event: &Event) -> Result<serde_json::Value, Error> {
    let bad = |err: serde_json::Error| Error::new(ErrorKind::Protocol, format!("bad event: {err}"));
    let messages = |list: &[crate::message::Message]| {
        list.iter()
            .map(crate::message::encode_message_value)
            .collect::<Result<Vec<_>, _>>()
    };
    match event {
        Event::Unknown(raw) => Ok(raw.replay("type")),
        Event::AgentEnd(end) => {
            let mut out = serde_json::Map::new();
            out.insert("type".to_string(), "agent_end".into());
            out.insert(
                "messages".to_string(),
                serde_json::Value::Array(messages(&end.messages)?),
            );
            out.insert("willRetry".to_string(), end.will_retry.into());
            Ok(serde_json::Value::Object(out))
        }
        Event::TurnEnd(end) => {
            let mut out = serde_json::Map::new();
            out.insert("type".to_string(), "turn_end".into());
            out.insert(
                "message".to_string(),
                crate::message::encode_message_value(&end.message)?,
            );
            out.insert(
                "toolResults".to_string(),
                serde_json::to_value(&end.tool_results).map_err(bad)?,
            );
            Ok(serde_json::Value::Object(out))
        }
        Event::MessageStart(start) => {
            let mut out = serde_json::Map::new();
            out.insert("type".to_string(), "message_start".into());
            out.insert(
                "message".to_string(),
                crate::message::encode_message_value(&start.message)?,
            );
            Ok(serde_json::Value::Object(out))
        }
        Event::MessageEnd(end) => {
            let mut out = serde_json::Map::new();
            out.insert("type".to_string(), "message_end".into());
            out.insert(
                "message".to_string(),
                crate::message::encode_message_value(&end.message)?,
            );
            Ok(serde_json::Value::Object(out))
        }
        Event::EntryAppended(appended) => {
            let mut out = serde_json::Map::new();
            out.insert("type".to_string(), "entry_appended".into());
            out.insert(
                "entry".to_string(),
                crate::entry::encode_entry_value(&appended.entry)?,
            );
            Ok(serde_json::Value::Object(out))
        }
        _ => serde_json::to_value(event).map_err(bad),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settled_and_end_parse() {
        let settled: Event =
            serde_json::from_str(r#"{"type":"agent_settled","aborted":false}"#).unwrap();
        assert!(matches!(settled, Event::AgentSettled(_)));
        let start: Event = serde_json::from_str(r#"{"type":"agent_start"}"#).unwrap();
        assert!(matches!(start, Event::AgentStart));
    }

    #[test]
    fn delta_update_parses_without_snapshot() {
        let wire = r#"{"type":"message_update","usage":{"input":100,"output":1,"cacheRead":0,"cacheWrite":0,"totalTokens":101,"cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}},"assistantMessageEvent":{"type":"text_delta","contentIndex":0,"delta":"Hello "}}"#;
        let event: Event = serde_json::from_str(wire).unwrap();
        assert!(matches!(
            event,
            Event::MessageUpdate(MessageUpdate {
                update: AssistantUpdate::TextDelta(_),
                ..
            })
        ));
    }

    #[test]
    fn unknown_event_maps_to_catch_all() {
        let event: Event = serde_json::from_str(r#"{"type":"future_event","x":1}"#).unwrap();
        assert!(matches!(event, Event::Unknown(_)));
    }
}
