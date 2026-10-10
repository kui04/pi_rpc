use std::sync::Arc;
use std::time::Duration;

use pi_rpc::client::Client;
use pi_rpc::error::{Error, ErrorKind, Result};
use pi_rpc::event::Event;
use pi_rpc::extui::{
    ConfirmDialog, EditorDialog, InputDialog, SelectDialog, UiAnswer, UiOutcome, encode_ui_response,
};
use pi_rpc::transport::Transport;

mod common;

use common::{fast_config, pair};

fn state_response(id: &str) -> String {
    format!(
        r#"{{"id":"{id}","type":"response","command":"get_state","success":true,"data":{{"thinkingLevel":"medium","isStreaming":false,"isCompacting":false,"steeringMode":"all","followUpMode":"one-at-a-time","sessionId":"s1","autoCompactionEnabled":true,"messageCount":0,"pendingMessageCount":0}}}}"#
    )
}

#[tokio::test]
async fn dropped_clone_keeps_session_alive() {
    let (transport, mut peer) = pair();
    let client = Client::with_config(transport, fast_config());
    let spare = client.clone();
    drop(spare);
    peer.out.send(state_response("req_1")).unwrap();
    let state = client.get_state().await.unwrap();
    assert_eq!(state.session_id, "s1");
    let sent: serde_json::Value = serde_json::from_str(&peer.inc.recv().await.unwrap()).unwrap();
    assert_eq!(sent["id"], "req_1");
    drop(peer.out);
    client.close().await.unwrap();
}

#[tokio::test]
async fn state_roundtrip_and_sent_shape() {
    let (transport, mut peer) = pair();
    let client = Client::with_config(transport, fast_config());
    peer.out.send(state_response("req_1")).unwrap();
    let state = client.get_state().await.unwrap();
    assert_eq!(state.session_id, "s1");
    let sent: serde_json::Value = serde_json::from_str(&peer.inc.recv().await.unwrap()).unwrap();
    assert_eq!(sent["type"], "get_state");
    assert_eq!(sent["id"], "req_1");
    drop(peer.out);
    client.close().await.unwrap();
}

#[tokio::test]
async fn out_of_order_responses_match_by_id() {
    let (transport, mut peer) = pair();
    let client = std::sync::Arc::new(Client::with_config(transport, fast_config()));
    let accept = |id: &str| {
        format!(
            r#"{{"id":"{id}","type":"response","command":"prompt","success":true,"data":{{"disposition":"started"}}}}"#
        )
    };
    let c1 = client.clone();
    let c2 = client.clone();
    let t = tokio::spawn(async move { tokio::join!(c1.prompt("one"), c2.prompt("two")) });
    let first = peer.inc.recv().await.unwrap();
    let second = peer.inc.recv().await.unwrap();
    let id_of = |line: &str| {
        serde_json::from_str::<serde_json::Value>(line).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string()
    };
    peer.out.send(accept(&id_of(&second))).unwrap();
    peer.out.send(accept(&id_of(&first))).unwrap();
    let (r1, r2) = t.await.unwrap();
    assert!(r1.is_ok() && r2.is_ok());
    drop(peer.out);
    client.close().await.unwrap();
}

#[tokio::test]
async fn failed_command_maps_to_command_kind() {
    let (transport, peer) = pair();
    let client = Client::with_config(transport, fast_config());
    peer.out
        .send(r#"{"id":"req_1","type":"response","command":"abort","success":false,"error":"no run"}"#.to_string())
        .unwrap();
    let err = client.abort().await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Command);
    drop(peer.out);
    client.close().await.unwrap();
}

#[tokio::test]
async fn no_response_times_out() {
    let (transport, _peer) = pair();
    let client = Client::with_config(transport, fast_config());
    let err = client.get_state().await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Timeout);
    drop(_peer.out);
    client.close().await.unwrap();
}

#[tokio::test]
async fn caller_id_is_stamped_over() {
    use pi_rpc::command::Command;
    let (transport, mut peer) = pair();
    let client = std::sync::Arc::new(Client::with_config(transport, fast_config()));
    let task = tokio::spawn({
        let c = client.clone();
        async move {
            let cmd: Command = serde_json::from_str(r#"{"id":"req_999","type":"abort"}"#).unwrap();
            c.send_raw(cmd).await
        }
    });
    let line = peer.inc.recv().await.expect("command arrives");
    assert_eq!(line.as_bytes().last(), Some(&b'}'));
    let id = serde_json::from_str::<serde_json::Value>(&line).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(id, "req_1");
    peer.out
        .send(format!(
            r#"{{"id":"{id}","type":"response","command":"abort","success":true}}"#
        ))
        .unwrap();
    let body = task.await.unwrap().unwrap();
    assert!(matches!(body, pi_rpc::response::Response::Abort));
    drop(peer.out);
    client.close().await.unwrap();
}

#[tokio::test]
async fn peer_end_closes_pending() {
    let (transport, peer) = pair();
    let client = Client::with_config(transport, fast_config());
    // Drop only the line sender: sends still succeed, reads end cleanly.
    drop(peer.out);
    let start = std::time::Instant::now();
    loop {
        if start.elapsed() > Duration::from_secs(2) {
            panic!("loop never reported close");
        }
        match client.get_state().await {
            Err(err) if err.kind() == ErrorKind::Closed => break,
            Err(err) => panic!("wrong kind: {err:?}"),
            Ok(_) => {}
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[tokio::test]
async fn close_drains_tail_records_until_transport_end() {
    let (transport, peer) = pair();
    let client = Client::with_config(transport, fast_config());
    let mut events = client.subscribe();
    let closing = tokio::spawn({
        let client = client.clone();
        async move { client.close().await }
    });
    // The write half closes first. The peer writes its tail records,
    // then exits; the client must keep reading until that exit, not
    // cut the pipe the moment stdin ends.
    peer.out
        .send(r#"{"type":"agent_settled","aborted":true}"#.to_string())
        .unwrap();
    drop(peer.out);
    let closed = tokio::time::timeout(Duration::from_secs(2), closing)
        .await
        .expect("close must finish")
        .expect("close must succeed");
    assert!(closed.is_ok());
    let late = tokio::time::timeout(Duration::from_secs(2), events.recv())
        .await
        .expect("tail record must arrive")
        .expect("stream must stay open until the drain ends");
    assert!(matches!(&*late, Event::AgentSettled(_)));
}

#[tokio::test]
async fn events_fan_out_to_both_sinks() {
    let (transport, peer) = pair();
    let client = Client::with_config(transport, fast_config());
    let seen = std::sync::Arc::new(std::sync::Mutex::new(0usize));
    let seen2 = seen.clone();
    let _sub = client.on_event(move |_| {
        *seen2.lock().unwrap() += 1;
    });
    let mut rx = client.subscribe();
    peer.out
        .send(r#"{"type":"agent_start"}"#.to_string())
        .unwrap();
    peer.out
        .send(r#"{"type":"agent_settled","aborted":false}"#.to_string())
        .unwrap();
    let events = client.collect_events(None).await.unwrap();
    assert_eq!(events.len(), 2);
    assert!(matches!(&*events[0], Event::AgentStart));
    assert!(matches!(&*events[1], Event::AgentSettled(_)));
    assert!(matches!(&*rx.recv().await.unwrap(), Event::AgentStart));
    assert_eq!(*seen.lock().unwrap(), 2);
    drop(peer.out);
    client.close().await.unwrap();
}

#[tokio::test]
async fn handled_prompt_skips_wait() {
    let (transport, peer) = pair();
    let client = Client::with_config(transport, fast_config());
    peer.out
        .send(r#"{"id":"req_1","type":"response","command":"prompt","success":true,"data":{"disposition":"handled"}}"#.to_string())
        .unwrap();
    let events = client
        .prompt_and_wait("hi", Some(Duration::from_millis(200)))
        .await
        .unwrap();
    assert!(events.is_empty());
    drop(peer.out);
    client.close().await.unwrap();
}

#[tokio::test]
async fn prompt_text_folds_deltas() {
    let (transport, peer) = pair();
    let client = Client::with_config(transport, fast_config());
    peer.out
        .send(r#"{"id":"req_1","type":"response","command":"prompt","success":true,"data":{"disposition":"started"}}"#.to_string())
        .unwrap();
    let usage = r#"{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":0,"cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}}"#;
    for line in [
        format!(
            r#"{{"type":"message_update","usage":{usage},"assistantMessageEvent":{{"type":"text_start","contentIndex":0}}}}"#
        ),
        format!(
            r#"{{"type":"message_update","usage":{usage},"assistantMessageEvent":{{"type":"text_delta","contentIndex":0,"delta":"Hel"}}}}"#
        ),
        format!(
            r#"{{"type":"message_update","usage":{usage},"assistantMessageEvent":{{"type":"text_delta","contentIndex":0,"delta":"lo"}}}}"#
        ),
        r#"{"type":"agent_settled","aborted":false}"#.to_string(),
    ] {
        peer.out.send(line).unwrap();
    }
    let text = client
        .prompt_text("hi", Some(Duration::from_millis(300)))
        .await
        .unwrap();
    assert_eq!(text, "Hello");
    drop(peer.out);
    client.close().await.unwrap();
}

struct AllowAll(Arc<std::sync::Mutex<Option<String>>>);

#[async_trait::async_trait]
impl pi_rpc::extui::UiHandler for AllowAll {
    async fn select(&mut self, id: String, _d: SelectDialog) -> UiOutcome {
        *self.0.lock().unwrap() = Some(id);
        UiOutcome::Value("Allow".to_string())
    }
    async fn confirm(&mut self, _id: String, _d: ConfirmDialog) -> bool {
        true
    }
    async fn input(&mut self, _id: String, _d: InputDialog) -> UiOutcome {
        UiOutcome::Dismissed
    }
    async fn editor(&mut self, _id: String, _d: EditorDialog) -> UiOutcome {
        UiOutcome::Dismissed
    }
}

#[tokio::test]
async fn dialog_roundtrip_answers_select() {
    let (transport, mut peer) = pair();
    let seen = Arc::new(std::sync::Mutex::new(None));
    let client = std::sync::Arc::new(Client::with_config(transport, fast_config()));
    client.set_ui_handler(AllowAll(seen.clone())).await;
    let task = tokio::spawn({
        let c = client.clone();
        async move { c.get_state().await }
    });
    let cmd = peer.inc.recv().await.expect("command arrives");
    assert_eq!(cmd.as_bytes().last(), Some(&b'}'));
    let id = serde_json::from_str::<serde_json::Value>(&cmd).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    peer.out
        .send(r#"{"type":"extension_ui_request","id":"u1","method":"select","title":"Go?","options":["Allow","Block"]}"#.to_string())
        .unwrap();
    let answer = peer.inc.recv().await.expect("answer arrives");
    assert_eq!(answer.as_bytes().last(), Some(&b'}'));
    let v: serde_json::Value = serde_json::from_str(&answer).unwrap();
    assert_eq!(v["type"], "extension_ui_response");
    assert_eq!(v["id"], "u1");
    assert_eq!(v["value"], "Allow");
    assert_eq!(seen.lock().unwrap().as_deref(), Some("u1"));
    peer.out.send(state_response(&id)).unwrap();
    task.await.unwrap().unwrap();
    drop(peer.out);
    client.close().await.unwrap();
}

#[tokio::test]
async fn missing_handler_auto_dismisses() {
    let (transport, mut peer) = pair();
    let client = std::sync::Arc::new(Client::with_config(transport, fast_config()));
    let task = tokio::spawn({
        let c = client.clone();
        async move { c.get_state().await }
    });
    let cmd = peer.inc.recv().await.expect("command arrives");
    assert_eq!(cmd.as_bytes().last(), Some(&b'}'));
    let id = serde_json::from_str::<serde_json::Value>(&cmd).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    peer.out
        .send(r#"{"type":"extension_ui_request","id":"u9","method":"confirm","title":"Sure?","message":"m"}"#.to_string())
        .unwrap();
    let answer = peer.inc.recv().await.expect("answer arrives");
    assert_eq!(answer.as_bytes().last(), Some(&b'}'));
    let v: serde_json::Value = serde_json::from_str(&answer).unwrap();
    assert_eq!(v["type"], "extension_ui_response");
    assert_eq!(v["cancelled"], true);
    peer.out.send(state_response(&id)).unwrap();
    task.await.unwrap().unwrap();
    drop(peer.out);
    client.close().await.unwrap();
}

#[test]
fn answer_shapes_hit_wire() {
    for (answer, key) in [
        (UiAnswer::Value("Allow".to_string()), "value"),
        (UiAnswer::Confirmed(false), "confirmed"),
        (UiAnswer::Cancelled, "cancelled"),
    ] {
        let line = encode_ui_response("u1", &answer).unwrap();
        let v: serde_json::Value = serde_json::from_str(line.trim_end()).unwrap();
        assert_eq!(v["type"], "extension_ui_response");
        assert_eq!(v["id"], "u1");
        assert!(v.get(key).is_some(), "answer carries {key}");
    }
}

struct WedgeTx;
struct WedgeRx;

impl pi_rpc::transport::TransportSend for WedgeTx {
    async fn send(&mut self, _line: String) -> Result<()> {
        std::future::pending().await
    }

    async fn close(&mut self) -> Result<()> {
        std::future::pending().await
    }
}

impl pi_rpc::transport::TransportRecv for WedgeRx {
    async fn recv(&mut self) -> Result<Option<String>> {
        std::future::pending().await
    }
}

struct Wedge;

impl Transport for Wedge {
    type Tx = WedgeTx;
    type Rx = WedgeRx;

    fn split(self) -> (WedgeTx, WedgeRx) {
        (WedgeTx, WedgeRx)
    }
}

#[tokio::test]
async fn wedged_close_times_out() {
    let client = Client::with_config(Wedge, fast_config());
    let err = client.close().await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Timeout);
}

struct FaultTx;
struct FaultRx;

impl pi_rpc::transport::TransportSend for FaultTx {
    async fn send(&mut self, _line: String) -> Result<()> {
        Ok(())
    }

    async fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

impl pi_rpc::transport::TransportRecv for FaultRx {
    async fn recv(&mut self) -> Result<Option<String>> {
        Err(Error::new(ErrorKind::Transport, "pipe reset"))
    }
}

struct Fault;

impl Transport for Fault {
    type Tx = FaultTx;
    type Rx = FaultRx;

    fn split(self) -> (FaultTx, FaultRx) {
        (FaultTx, FaultRx)
    }
}

#[tokio::test]
async fn close_surfaces_read_fault() {
    let client = Client::with_config(Fault, fast_config());
    let err = client.close().await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Transport);
}

#[tokio::test]
async fn close_drain_honors_close_timeout() {
    let (transport, _peer) = pair();
    let config = fast_config().with_close_timeout(Duration::from_millis(30));
    let client = Client::with_config(transport, config);
    // The peer never ends its read side, so the drain hits its own
    // budget instead of the request timeout.
    let err = client.close().await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Timeout);
}

#[tokio::test]
async fn panicking_listener_counts_without_killing_loop() {
    let (transport, peer) = pair();
    let client = Client::with_config(transport, fast_config());
    let _sub = client.on_event(|_| panic!("boom"));
    let mut rx = client.subscribe();
    peer.out
        .send(r#"{"type":"agent_start"}"#.to_string())
        .unwrap();
    peer.out
        .send(r#"{"type":"agent_settled","aborted":false}"#.to_string())
        .unwrap();
    let marker = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("marker arrives")
        .expect("stream open");
    assert!(matches!(&*marker, Event::AgentStart));
    let settled = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("settle arrives")
        .expect("stream open");
    assert!(matches!(&*settled, Event::AgentSettled(_)));
    assert_eq!(client.drop_counts().callback_panic, 2);
    let state = client.get_state().await;
    assert!(state.is_err());
    drop(peer.out);
    client.close().await.unwrap();
}

#[tokio::test]
async fn lagged_reader_reports_protocol() {
    let (transport, peer) = pair();
    let client = Client::with_config(transport, fast_config().with_event_capacity(2));
    let mut rx = client.subscribe();
    for _ in 0..64 {
        peer.out
            .send(r#"{"type":"agent_start"}"#.to_string())
            .unwrap();
    }
    let marker = tokio::time::timeout(Duration::from_secs(2), rx.recv()).await;
    match marker {
        Ok(Err(err)) => assert_eq!(err.kind(), ErrorKind::Protocol),
        Ok(Ok(_)) => panic!("expected lag"),
        Err(_) => panic!("expected lag, got timeout"),
    }
    drop(peer.out);
    client.close().await.unwrap();
}

#[tokio::test]
async fn long_line_skips_without_killing_stream() {
    let (transport, mut peer) = pair();
    let client = Client::with_config(transport, fast_config().with_max_line(400));
    let pad = "0".repeat(500);
    peer.out
        .send(format!(r#"{{"type":"agent_start","pad":"{pad}"}}"#))
        .unwrap();
    let client = std::sync::Arc::new(client);
    let task = tokio::spawn({
        let c = client.clone();
        async move { c.get_state().await }
    });
    let cmd = peer.inc.recv().await.expect("command arrives");
    assert_eq!(cmd.as_bytes().last(), Some(&b'}'));
    let id = serde_json::from_str::<serde_json::Value>(&cmd).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    peer.out.send(state_response(&id)).unwrap();
    let state = task.await.unwrap().unwrap();
    assert_eq!(state.session_id, "s1");
    assert_eq!(client.drop_counts().malformed, 1);
    drop(peer.out);
    client.close().await.unwrap();
}

#[tokio::test]
async fn drops_are_counted() {
    let (transport, peer) = pair();
    let client = Client::with_config(transport, fast_config());
    let mut rx = client.subscribe();
    peer.out.send("not json".to_string()).unwrap();
    peer.out
        .send(r#"{"id":"nobody","type":"response","command":"abort","success":true}"#.to_string())
        .unwrap();
    peer.out
        .send(r#"{"type":"response","command":"abort","success":true}"#.to_string())
        .unwrap();
    peer.out
        .send(r#"{"type":"message_update"}"#.to_string())
        .unwrap();
    peer.out
        .send(r#"{"type":"extension_ui_request","id":"x"}"#.to_string())
        .unwrap();
    peer.out
        .send(r#"{"type":"agent_start"}"#.to_string())
        .unwrap();
    let marker = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("marker arrives")
        .expect("stream open");
    assert!(matches!(&*marker, pi_rpc::event::Event::AgentStart));
    let counts = client.drop_counts();
    assert_eq!(counts.malformed, 1);
    assert_eq!(counts.stray_response, 2);
    assert_eq!(counts.bad_event, 1);
    assert_eq!(counts.bad_ui, 1);
    drop(peer.out);
    client.close().await.unwrap();
}
