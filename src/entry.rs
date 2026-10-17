//! Append-only session entries forming the branch tree.
//!
//! Source: pi `v1.3.0`
//! `packages/coding-agent/src/core/session-manager.ts:43-50` (header),
//! `:57-62` (base), `:64-180` (11 entry kinds), `:183-194` (union),
//! `:200-207` (tree node).
//! Shapes: `packages/coding-agent/docs/session-format.md:49-207`.

use serde::{Deserialize, Serialize};

use crate::error::{Error, ErrorKind};
use crate::message::{AssistantBlock, Message, SystemMessage, UserBlock, UserContent};
use crate::types::{RawRecord, Usage};

/// First line of a session file. Metadata only, outside the tree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionHeader {
    #[serde(rename = "type")]
    pub kind: HeaderKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<f64>,
    pub id: String,
    pub timestamp: String,
    pub cwd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(rename = "parentSession")]
    pub parent_session: Option<String>,
}

/// Marker that this line is a session header.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HeaderKind {
    Session,
}

/// Link fields shared by every tree entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntryBase {
    pub id: String,
    #[serde(rename = "parentId")]
    pub parent_id: Option<String>,
    pub timestamp: String,
}

/// Append-only session entries forming the branch tree.
///
/// Unknown kinds keep their raw record and replay it verbatim everywhere,
/// including inside lists, trees, and event payloads.
///
/// The derived `Serialize` is lossy for unknowns (it renames the tag).
/// The `encode_entry_value` twin is the faithful path; use it to replay.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[expect(
    clippy::large_enum_variant,
    reason = "decodes once per record; boxing buys indirection"
)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Entry {
    Message(MessageEntry),
    ThinkingLevelChange(ThinkingLevelEntry),
    ModelChange(ModelEntry),
    Usage(UsageEntry),
    Compaction(CompactionEntry),
    BranchSummary(BranchSummaryEntry),
    Custom(CustomEntry),
    CustomMessage(CustomMessageEntry),
    ContextEdit(ContextEditEntry),
    Label(LabelEntry),
    SessionInfo(SessionInfoEntry),
    Unknown(RawRecord),
}

impl<'de> Deserialize<'de> for Entry {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        decode_entry_value(value).map_err(serde::de::Error::custom)
    }
}

/// Decode one already-parsed entry value. Unknown kinds keep raw payload.
pub fn decode_entry_value(value: serde_json::Value) -> Result<Entry, Error> {
    let kind = value
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if kind.is_empty() {
        return Err(Error::new(ErrorKind::Protocol, "entry lacks type"));
    }
    let bad = |err: serde_json::Error| {
        Error::new(ErrorKind::Protocol, format!("bad {kind} entry: {err}"))
    };
    let entry = match kind.as_str() {
        "message" => Entry::Message(serde_json::from_value(value).map_err(bad)?),
        "thinking_level_change" => {
            Entry::ThinkingLevelChange(serde_json::from_value(value).map_err(bad)?)
        }
        "model_change" => Entry::ModelChange(serde_json::from_value(value).map_err(bad)?),
        "usage" => Entry::Usage(serde_json::from_value(value).map_err(bad)?),
        "compaction" => Entry::Compaction(serde_json::from_value(value).map_err(bad)?),
        "branch_summary" => Entry::BranchSummary(serde_json::from_value(value).map_err(bad)?),
        "custom" => Entry::Custom(serde_json::from_value(value).map_err(bad)?),
        "custom_message" => Entry::CustomMessage(serde_json::from_value(value).map_err(bad)?),
        "context_edit" => Entry::ContextEdit(serde_json::from_value(value).map_err(bad)?),
        "label" => Entry::Label(serde_json::from_value(value).map_err(bad)?),
        "session_info" => Entry::SessionInfo(serde_json::from_value(value).map_err(bad)?),
        _ => Entry::Unknown(RawRecord::new("type", value)),
    };
    Ok(entry)
}

/// Conversation message with tree links.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageEntry {
    #[serde(flatten)]
    pub base: EntryBase,
    pub message: Message,
}

/// Thinking level switch mid-session. Level stays a string for forward compat.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThinkingLevelEntry {
    #[serde(flatten)]
    pub base: EntryBase,
    pub thinking_level: String,
}

/// Model switch mid-session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelEntry {
    #[serde(flatten)]
    pub base: EntryBase,
    pub provider: String,
    pub model_id: String,
}

/// Model-attributed use outside assistant messages.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageEntry {
    #[serde(flatten)]
    pub base: EntryBase,
    pub kind: String,
    pub provider: String,
    pub model: String,
    pub usage: Usage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Context checkpoint with prompt state replay.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactionEntry {
    #[serde(flatten)]
    pub base: EntryBase,
    pub summary: String,
    pub first_kept_entry_id: String,
    pub tokens_before: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_hook: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_message: Option<SystemMessage>,
}

/// Summary of an abandoned branch path.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchSummaryEntry {
    #[serde(flatten)]
    pub base: EntryBase,
    pub from_id: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_hook: Option<bool>,
}

/// Extension state outside LLM context.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomEntry {
    #[serde(flatten)]
    pub base: EntryBase,
    pub custom_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

/// Extension message inside LLM context.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomMessageEntry {
    #[serde(flatten)]
    pub base: EntryBase,
    pub custom_type: String,
    pub content: UserContent,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
    pub display: bool,
}

/// Append-only content edit of one earlier entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextEditEntry {
    #[serde(flatten)]
    pub base: EntryBase,
    pub target_id: String,
    pub replacement: Option<ContentReplacement>,
}

/// New content for the target. Null omits it from model context.
///
/// Covers all four upstream shapes (`session-manager.ts:167-172`): user,
/// assistant (with thinking and tool calls), tool-result, and custom
/// content. Block arrays decode to assistant form when they fit; wire
/// bytes roundtrip either way.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ReplacementContent {
    Text(String),
    AssistantBlocks(Vec<AssistantBlock>),
    UserBlocks(Vec<UserBlock>),
    Other(serde_json::Value),
}

/// New content wrapper for the target.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContentReplacement {
    pub content: ReplacementContent,
}

/// Bookmark on one entry. Null label clears it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LabelEntry {
    #[serde(flatten)]
    pub base: EntryBase,
    pub target_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// Session metadata such as the display name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionInfoEntry {
    #[serde(flatten)]
    pub base: EntryBase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// One tree node with resolved label for `get_tree`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeNode {
    pub entry: Entry,
    pub children: Vec<TreeNode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_timestamp: Option<String>,
}

/// Encode one entry back to its exact wire shape.
///
/// Unknown kinds replay their stored record verbatim. Message entries
/// encode their message through its twin, so nested unknown roles replay
/// verbatim too.
pub fn encode_entry_value(entry: &Entry) -> Result<serde_json::Value, Error> {
    match entry {
        Entry::Unknown(raw) => Ok(raw.replay("type")),
        Entry::Message(message) => {
            let mut out = serde_json::Map::new();
            out.insert("type".to_string(), "message".into());
            out.insert("id".to_string(), message.base.id.clone().into());
            out.insert(
                "parentId".to_string(),
                message
                    .base
                    .parent_id
                    .clone()
                    .map_or(serde_json::Value::Null, serde_json::Value::String),
            );
            out.insert(
                "timestamp".to_string(),
                message.base.timestamp.clone().into(),
            );
            out.insert(
                "message".to_string(),
                crate::message::encode_message_value(&message.message)?,
            );
            Ok(serde_json::Value::Object(out))
        }
        _ => serde_json::to_value(entry)
            .map_err(|err| Error::new(ErrorKind::Protocol, format!("bad entry: {err}"))),
    }
}

/// Encode one tree node with verbatim entries at every depth.
pub fn encode_tree_node_value(node: &TreeNode) -> Result<serde_json::Value, Error> {
    let mut children = Vec::with_capacity(node.children.len());
    for child in &node.children {
        children.push(encode_tree_node_value(child)?);
    }
    let mut out = serde_json::Map::new();
    out.insert("entry".to_string(), encode_entry_value(&node.entry)?);
    out.insert("children".to_string(), serde_json::Value::Array(children));
    if let Some(label) = &node.label {
        out.insert(
            "label".to_string(),
            serde_json::Value::String(label.clone()),
        );
    }
    if let Some(timestamp) = &node.label_timestamp {
        out.insert(
            "labelTimestamp".to_string(),
            serde_json::Value::String(timestamp.clone()),
        );
    }
    Ok(serde_json::Value::Object(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_kinds_parse() {
        let base = r#""id":"a1","parentId":null,"timestamp":"2024-12-03T14:00:01.000Z""#;
        let cases = [
            format!(
                r#"{{"type":"message",{base},"message":{{"role":"user","content":"Hello","timestamp":1}}}}"#
            ),
            format!(r#"{{"type":"model_change",{base},"provider":"a","modelId":"m"}}"#),
            format!(r#"{{"type":"thinking_level_change",{base},"thinkingLevel":"high"}}"#),
            format!(r#"{{"type":"label",{base},"targetId":"a1","label":"mark"}}"#),
            format!(r#"{{"type":"session_info",{base},"name":"work"}}"#),
        ];
        for wire in cases {
            let entry: Entry = serde_json::from_str(&wire).unwrap();
            assert!(!matches!(entry, Entry::Unknown(_)));
        }
    }
}
