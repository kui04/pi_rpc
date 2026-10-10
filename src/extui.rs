//! Extension UI dialog subprotocol.
//!
//! Source: pi `f1b2e77`
//! `packages/coding-agent/src/modes/rpc/rpc-types.ts:252-297`
//! (request and response unions).
//! Behaviors: `packages/coding-agent/docs/rpc-extension-ui.md:12-26`
//! (limits), `:27-170` (requests), `:172-194` (answers).

use serde::{Deserialize, Serialize};

use crate::error::{Error, ErrorKind};
use crate::types::RawRecord;

/// Dialog or notice pi needs from the user.
///
/// Dialog methods (`select`, `confirm`, `input`, `editor`) block pi until
/// the client answers with the matching id. Fire-and-forget methods expect
/// no answer and the client may ignore them.
///
/// Note: `set_editor_text` is snake_case upstream while the other eight
/// methods are camelCase (`rpc-types.ts:287`). Each variant carries its
/// exact wire name, so a rename lint can never rejoin them.
///
/// Unknown methods keep their raw record. The client dismisses them by id,
/// so new pi dialogs never stall the stream.
///
/// The derived `Serialize` is lossy for unknowns (it renames the method).
/// Requests are inbound-only, so no encode twin exists.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "method")]
pub enum UiRequest {
    #[serde(rename = "select")]
    Select(SelectDialog),
    #[serde(rename = "confirm")]
    Confirm(ConfirmDialog),
    #[serde(rename = "input")]
    Input(InputDialog),
    #[serde(rename = "editor")]
    Editor(EditorDialog),
    #[serde(rename = "notify")]
    Notify(Notice),
    #[serde(rename = "setStatus")]
    SetStatus(StatusEntry),
    #[serde(rename = "setWidget")]
    SetWidget(WidgetEntry),
    #[serde(rename = "setTitle")]
    SetTitle(WindowTitle),
    #[serde(rename = "set_editor_text")]
    SetEditorText(EditorText),
    Unknown(RawRecord),
}

impl UiRequest {
    /// Short tag for log fields. Known variants use snake_case even where
    /// the wire method is camelCase (`setStatus` logs as `set_status`); the
    /// catch-all carries the wire method verbatim. Mirrors the decode table
    /// in [`decode_ui_request_value`]; keep both in one edit.
    pub fn tag(&self) -> &str {
        match self {
            UiRequest::Select(_) => "select",
            UiRequest::Confirm(_) => "confirm",
            UiRequest::Input(_) => "input",
            UiRequest::Editor(_) => "editor",
            UiRequest::Notify(_) => "notify",
            UiRequest::SetStatus(_) => "set_status",
            UiRequest::SetWidget(_) => "set_widget",
            UiRequest::SetTitle(_) => "set_title",
            UiRequest::SetEditorText(_) => "set_editor_text",
            UiRequest::Unknown(record) => record.kind.as_str(),
        }
    }
}

impl<'de> Deserialize<'de> for UiRequest {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        decode_ui_request_value(value).map_err(serde::de::Error::custom)
    }
}

/// Decode one already-parsed UI record. Unknown methods keep raw payload.
pub fn decode_ui_request_value(value: serde_json::Value) -> Result<UiRequest, Error> {
    let method = value
        .get("method")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if method.is_empty() {
        return Err(Error::new(ErrorKind::Protocol, "ui record lacks method"));
    }
    let bad = |err: serde_json::Error| {
        Error::new(
            ErrorKind::Protocol,
            format!("bad {method} ui record: {err}"),
        )
    };
    let request = match method.as_str() {
        "select" => UiRequest::Select(serde_json::from_value(value).map_err(bad)?),
        "confirm" => UiRequest::Confirm(serde_json::from_value(value).map_err(bad)?),
        "input" => UiRequest::Input(serde_json::from_value(value).map_err(bad)?),
        "editor" => UiRequest::Editor(serde_json::from_value(value).map_err(bad)?),
        "notify" => UiRequest::Notify(serde_json::from_value(value).map_err(bad)?),
        "setStatus" => UiRequest::SetStatus(serde_json::from_value(value).map_err(bad)?),
        "setWidget" => UiRequest::SetWidget(serde_json::from_value(value).map_err(bad)?),
        "setTitle" => UiRequest::SetTitle(serde_json::from_value(value).map_err(bad)?),
        "set_editor_text" => UiRequest::SetEditorText(serde_json::from_value(value).map_err(bad)?),
        _ => UiRequest::Unknown(RawRecord::new("method", value)),
    };
    Ok(request)
}

/// Choice prompt with agent-side timeout in ms.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SelectDialog {
    pub title: String,
    pub options: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<f64>,
}

/// Yes-or-no prompt with agent-side timeout in ms.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfirmDialog {
    pub title: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<f64>,
}

/// Free-text prompt with agent-side timeout in ms.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InputDialog {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<f64>,
}

/// Multi-line editor with optional prefill.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EditorDialog {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefill: Option<String>,
}

/// Fire-and-forget notice.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Notice {
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notify_type: Option<NoticeKind>,
}

/// Notice severity, `info` when absent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoticeKind {
    Info,
    Warning,
    Error,
    /// Future severity. Reads widen; writes send the string back verbatim.
    Other(String),
}

crate::wide_string_enum!(NoticeKind {
    "info" => Info,
    "warning" => Warning,
    "error" => Error,
});

/// Footer status entry. Absent text clears the key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusEntry {
    pub status_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_text: Option<String>,
}

/// Editor widget lines. Absent lines clear the key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WidgetEntry {
    pub widget_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub widget_lines: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub widget_placement: Option<WidgetPlace>,
}

/// Widget position above or below the editor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WidgetPlace {
    AboveEditor,
    BelowEditor,
    /// Future placement. Reads widen; writes send the string back verbatim.
    Other(String),
}

crate::wide_string_enum!(WidgetPlace {
    "aboveEditor" => AboveEditor,
    "belowEditor" => BelowEditor,
});

/// Terminal title update.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowTitle {
    pub title: String,
}

/// Editor text replacement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EditorText {
    pub text: String,
}

/// Answer to one dialog request. Id must match the request.
///
/// Wire conversion goes through [`encode_ui_response`]; this enum carries
/// no serde shape of its own so the record tag stays exact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiAnswer {
    /// Text answer for select, input, and editor dialogs.
    Value(String),
    /// Boolean answer for confirm dialogs.
    Confirmed(bool),
    /// Dismissal for any dialog. Extension sees undefined or false.
    Cancelled,
}

/// Parse one stdout UI request line into its id plus typed body.
pub fn decode_ui_request(line: &str) -> Result<(String, UiRequest), Error> {
    let wire: WireUiRequest = serde_json::from_str(line)
        .map_err(|err| Error::new(ErrorKind::Protocol, format!("bad ui request: {err}")))?;
    if wire.kind != "extension_ui_request" {
        return Err(Error::new(
            ErrorKind::Protocol,
            format!("not a ui request record: {}", wire.kind),
        ));
    }
    let id = wire
        .id
        .ok_or_else(|| Error::new(ErrorKind::Protocol, "ui request lacks id"))?;
    let body = decode_ui_request_value(wire.rest)?;
    Ok((id, body))
}

/// Encode one dialog answer with the exact wire tag.
///
/// No LF terminator, per the [`Transport`](crate::transport::Transport)
/// contract. The caller frames the line for its own path.
pub fn encode_ui_response(id: &str, answer: &UiAnswer) -> Result<String, Error> {
    let mut obj = serde_json::Map::with_capacity(4);
    obj.insert(
        "type".to_string(),
        serde_json::Value::String("extension_ui_response".to_string()),
    );
    obj.insert("id".to_string(), serde_json::Value::String(id.to_string()));
    match answer {
        UiAnswer::Value(value) => {
            obj.insert(
                "value".to_string(),
                serde_json::Value::String(value.clone()),
            );
        }
        UiAnswer::Confirmed(confirmed) => {
            obj.insert("confirmed".to_string(), serde_json::Value::Bool(*confirmed));
        }
        UiAnswer::Cancelled => {
            obj.insert("cancelled".to_string(), serde_json::Value::Bool(true));
        }
    }
    serde_json::to_string(&serde_json::Value::Object(obj))
        .map_err(|err| Error::new(ErrorKind::Protocol, format!("bad ui answer: {err}")))
}

#[derive(Debug, Deserialize)]
struct WireUiRequest {
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(flatten)]
    rest: serde_json::Value,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_nine_methods_parse_and_tag() {
        let cases = [
            (
                r#"{"type":"extension_ui_request","id":"1","method":"select","title":"T","options":["A"]}"#,
                "Select",
                "select",
            ),
            (
                r#"{"type":"extension_ui_request","id":"2","method":"confirm","title":"T","message":"M"}"#,
                "Confirm",
                "confirm",
            ),
            (
                r#"{"type":"extension_ui_request","id":"3","method":"input","title":"T"}"#,
                "Input",
                "input",
            ),
            (
                r#"{"type":"extension_ui_request","id":"4","method":"editor","title":"T"}"#,
                "Editor",
                "editor",
            ),
            (
                r#"{"type":"extension_ui_request","id":"5","method":"notify","message":"M"}"#,
                "Notify",
                "notify",
            ),
            (
                r#"{"type":"extension_ui_request","id":"6","method":"setStatus","statusKey":"k"}"#,
                "SetStatus",
                "set_status",
            ),
            (
                r#"{"type":"extension_ui_request","id":"7","method":"setWidget","widgetKey":"k"}"#,
                "SetWidget",
                "set_widget",
            ),
            (
                r#"{"type":"extension_ui_request","id":"8","method":"setTitle","title":"T"}"#,
                "SetTitle",
                "set_title",
            ),
            (
                r#"{"type":"extension_ui_request","id":"9","method":"set_editor_text","text":"t"}"#,
                "SetEditorText",
                "set_editor_text",
            ),
        ];
        for (wire, name, tag) in cases {
            let (id, req) = decode_ui_request(wire).unwrap();
            assert!(!id.is_empty());
            assert_eq!(format!("{req:?}").split('(').next(), Some(name));
            assert_eq!(req.tag(), tag);
        }
    }

    #[test]
    fn unknown_method_maps_to_catch_all() {
        let (_, req) = decode_ui_request(
            r#"{"type":"extension_ui_request","id":"z","method":"future_dialog","title":"T"}"#,
        )
        .unwrap();
        assert!(matches!(req, UiRequest::Unknown(_)));
        assert_eq!(req.tag(), "future_dialog");
    }
}

/// Handler that answers dialog requests.
///
/// Fire-and-forget methods need no answer; default impls ignore them.
/// Timeouts resolve pi-side, so answer fast and skip local timers.
#[async_trait::async_trait]
pub trait UiHandler: Send {
    async fn select(&mut self, dialog: SelectDialog) -> UiOutcome;
    async fn confirm(&mut self, dialog: ConfirmDialog) -> bool;
    async fn input(&mut self, dialog: InputDialog) -> UiOutcome;
    async fn editor(&mut self, dialog: EditorDialog) -> UiOutcome;
    async fn notify(&mut self, _notice: Notice) {}
    async fn set_status(&mut self, _entry: StatusEntry) {}
    async fn set_widget(&mut self, _entry: WidgetEntry) {}
    async fn set_title(&mut self, _title: WindowTitle) {}
    async fn set_editor_text(&mut self, _text: EditorText) {}
}

/// Dialog result: text when answered, none when dismissed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiOutcome {
    Value(String),
    Dismissed,
}
