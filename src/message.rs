//! Conversation messages carried by responses and session entries.
//!
//! Source: pi `v1.3.0`
//! `packages/coding-agent/docs/message-types.md:13-65` (blocks),
//! `:67-90` (`Usage`), `:92-195` (base messages),
//! `:196-261` (coding-agent roles), `:263-280` (union).

use serde::{Deserialize, Serialize};

use crate::error::{Error, ErrorKind};
use crate::types::{ImageContent, RawRecord, StopReason, ThinkingLevel, Usage};

/// Plain text with opaque provider metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextContent {
    #[serde(rename = "type")]
    pub kind: TextKind,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_signature: Option<String>,
}

/// Marker that this block is text.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TextKind {
    Text,
}

/// Model thinking with opaque replay payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThinkingContent {
    #[serde(rename = "type")]
    pub kind: ThinkingKind,
    #[serde(default)]
    pub thinking: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_signature: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redacted: Option<bool>,
}

/// Marker that this block holds thinking.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThinkingKind {
    Thinking,
}

/// One tool invocation inside an assistant message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCall {
    #[serde(rename = "type")]
    pub kind: ToolCallKind,
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Map<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thought_signature: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
}

/// Marker that this block is a tool call.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ToolCallKind {
    ToolCall,
}

/// Content blocks inside assistant messages, told apart by inner type.
///
/// Future block types land in `Other` with their raw value and replay
/// verbatim, so one new block never fails the whole message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AssistantBlock {
    Text(TextContent),
    Thinking(ThinkingContent),
    ToolCall(ToolCall),
    Other(serde_json::Value),
}

/// Content blocks inside user and result messages, told apart by inner type.
///
/// Future block types land in `Other` with their raw value and replay
/// verbatim, so one new block never fails the whole message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum UserBlock {
    Text(TextContent),
    Image(ImageContent),
    Other(serde_json::Value),
}

/// Provider data to fetch a deferred response later.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeferredHandle {
    pub provider: String,
    pub model_id: String,
    pub api: String,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poll_after_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

/// Bounded record of one nested tool call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NestedToolCall {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<serde_json::Map<String, serde_json::Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments_bytes: Option<f64>,
    pub status: NestedStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Outcome of one nested tool call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NestedStatus {
    Ok,
    Error,
    Unfinished,
    /// Future status. Reads widen; writes send the string back verbatim.
    Other(String),
}

crate::wide_string_enum!(NestedStatus {
    "ok" => Ok,
    "error" => Error,
    "unfinished" => Unfinished,
});

/// System prompt state: sections plus tool loadout patches.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemMessage {
    pub content: SystemContent,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sections: Option<std::collections::HashMap<String, Option<String>>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools_added: Option<Vec<serde_json::Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools_removed: Option<Vec<serde_json::Value>>,
    pub timestamp: f64,
}

/// System content is plain text or text blocks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SystemContent {
    Text(String),
    Blocks(Vec<TextContent>),
}

/// User content is plain text or text/image blocks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum UserContent {
    Text(String),
    Blocks(Vec<UserBlock>),
}

/// One assistant turn with provider details and use.
///
/// Optional replay fields mirror `AssistantMessage` (`ai/types.ts:565-596`)
/// so re-encode keeps them: response and provider ids, thinking levels,
/// diagnostics, stop detail, turn flag, and duration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantMessage {
    pub content: Vec<AssistantBlock>,
    pub api: String,
    pub provider: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_thinking_level: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_level: Option<ThinkingLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<serde_json::Value>,
    pub usage: Usage,
    pub stop_reason: StopReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deferred: Option<DeferredHandle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_stop_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_turn: Option<bool>,
    pub timestamp: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<f64>,
}

/// Tool result with optional nested-model use.
///
/// Shapes mirror `ToolResultMessage` (`ai/types.ts:620-637`), including
/// `durationMs`, which the doc page omits.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolResultMessage {
    pub tool_call_id: String,
    pub tool_name: String,
    pub content: Vec<UserBlock>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nested_calls: Option<NestedCalls>,
    pub is_error: bool,
    pub timestamp: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<f64>,
}

/// Bounded list of nested calls with a completeness flag.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NestedCalls {
    pub calls: Vec<NestedToolCall>,
    pub complete: bool,
}

/// Direct shell result stored outside LLM tool flow.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BashExecutionMessage {
    pub command: String,
    pub output: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<f64>,
    pub cancelled: bool,
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full_output_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude_from_context: Option<bool>,
    pub timestamp: f64,
}

/// Extension context message with display flag.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomMessage {
    pub custom_type: String,
    pub content: UserContent,
    pub display: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
    pub timestamp: f64,
}

/// Checkpoint of an abandoned branch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchSummaryMessage {
    pub summary: String,
    pub from_id: Option<String>,
    pub timestamp: f64,
}

/// Checkpoint of compacted context.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactionSummaryMessage {
    pub summary: String,
    pub tokens_before: f64,
    pub timestamp: f64,
}

/// Full conversation message union.
///
/// Unknown roles keep their raw record and replay it verbatim everywhere,
/// including inside entries, lists, and event payloads.
///
/// The derived `Serialize` is lossy for unknowns (it renames the tag).
/// The `encode_message_value` twin is the faithful path; use it to replay.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[expect(
    clippy::large_enum_variant,
    reason = "decodes once per record; boxing buys indirection"
)]
#[serde(tag = "role", rename_all = "camelCase")]
pub enum Message {
    System(SystemMessage),
    User(UserMessage),
    Assistant(AssistantMessage),
    ToolResult(ToolResultMessage),
    BashExecution(BashExecutionMessage),
    Custom(CustomMessage),
    BranchSummary(BranchSummaryMessage),
    CompactionSummary(CompactionSummaryMessage),
    Unknown(RawRecord),
}

impl<'de> Deserialize<'de> for Message {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        decode_message_value(value).map_err(serde::de::Error::custom)
    }
}

/// Decode one already-parsed message value. Unknown roles keep raw payload.
pub fn decode_message_value(value: serde_json::Value) -> Result<Message, Error> {
    let role = value
        .get("role")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if role.is_empty() {
        return Err(Error::new(ErrorKind::Protocol, "message lacks role"));
    }
    let bad = |err: serde_json::Error| {
        Error::new(ErrorKind::Protocol, format!("bad {role} message: {err}"))
    };
    let message = match role.as_str() {
        "system" => Message::System(serde_json::from_value(value).map_err(bad)?),
        "user" => Message::User(serde_json::from_value(value).map_err(bad)?),
        "assistant" => Message::Assistant(serde_json::from_value(value).map_err(bad)?),
        "toolResult" => Message::ToolResult(serde_json::from_value(value).map_err(bad)?),
        "bashExecution" => Message::BashExecution(serde_json::from_value(value).map_err(bad)?),
        "custom" => Message::Custom(serde_json::from_value(value).map_err(bad)?),
        "branchSummary" => Message::BranchSummary(serde_json::from_value(value).map_err(bad)?),
        "compactionSummary" => {
            Message::CompactionSummary(serde_json::from_value(value).map_err(bad)?)
        }
        _ => Message::Unknown(RawRecord::new("role", value)),
    };
    Ok(message)
}

/// User turn with text or blocks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserMessage {
    pub content: UserContent,
    pub timestamp: f64,
}

/// Encode one message back to its exact wire shape.
///
/// Unknown roles replay their stored record verbatim.
pub fn encode_message_value(message: &Message) -> Result<serde_json::Value, Error> {
    match message {
        Message::Unknown(raw) => Ok(raw.replay("role")),
        _ => serde_json::to_value(message)
            .map_err(|err| Error::new(ErrorKind::Protocol, format!("bad message: {err}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assistant_wire_roundtrips() {
        let wire = r#"{"role":"assistant","content":[{"type":"text","text":"Hi"}],"api":"anthropic-messages","provider":"anthropic","model":"m","usage":{"input":1,"output":2,"cacheRead":0,"cacheWrite":0,"totalTokens":3,"cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}},"stopReason":"stop","timestamp":1}"#;
        let msg: Message = serde_json::from_str(wire).unwrap();
        assert!(matches!(msg, Message::Assistant(_)));
        let back = serde_json::to_string(&msg).unwrap();
        let again: Message = serde_json::from_str(&back).unwrap();
        assert_eq!(msg, again);
    }

    #[test]
    fn unknown_role_maps_to_catch_all() {
        let msg: Message =
            serde_json::from_str(r#"{"role":"futureRole","content":"x","timestamp":1}"#).unwrap();
        assert!(matches!(msg, Message::Unknown(_)));
    }
}
