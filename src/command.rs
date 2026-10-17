//! Stdin commands. One JSON object per line.
//!
//! Source: pi `v1.3.0`
//! `packages/coding-agent/src/modes/rpc/rpc-types.ts:20-94` (`RpcCommand`).
//! Shapes: `packages/coding-agent/docs/rpc-commands.md`.

use serde::{Deserialize, Serialize};

use crate::types::{ImageContent, QueueMode, StreamingBehavior, ThinkingLevel};

/// Command sent on stdin. One JSON object per line.
///
/// The client always stamps a fresh `req_N` id on send, like upstream, and
/// ignores any id already present. Responses repeat it for correlation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
    Prompt(Prompt),
    Steer(Steer),
    FollowUp(FollowUp),
    Abort(Common),
    ClearQueue(Common),
    NewSession(NewSession),
    GetState(Common),
    SetModel(SetModel),
    CycleModel(Common),
    GetAvailableModels(Common),
    SetThinkingLevel(SetThinkingLevel),
    CycleThinkingLevel(Common),
    GetAvailableThinkingLevels(Common),
    SetSteeringMode(SetQueueMode),
    SetFollowUpMode(SetQueueMode),
    Compact(Compact),
    SetAutoCompaction(SetFlag),
    SetAutoRetry(SetFlag),
    AbortRetry(Common),
    Bash(Bash),
    AbortBash(Common),
    GetSessionStats(Common),
    ExportHtml(ExportHtml),
    SwitchSession(SwitchSession),
    Fork(Fork),
    Clone(Common),
    GetForkMessages(Common),
    GetEntries(GetEntries),
    GetTree(Common),
    GetLastAssistantText(Common),
    SetSessionName(SetSessionName),
    GetMessages(Common),
    GetCommands(Common),
}

/// Optional correlation id shared by every command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Common {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

impl Command {
    /// Wire tag of this command. Single home for all 33 tags, so no
    /// hand-written copy can drift from the enum.
    pub fn tag(&self) -> &'static str {
        match self {
            Command::Prompt(_) => "prompt",
            Command::Steer(_) => "steer",
            Command::FollowUp(_) => "follow_up",
            Command::Abort(_) => "abort",
            Command::ClearQueue(_) => "clear_queue",
            Command::NewSession(_) => "new_session",
            Command::GetState(_) => "get_state",
            Command::SetModel(_) => "set_model",
            Command::CycleModel(_) => "cycle_model",
            Command::GetAvailableModels(_) => "get_available_models",
            Command::SetThinkingLevel(_) => "set_thinking_level",
            Command::CycleThinkingLevel(_) => "cycle_thinking_level",
            Command::GetAvailableThinkingLevels(_) => "get_available_thinking_levels",
            Command::SetSteeringMode(_) => "set_steering_mode",
            Command::SetFollowUpMode(_) => "set_follow_up_mode",
            Command::Compact(_) => "compact",
            Command::SetAutoCompaction(_) => "set_auto_compaction",
            Command::SetAutoRetry(_) => "set_auto_retry",
            Command::AbortRetry(_) => "abort_retry",
            Command::Bash(_) => "bash",
            Command::AbortBash(_) => "abort_bash",
            Command::GetSessionStats(_) => "get_session_stats",
            Command::ExportHtml(_) => "export_html",
            Command::SwitchSession(_) => "switch_session",
            Command::Fork(_) => "fork",
            Command::Clone(_) => "clone",
            Command::GetForkMessages(_) => "get_fork_messages",
            Command::GetEntries(_) => "get_entries",
            Command::GetTree(_) => "get_tree",
            Command::GetLastAssistantText(_) => "get_last_assistant_text",
            Command::SetSessionName(_) => "set_session_name",
            Command::GetMessages(_) => "get_messages",
            Command::GetCommands(_) => "get_commands",
        }
    }
}

/// User prompt for the agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Prompt {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<ImageContent>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(rename = "streamingBehavior")]
    pub streaming_behavior: Option<StreamingBehavior>,
}

/// Steering message delivered after the current assistant turn.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Steer {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<ImageContent>>,
}

/// Follow-up message delivered after the run settles.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FollowUp {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<ImageContent>>,
}

/// Start a fresh session, optionally tracking its parent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewSession {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(rename = "parentSession")]
    pub parent_session: Option<String>,
}

/// Switch to a model by provider and id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetModel {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub provider: String,
    #[serde(rename = "modelId")]
    pub model_id: String,
}

/// Set the reasoning level for models that support it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetThinkingLevel {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub level: ThinkingLevel,
}

/// Control steering or follow-up flush behavior.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetQueueMode {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub mode: QueueMode,
}

/// Compact context with optional focus instructions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Compact {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(rename = "customInstructions")]
    pub custom_instructions: Option<String>,
}

/// Toggle for auto compaction and auto retry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetFlag {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub enabled: bool,
}

/// Run a shell command inside the session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bash {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(rename = "excludeFromContext")]
    pub exclude_from_context: Option<bool>,
}

/// Export the session transcript to HTML.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportHtml {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(rename = "outputPath")]
    pub output_path: Option<String>,
}

/// Load a different session file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SwitchSession {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "sessionPath")]
    pub session_path: String,
}

/// Fork from a previous user message entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fork {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "entryId")]
    pub entry_id: String,
}

/// Entries after a durable cursor. Omit `since` for the full list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GetEntries {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
}

/// Set the display name of the current session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetSessionName {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_wire_shape_matches_upstream() {
        let cmd = Command::Prompt(Prompt {
            id: Some("req-1".to_string()),
            message: "Hello".to_string(),
            images: None,
            streaming_behavior: None,
        });
        let wire = serde_json::to_string(&cmd).unwrap();
        assert_eq!(wire, r#"{"type":"prompt","id":"req-1","message":"Hello"}"#);
    }

    #[test]
    fn all_command_tags_parse() {
        let cases = [
            (r#"{"type":"prompt","message":"hi"}"#, "Prompt"),
            (r#"{"type":"steer","message":"hi"}"#, "Steer"),
            (r#"{"type":"follow_up","message":"hi"}"#, "FollowUp"),
            (r#"{"type":"abort"}"#, "Abort"),
            (r#"{"type":"clear_queue"}"#, "ClearQueue"),
            (r#"{"type":"new_session"}"#, "NewSession"),
            (r#"{"type":"get_state"}"#, "GetState"),
            (
                r#"{"type":"set_model","provider":"a","modelId":"m"}"#,
                "SetModel",
            ),
            (r#"{"type":"cycle_model"}"#, "CycleModel"),
            (r#"{"type":"get_available_models"}"#, "GetAvailableModels"),
            (
                r#"{"type":"set_thinking_level","level":"high"}"#,
                "SetThinkingLevel",
            ),
            (r#"{"type":"cycle_thinking_level"}"#, "CycleThinkingLevel"),
            (
                r#"{"type":"get_available_thinking_levels"}"#,
                "GetAvailableThinkingLevels",
            ),
            (
                r#"{"type":"set_steering_mode","mode":"all"}"#,
                "SetSteeringMode",
            ),
            (
                r#"{"type":"set_follow_up_mode","mode":"one-at-a-time"}"#,
                "SetFollowUpMode",
            ),
            (r#"{"type":"compact"}"#, "Compact"),
            (
                r#"{"type":"set_auto_compaction","enabled":true}"#,
                "SetAutoCompaction",
            ),
            (
                r#"{"type":"set_auto_retry","enabled":false}"#,
                "SetAutoRetry",
            ),
            (r#"{"type":"abort_retry"}"#, "AbortRetry"),
            (r#"{"type":"bash","command":"ls"}"#, "Bash"),
            (r#"{"type":"abort_bash"}"#, "AbortBash"),
            (r#"{"type":"get_session_stats"}"#, "GetSessionStats"),
            (r#"{"type":"export_html"}"#, "ExportHtml"),
            (
                r#"{"type":"switch_session","sessionPath":"/s.jsonl"}"#,
                "SwitchSession",
            ),
            (r#"{"type":"fork","entryId":"abc"}"#, "Fork"),
            (r#"{"type":"clone"}"#, "Clone"),
            (r#"{"type":"get_fork_messages"}"#, "GetForkMessages"),
            (r#"{"type":"get_entries"}"#, "GetEntries"),
            (r#"{"type":"get_tree"}"#, "GetTree"),
            (
                r#"{"type":"get_last_assistant_text"}"#,
                "GetLastAssistantText",
            ),
            (
                r#"{"type":"set_session_name","name":"work"}"#,
                "SetSessionName",
            ),
            (r#"{"type":"get_messages"}"#, "GetMessages"),
            (r#"{"type":"get_commands"}"#, "GetCommands"),
        ];
        assert_eq!(cases.len(), 33);
        for (wire, name) in cases {
            let parsed: Command = serde_json::from_str(wire).unwrap();
            assert_eq!(format!("{parsed:?}").split('(').next(), Some(name));
            let tag = serde_json::from_str::<serde_json::Value>(wire).unwrap()["type"]
                .as_str()
                .unwrap()
                .to_string();
            assert_eq!(parsed.tag(), tag);
        }
    }
}
