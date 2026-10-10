//! Shared types used by commands, responses, messages, and events.
//!
//! Source: pi `f1b2e77`
//! `packages/agent/src/types.ts:349` (`ThinkingLevel`);
//! `packages/ai/src/types.ts:446-467` (`Usage`);
//! `packages/ai/src/types.ts:820-867` (`BaseModel`, `Model`);
//! `packages/ai/src/providers/model-schema.ts:5` (modalities),
//! `:25-46` (cost rates);
//! `packages/coding-agent/src/core/agent-session.ts:310-311` (dispositions);
//! `packages/coding-agent/src/core/source-info.ts:3-12` (`SourceInfo`).
//! Model object shape: `packages/coding-agent/docs/rpc-commands.md`
//! (Model object).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Wide string enum decode: known wire strings map to variants, anything
/// else keeps its text in `Other`. Writes always emit the exact string.
#[doc(hidden)]
#[macro_export]
macro_rules! wide_string_enum {
    ($name:ident { $($wire:literal => $variant:ident),* $(,)? }) => {
        impl serde::Serialize for $name {
            fn serialize<S: serde::Serializer>(
                &self,
                serializer: S,
            ) -> std::result::Result<S::Ok, S::Error> {
                match self {
                    $(Self::$variant => serializer.serialize_str($wire),)*
                    Self::Other(text) => text.serialize(serializer),
                }
            }
        }

        impl<'de> serde::Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(
                deserializer: D,
            ) -> std::result::Result<Self, D::Error> {
                let text = String::deserialize(deserializer)?;
                Ok(match text.as_str() {
                    $($wire => Self::$variant,)*
                    _ => Self::Other(text),
                })
            }
        }
    };
}

/// Reasoning effort for models that support it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThinkingLevel {
    Off,
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
    Max,
    /// Future level. Reads widen; writes send the string back verbatim.
    Other(String),
}

crate::wide_string_enum!(ThinkingLevel {
    "off" => Off,
    "minimal" => Minimal,
    "low" => Low,
    "medium" => Medium,
    "high" => High,
    "xhigh" => Xhigh,
    "max" => Max,
});

/// How a prompt sent mid-run joins the queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamingBehavior {
    Steer,
    FollowUp,
    /// Future behavior. Reads widen; writes send the string back verbatim.
    Other(String),
}

crate::wide_string_enum!(StreamingBehavior {
    "steer" => Steer,
    "followUp" => FollowUp,
});

/// How queued steering or follow-up messages flush.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueMode {
    All,
    OneAtATime,
    /// Future mode. Reads widen; writes send the string back verbatim.
    Other(String),
}

crate::wide_string_enum!(QueueMode {
    "all" => All,
    "one-at-a-time" => OneAtATime,
});

/// What happened to a submitted prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptDisposition {
    Started,
    Queued,
    Handled,
    /// Future disposition. Reads widen; writes send the string back verbatim.
    Other(String),
}

crate::wide_string_enum!(PromptDisposition {
    "started" => Started,
    "queued" => Queued,
    "handled" => Handled,
});

/// What happened to a queued steer or follow-up message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueuedInputDisposition {
    Queued,
    Handled,
    /// Future disposition. Reads widen; writes send the string back verbatim.
    Other(String),
}

crate::wide_string_enum!(QueuedInputDisposition {
    "queued" => Queued,
    "handled" => Handled,
});

/// Base64 image payload shared by prompts and messages.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageContent {
    #[serde(rename = "type")]
    pub kind: ImageKind,
    pub data: String,
    #[serde(rename = "mimeType")]
    pub mime_type: String,
}

/// Marker that this content block is an image.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageKind {
    Image,
}

/// Token use and cost for one assistant message or nested work.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_1h: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<f64>,
    pub total_tokens: f64,
    pub cost: CostRates,
}

/// Per-million-token prices in USD.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CostRates {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
    pub total: f64,
}

/// Model price rates. Unlike `CostRates`, upstream models carry no total
/// (`model-schema.ts:25-30`, doc Model object).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRates {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
}

/// Model pricing with optional request-wide tiers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelCost {
    #[serde(flatten)]
    pub rates: ModelRates,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tiers: Option<Vec<CostTier>>,
}

/// One pricing tier above an input-token threshold.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CostTier {
    pub input_tokens_above: f64,
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
}

/// Input kinds a model accepts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelInput {
    Text,
    Image,
    /// Future modality. Reads widen; writes send the string back verbatim.
    Other(String),
}

crate::wide_string_enum!(ModelInput {
    "text" => Text,
    "image" => Image,
});

/// Configured model definition returned by model commands.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Model {
    pub id: String,
    pub name: String,
    pub api: String,
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    pub reasoning: bool,
    #[serde(default)]
    pub input: Vec<ModelInput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<ModelCost>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

/// Live context-window estimate for compaction display.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextUsage {
    pub tokens: Option<f64>,
    pub context_window: f64,
    pub percent: Option<f64>,
}

/// Where a discoverable command came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandSource {
    Extension,
    Prompt,
    Skill,
    /// Future source. Reads widen; writes send the string back verbatim.
    Other(String),
}

crate::wide_string_enum!(CommandSource {
    "extension" => Extension,
    "prompt" => Prompt,
    "skill" => Skill,
});

/// Scope that registered a command resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceScope {
    User,
    Project,
    Temporary,
    /// Future scope. Reads widen; writes send the string back verbatim.
    Other(String),
}

crate::wide_string_enum!(SourceScope {
    "user" => User,
    "project" => Project,
    "temporary" => Temporary,
});

/// Load origin of a command resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceOrigin {
    TopLevel,
    Package,
    /// Future origin. Reads widen; writes send the string back verbatim.
    Other(String),
}

crate::wide_string_enum!(SourceOrigin {
    "top-level" => TopLevel,
    "package" => Package,
});

/// Metadata for the resource behind a discoverable command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceInfo {
    pub path: String,
    pub source: String,
    pub scope: SourceScope,
    pub origin: SourceOrigin,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_dir: Option<String>,
}

/// Command available through `prompt` with a `/` prefix.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlashCommand {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub source: CommandSource,
    pub source_info: SourceInfo,
}

/// Terminal state of one assistant message.
#[derive(Debug, Clone, PartialEq)]
pub enum StopReason {
    Pending,
    Stop,
    Length,
    ToolUse,
    Error,
    Aborted,
    Deferred,
    /// Future reason. Reads widen; writes send the string back verbatim.
    Other(String),
}

crate::wide_string_enum!(StopReason {
    "pending" => Pending,
    "stop" => Stop,
    "length" => Length,
    "toolUse" => ToolUse,
    "error" => Error,
    "aborted" => Aborted,
    "deferred" => Deferred,
});

/// Raw record for a kind this lib does not know yet.
///
/// `kind` repeats the tag for direct reads. `rest` holds every other
/// field, never the tag itself, so derived parents can insert exactly one
/// tag and never print duplicates. Replay rebuilds the exact record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawRecord {
    pub kind: String,
    pub rest: serde_json::Value,
}

impl RawRecord {
    /// Split the tag at `key` off `value`. Missing tag becomes empty.
    pub fn new(key: &str, value: serde_json::Value) -> Self {
        let mut rest = value;
        let kind = rest
            .get(key)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if let Some(map) = rest.as_object_mut() {
            map.remove(key);
        }
        Self { kind, rest }
    }

    /// Rebuild the exact record: tag first, then stored fields.
    pub fn replay(&self, key: &str) -> serde_json::Value {
        let mut out =
            serde_json::Map::with_capacity(self.rest.as_object().map(|m| m.len() + 1).unwrap_or(1));
        out.insert(
            key.to_string(),
            serde_json::Value::String(self.kind.clone()),
        );
        if let Some(map) = self.rest.as_object() {
            out.extend(map.clone());
        }
        serde_json::Value::Object(out)
    }
}

impl Serialize for RawRecord {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.rest.serialize(serializer)
    }
}
