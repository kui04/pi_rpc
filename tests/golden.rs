use pi_rpc::command::Command;
use pi_rpc::entry::{Entry, ReplacementContent};
use pi_rpc::event::{CompactionReason, Event, RetrySource};
use pi_rpc::event::{decode_event, encode_event};
use pi_rpc::extui::encode_ui_response;
use pi_rpc::extui::{NoticeKind, UiAnswer, UiRequest, WidgetPlace, decode_ui_request};
use pi_rpc::frame::encode_line;
use pi_rpc::message::{Message, NestedStatus, UserContent};
use pi_rpc::response::{PromptData, Response, decode_envelope};
use pi_rpc::types::{
    CommandSource, ModelInput, PromptDisposition, QueueMode, QueuedInputDisposition, SourceOrigin,
    SourceScope, StopReason, StreamingBehavior, ThinkingLevel,
};

fn parse_command(wire: &str) -> Command {
    serde_json::from_str(wire).expect("doc vector must parse")
}

#[test]
fn doc_prompt_roundtrip() {
    let wire = r#"{"id":"req-2","type":"prompt","message":"Review this repository"}"#;
    let cmd = parse_command(wire);
    assert!(matches!(cmd, Command::Prompt(_)));
    let back = encode_line(&cmd).unwrap();
    let want: serde_json::Value = serde_json::from_str(wire).unwrap();
    let got: serde_json::Value = serde_json::from_str(back.trim_end()).unwrap();
    assert_eq!(got, want);
}

#[test]
fn doc_steer_during_stream() {
    let cmd = parse_command(
        r#"{"type":"prompt","message":"New instruction","streamingBehavior":"steer"}"#,
    );
    match cmd {
        Command::Prompt(p) => {
            assert_eq!(p.streaming_behavior, Some(StreamingBehavior::Steer))
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

#[test]
fn doc_prompt_accept_response() {
    let wire = r#"{"id":"req-2","type":"response","command":"prompt","success":true,"data":{"disposition":"started"}}"#;
    let env = decode_envelope(wire).expect("doc vector must parse");
    assert_eq!(env.id.as_deref(), Some("req-2"));
    assert!(matches!(
        env.body,
        Response::Prompt(PromptData {
            disposition: PromptDisposition::Started
        })
    ));
}

#[test]
fn doc_failed_command() {
    let wire = r#"{"id":"req-3","type":"response","command":"set_model","success":false,"error":"Model not found: invalid/model"}"#;
    let env = decode_envelope(wire).expect("doc vector must parse");
    match env.body {
        Response::Error(f) => {
            assert_eq!(f.command, "set_model");
            assert!(f.error.contains("invalid/model"));
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

#[test]
fn doc_parse_error_without_id() {
    let wire = r#"{"type":"response","command":"parse","success":false,"error":"Failed to parse command: boom"}"#;
    let env = decode_envelope(wire).expect("doc vector must parse");
    assert!(env.id.is_none());
    assert!(matches!(env.body, Response::Error(_)));
}

#[test]
fn doc_bash_flow() {
    let cmd = parse_command(r#"{"id":"req-1","type":"bash","command":"ls -la"}"#);
    assert!(matches!(cmd, Command::Bash(_)));
    let chunk =
        decode_event(r#"{"type":"bash_execution_update","id":"req-1","delta":"total 48\n"}"#)
            .expect("doc vector must parse");
    match chunk {
        Event::BashExecutionUpdate(c) => {
            assert_eq!(c.id.as_deref(), Some("req-1"));
            assert_eq!(c.delta, "total 48\n");
        }
        other => panic!("wrong variant: {other:?}"),
    }
    let wire = r#"{"id":"req-1","type":"response","command":"bash","success":true,"data":{"output":"total 48\n","exitCode":0,"cancelled":false,"truncated":false}}"#;
    let env = decode_envelope(wire).expect("doc vector must parse");
    match env.body {
        Response::Bash(b) => {
            assert_eq!(b.exit_code, Some(0.0));
            assert!(!b.truncated);
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

#[test]
fn doc_compact_response() {
    let wire = r#"{"type":"response","command":"compact","success":true,"data":{"summary":"s","firstKeptEntryId":"abc123","tokensBefore":150000,"estimatedTokensAfter":32000}}"#;
    let env = decode_envelope(wire).expect("doc vector must parse");
    match env.body {
        Response::Compact(c) => {
            assert_eq!(c.first_kept_entry_id, "abc123");
            assert_eq!(c.estimated_tokens_after, Some(32000.0));
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

#[test]
fn doc_entries_cursor() {
    let wire = r#"{"type":"response","command":"get_entries","success":true,"data":{"entries":[{"type":"message","id":"def456","parentId":"abc123","timestamp":"2024-12-03T14:00:02.000Z","message":{"role":"user","content":"Hello","timestamp":1733234401000}}],"leafId":"def456"}}"#;
    let env = decode_envelope(wire).expect("doc vector must parse");
    match env.body {
        Response::GetEntries(list) => {
            assert_eq!(list.leaf_id.as_deref(), Some("def456"));
            assert!(matches!(list.entries[0], Entry::Message(_)));
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

#[test]
fn doc_extension_select_dialog() {
    let wire = r#"{"type":"extension_ui_request","id":"uuid-1","method":"select","title":"Allow dangerous command?","options":["Allow","Block"],"timeout":10000}"#;
    let (id, req) = decode_ui_request(wire).expect("doc vector must parse");
    assert_eq!(id, "uuid-1");
    match req {
        UiRequest::Select(d) => {
            assert_eq!(d.options.len(), 2);
            assert_eq!(d.timeout, Some(10000.0));
        }
        other => panic!("wrong variant: {other:?}"),
    }
    for (wire, want) in [
        (
            encode_ui_response("uuid-1", &UiAnswer::Value("Allow".to_string())).unwrap(),
            r#"{"type":"extension_ui_response","id":"uuid-1","value":"Allow"}"#,
        ),
        (
            encode_ui_response("uuid-2", &UiAnswer::Confirmed(true)).unwrap(),
            r#"{"type":"extension_ui_response","id":"uuid-2","confirmed":true}"#,
        ),
        (
            encode_ui_response("uuid-3", &UiAnswer::Cancelled).unwrap(),
            r#"{"type":"extension_ui_response","id":"uuid-3","cancelled":true}"#,
        ),
    ] {
        let got: serde_json::Value = serde_json::from_str(&wire).unwrap();
        let want: serde_json::Value = serde_json::from_str(want).unwrap();
        assert_eq!(got, want);
        assert_eq!(wire.as_bytes().last(), Some(&b'}'));
    }
}

#[test]
fn doc_fork_response_shapes() {
    let wire = r#"{"type":"response","command":"fork","success":true,"data":{"text":"The original prompt text...","cancelled":false}}"#;
    let env = decode_envelope(wire).expect("doc vector must parse");
    match env.body {
        Response::Fork(data) => {
            assert!(!data.cancelled);
            assert_eq!(data.text.as_deref(), Some("The original prompt text..."));
        }
        other => panic!("wrong variant: {other:?}"),
    }
    let wire = r#"{"type":"response","command":"fork","success":true,"data":{"text":"","cancelled":true}}"#;
    let env = decode_envelope(wire).expect("doc vector must parse");
    match env.body {
        Response::Fork(data) => {
            assert!(data.cancelled);
            assert_eq!(data.text.as_deref(), Some(""));
        }
        other => panic!("wrong variant: {other:?}"),
    }
    let wire = r#"{"type":"response","command":"fork","success":true,"data":{"cancelled":true}}"#;
    let env = decode_envelope(wire).expect("doc vector must parse");
    match env.body {
        Response::Fork(data) => {
            assert!(data.cancelled);
            assert!(data.text.is_none());
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

#[test]
fn doc_extension_error_event() {
    let event = decode_event(
        r#"{"type":"extension_error","extensionPath":"/x.ts","event":"tool_call","error":"boom"}"#,
    )
    .expect("doc vector must parse");
    assert!(matches!(event, Event::ExtensionError(_)));
}

#[test]
fn doc_model_with_cost_parses() {
    let wire = r#"{"type":"response","command":"set_model","success":true,"data":{"id":"claude-sonnet-4-20250514","name":"Claude Sonnet 4","api":"anthropic-messages","provider":"anthropic","baseUrl":"https://api.anthropic.com","reasoning":true,"input":["text","image"],"contextWindow":200000,"maxTokens":16384,"cost":{"input":3.0,"output":15.0,"cacheRead":0.3,"cacheWrite":3.75}}}"#;
    let env = decode_envelope(wire).expect("doc vector must parse");
    match env.body {
        Response::SetModel(model) => {
            let cost = model.cost.expect("cost present");
            assert_eq!(cost.rates.input, 3.0);
            assert_eq!(cost.rates.cache_write, 3.75);
            assert!(cost.tiers.is_none());
            assert_eq!(model.context_window, Some(200000.0));
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

#[test]
fn doc_context_edit_with_thinking_replacement() {
    let wire = r#"{"type":"response","command":"get_entries","success":true,"data":{"entries":[{"type":"context_edit","id":"g6","parentId":"f6","timestamp":"2024-12-03T14:11:00.000Z","targetId":"c3","replacement":{"content":[{"type":"thinking","thinking":"hmm"}]}}],"leafId":"g6"}}"#;
    let env = decode_envelope(wire).expect("doc vector must parse");
    match env.body {
        Response::GetEntries(list) => match &list.entries[0] {
            Entry::ContextEdit(edit) => match &edit.replacement {
                Some(rep) => assert!(matches!(
                    rep.content,
                    ReplacementContent::AssistantBlocks(_)
                )),
                None => panic!("replacement present"),
            },
            other => panic!("wrong variant: {other:?}"),
        },
        other => panic!("wrong variant: {other:?}"),
    }
}

#[test]
fn unknown_kinds_replay_verbatim() {
    let event_wire = r#"{"type":"future_event","x":1}"#;
    let event = decode_event(event_wire).unwrap();
    match &event {
        Event::Unknown(raw) => {
            assert_eq!(raw.kind, "future_event");
            assert!(raw.rest.get("type").is_none());
        }
        other => panic!("wrong variant: {other:?}"),
    }
    let back: serde_json::Value =
        serde_json::from_str(encode_event(&event).unwrap().trim_end()).unwrap();
    let want: serde_json::Value = serde_json::from_str(event_wire).unwrap();
    assert_eq!(back, want);

    let msg: Message = serde_json::from_str(r#"{"role":"futureRole","content":"x"}"#).unwrap();
    match &msg {
        Message::Unknown(raw) => assert_eq!(raw.kind, "futureRole"),
        other => panic!("wrong variant: {other:?}"),
    }

    let entry: Entry = serde_json::from_str(r#"{"type":"future_entry","id":"a"}"#).unwrap();
    match &entry {
        Entry::Unknown(raw) => assert_eq!(raw.kind, "future_entry"),
        other => panic!("wrong variant: {other:?}"),
    }

    let (_, req) =
        decode_ui_request(r#"{"type":"extension_ui_request","id":"z","method":"future_dialog"}"#)
            .unwrap();
    assert!(matches!(req, UiRequest::Unknown(_)));
}

#[test]
fn doc_assistant_replay_keeps_optionals() {
    let wire = r#"{"role":"assistant","content":[{"type":"text","text":"Hi"}],"api":"a","provider":"p","model":"m","responseModel":"m2","responseId":"r1","providerThinkingLevel":"high","thinkingLevel":"medium","diagnostics":[{"x":1}],"usage":{"input":1,"output":1,"cacheRead":0,"cacheWrite":0,"totalTokens":2,"cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}},"stopReason":"stop","rawStopReason":"end_turn","endTurn":true,"timestamp":1,"durationMs":12}"#;
    let msg: Message = serde_json::from_str(wire).unwrap();
    // Numbers may print as floats (12 vs 12.0): documented drift, values equal.
    let back = serde_json::to_value(&msg).unwrap();
    for key in [
        "responseModel",
        "responseId",
        "providerThinkingLevel",
        "thinkingLevel",
        "diagnostics",
        "rawStopReason",
        "endTurn",
        "durationMs",
    ] {
        assert!(back.get(key).is_some(), "{key} survives replay");
    }
    match msg {
        Message::Assistant(a) => {
            assert_eq!(a.response_id.as_deref(), Some("r1"));
            assert_eq!(a.end_turn, Some(true));
            assert_eq!(a.duration_ms, Some(12.0));
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

#[test]
fn doc_tool_result_keeps_duration() {
    let wire = r#"{"role":"toolResult","toolCallId":"c1","toolName":"bash","content":[{"type":"text","text":"out"}],"isError":false,"timestamp":1,"durationMs":12}"#;
    let msg: Message = serde_json::from_str(wire).unwrap();
    match msg {
        Message::ToolResult(result) => assert_eq!(result.duration_ms, Some(12.0)),
        other => panic!("wrong variant: {other:?}"),
    }
}

#[test]
fn unknown_string_values_roundtrip() {
    fn check<T>(wire: &str)
    where
        T: serde::de::DeserializeOwned + serde::Serialize + PartialEq + std::fmt::Debug,
    {
        let value: T = serde_json::from_str(wire).unwrap();
        let back = serde_json::to_string(&value).unwrap();
        assert_eq!(back, wire);
        let again: T = serde_json::from_str(&back).unwrap();
        assert_eq!(value, again);
    }
    check::<ThinkingLevel>("\"ultra\"");
    check::<StreamingBehavior>("\"defer\"");
    check::<QueueMode>("\"latest\"");
    check::<PromptDisposition>("\"dropped\"");
    check::<QueuedInputDisposition>("\"dropped\"");
    check::<ModelInput>("\"video\"");
    check::<CommandSource>("\"script\"");
    check::<SourceScope>("\"shared\"");
    check::<SourceOrigin>("\"remote\"");
    check::<StopReason>("\"paused\"");
    check::<NestedStatus>("\"stalled\"");
    check::<CompactionReason>("\"panic\"");
    check::<RetrySource>("\"manual\"");
    check::<NoticeKind>("\"fatal\"");
    check::<WidgetPlace>("\"sidebar\"");
}

#[test]
fn unknown_command_roundtrips() {
    let wire =
        r#"{"id":"r1","type":"response","command":"future_cmd","success":true,"data":{"x":1}}"#;
    let env = decode_envelope(wire).expect("doc vector must parse");
    match &env.body {
        Response::Unknown(unknown) => {
            assert_eq!(unknown.command, "future_cmd");
            assert!(unknown.success);
        }
        other => panic!("wrong variant: {other:?}"),
    }
    let back = pi_rpc::response::encode_envelope(&env).unwrap();
    let got: serde_json::Value = serde_json::from_str(back.trim_end()).unwrap();
    let want: serde_json::Value = serde_json::from_str(wire).unwrap();
    assert_eq!(got, want);
}

#[test]
fn unknown_command_without_data_stays_absent() {
    let wire = r#"{"id":"r2","type":"response","command":"future_cmd","success":true}"#;
    let env = decode_envelope(wire).expect("doc vector must parse");
    match &env.body {
        Response::Unknown(unknown) => assert!(unknown.data.is_none()),
        other => panic!("wrong variant: {other:?}"),
    }
    let back = pi_rpc::response::encode_envelope(&env).unwrap();
    let got: serde_json::Value = serde_json::from_str(back.trim_end()).unwrap();
    assert!(got.get("data").is_none(), "missing stays missing");
}

#[test]
fn nested_unknown_message_replays_verbatim() {
    for record in [
        r#"{"type":"message_end","message":{"role":"futureRole","content":"x","timestamp":1}}"#,
        r#"{"type":"message_start","message":{"role":"futureRole","content":"x","timestamp":1}}"#,
    ] {
        let event = decode_event(record).unwrap();
        let back = encode_event(&event).unwrap();
        let got: serde_json::Value = serde_json::from_str(back.trim_end()).unwrap();
        let want: serde_json::Value = serde_json::from_str(record).unwrap();
        assert_eq!(got, want);
    }
}

#[test]
fn doc_prompt_with_images_roundtrips() {
    let wire = r#"{"id":"p1","type":"prompt","message":"What's in this image?","images":[{"type":"image","data":"AAAA","mimeType":"image/png"}]}"#;
    let cmd: Command = serde_json::from_str(wire).unwrap();
    match &cmd {
        Command::Prompt(prompt) => {
            let images = prompt.images.as_ref().expect("images present");
            assert_eq!(images[0].mime_type, "image/png");
        }
        other => panic!("wrong variant: {other:?}"),
    }
    let back = encode_line(&cmd).unwrap();
    let got: serde_json::Value = serde_json::from_str(back.trim_end()).unwrap();
    let want: serde_json::Value = serde_json::from_str(wire).unwrap();
    assert_eq!(got, want);
}

#[test]
fn doc_user_message_with_image_block() {
    let wire = r#"{"role":"user","content":[{"type":"text","text":"Look"},{"type":"image","data":"AAAA","mimeType":"image/jpeg"}],"timestamp":1}"#;
    let msg: Message = serde_json::from_str(wire).unwrap();
    match &msg {
        Message::User(user) => match &user.content {
            UserContent::Blocks(blocks) => assert_eq!(blocks.len(), 2),
            other => panic!("wrong content: {other:?}"),
        },
        other => panic!("wrong variant: {other:?}"),
    }
}

#[test]
fn doc_get_state_response() {
    let wire = r#"{"type":"response","command":"get_state","success":true,"data":{"model":{"id":"m","name":"M","api":"anthropic-messages","provider":"anthropic","reasoning":true,"input":["text"]},"thinkingLevel":"medium","isStreaming":false,"isCompacting":false,"steeringMode":"all","followUpMode":"one-at-a-time","sessionId":"abc123","autoCompactionEnabled":true,"messageCount":5,"pendingMessageCount":0}}"#;
    let env = decode_envelope(wire).expect("doc vector must parse");
    match env.body {
        Response::GetState(s) => {
            assert_eq!(s.session_id, "abc123");
            assert_eq!(s.message_count, 5);
        }
        other => panic!("wrong variant: {other:?}"),
    }
}
