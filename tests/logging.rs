use std::fmt::Debug;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pi_rpc::client::Client;
use pi_rpc::error::ErrorKind;
use tracing::field::{Field, Visit};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;

mod common;

use common::{fast_config, pair};

/// Captured event or span, one line per record: `name field=value ...`.
#[derive(Default, Clone)]
struct Records(Arc<Mutex<Vec<String>>>);

impl Records {
    fn lines(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }

    fn has(&self, needle: &str) -> bool {
        self.lines().iter().any(|line| line.contains(needle))
    }
}

struct Writer {
    buf: Arc<Mutex<Vec<String>>>,
    head: String,
    fields: Vec<String>,
}

impl Writer {
    fn push(&mut self, text: String) {
        self.fields.push(text);
    }

    fn done(self) {
        let line = if self.fields.is_empty() {
            self.head
        } else {
            format!("{} {}", self.head, self.fields.join(" "))
        };
        self.buf.lock().unwrap().push(line);
    }
}

impl Visit for Writer {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.push(format!("{}={value:?}", field.name()));
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.push(format!("{}={value}", field.name()));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.push(format!("{}={value}", field.name()));
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.push(format!("{}={value}", field.name()));
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        self.push(format!("{}={value:?}", field.name()));
    }
}

impl<S> Layer<S> for Records
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(
        &self,
        attrs: &tracing::span::Attributes<'_>,
        _id: &tracing::span::Id,
        _ctx: Context<'_, S>,
    ) {
        let mut out = Writer {
            buf: self.0.clone(),
            head: format!("span:{}", attrs.metadata().name()),
            fields: Vec::new(),
        };
        attrs.record(&mut out);
        out.done();
    }

    fn on_record(
        &self,
        _span: &tracing::span::Id,
        values: &tracing::span::Record<'_>,
        _ctx: Context<'_, S>,
    ) {
        let mut out = Writer {
            buf: self.0.clone(),
            head: "span:record".to_string(),
            fields: Vec::new(),
        };
        values.record(&mut out);
        out.done();
    }

    fn on_event(&self, event: &tracing::Event<'_>, ctx: Context<'_, S>) {
        let parent = ctx
            .lookup_current()
            .map(|span| span.metadata().name().to_string())
            .unwrap_or_else(|| "none".to_string());
        let mut out = Writer {
            buf: self.0.clone(),
            head: format!("event:{} parent:{parent}", event.metadata().name()),
            fields: Vec::new(),
        };
        event.record(&mut out);
        out.done();
    }
}

/// Install a test-local subscriber. The guard must outlive the client
/// tasks; the current-thread runtime keeps every poll on this thread.
fn capture(filter: &str) -> (Records, tracing::subscriber::DefaultGuard) {
    let records = Records::default();
    let sub = tracing_subscriber::registry()
        .with(records.clone())
        .with(tracing_subscriber::EnvFilter::new(filter));
    let guard = tracing::subscriber::set_default(sub);
    (records, guard)
}

fn prompt_response(id: &str) -> String {
    format!(
        r#"{{"id":"{id}","type":"response","command":"prompt","success":true,"data":{{"disposition":"started"}}}}"#
    )
}

#[tokio::test(flavor = "current_thread")]
async fn request_span_carries_tag_and_id() {
    let (records, _guard) = capture("pi_rpc=trace");
    let (transport, mut peer) = pair();
    let client = Client::with_config(transport, fast_config());
    peer.out.send(prompt_response("req_1")).unwrap();
    // Sentinel content must never reach the logs, only tag and id may.
    client.prompt("SENTINEL_qzx7_no_leak").await.unwrap();
    let _ = client.close().await;
    assert!(records.has("span:pi_rpc.request rpc.tag=\"prompt\""));
    assert!(records.has("span:record rpc.id=\"req_1\""));
    assert!(records.has("event:pi_rpc.request.sent parent:pi_rpc.request"));
    assert!(records.has("event:pi_rpc.request.done parent:pi_rpc.request"));
    assert!(records.has("rpc.kind=\"prompt\""));
    assert!(
        !records.lines().iter().any(|line| line.contains("SENTINEL")),
        "prompt text leaked into logs"
    );
    let _ = peer.inc.recv().await;
}

#[tokio::test(flavor = "current_thread")]
async fn orphan_response_emits_event() {
    let (records, _guard) = capture("pi_rpc=trace");
    let (transport, mut peer) = pair();
    let client = Client::with_config(transport, fast_config());
    peer.out.send(prompt_response("req_999")).unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while client.drop_counts().stray_response == 0 {
        assert!(
            tokio::time::Instant::now() < deadline,
            "orphan never dispatched"
        );
        tokio::task::yield_now().await;
    }
    assert!(records.has("event:pi_rpc.dispatch.orphan parent:pi_rpc.reader"));
    assert!(records.has("rpc.id=\"req_999\""));
    let _ = client.close().await;
    let _ = peer.inc.recv().await;
}

#[tokio::test(flavor = "current_thread")]
async fn over_cap_line_warns() {
    let (records, _guard) = capture("pi_rpc=trace");
    let (transport, peer) = pair();
    let config = fast_config().with_max_line(16);
    let client = Client::with_config(transport, config);
    peer.out.send("{\"type\":\"response\",\"id\":\"req_1\",\"command\":\"abort\",\"success\":true,\"data\":{}}".to_string()).unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while client.drop_counts().malformed == 0 {
        assert!(
            tokio::time::Instant::now() < deadline,
            "over-cap line never read"
        );
        tokio::task::yield_now().await;
    }
    assert!(records.has("event:pi_rpc.reader.over_cap"));
    assert!(records.has("rpc.cap=16"));
    let _ = client.close().await;
}

#[tokio::test(flavor = "current_thread")]
async fn request_timeout_warns() {
    let (records, _guard) = capture("pi_rpc=trace");
    let (transport, _peer) = pair();
    let config = fast_config().with_request_timeout(Duration::from_millis(50));
    let client = Client::with_config(transport, config);
    let err = client.prompt("hello").await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Timeout);
    assert!(records.has("event:pi_rpc.request.timeout"));
    assert!(records.has("rpc.phase=\"wait\""));
    let _ = client.close().await;
}

#[tokio::test(flavor = "current_thread")]
async fn warn_filter_hides_debug_keeps_warn() {
    let (records, _guard) = capture("pi_rpc=warn");
    let (transport, peer) = pair();
    let config = fast_config().with_max_line(16);
    let client = Client::with_config(transport, config);
    // A short valid line fires the TRACE line event; the long line fires
    // the WARN over-cap event. Under a warn filter only the latter shows.
    peer.out
        .send(r#"{"type":"turn_start"}"#.to_string())
        .unwrap();
    peer.out.send("{\"type\":\"response\",\"id\":\"req_1\",\"command\":\"abort\",\"success\":true,\"data\":{}}".to_string()).unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while client.drop_counts().malformed == 0 {
        assert!(
            tokio::time::Instant::now() < deadline,
            "over-cap line never read"
        );
        tokio::task::yield_now().await;
    }
    assert!(records.has("event:pi_rpc.reader.over_cap"));
    assert!(
        !records
            .lines()
            .iter()
            .any(|line| line.contains("pi_rpc.reader.line")),
        "trace event passed a warn filter"
    );
    let _ = client.close().await;
}

#[tokio::test(flavor = "current_thread")]
async fn decoder_over_cap_emits_event() {
    use pi_rpc::frame::Decoder;

    let (records, _guard) = capture("pi_rpc=trace");
    let mut decoder = Decoder::with_max_line(16);
    let err = decoder.push(&[b'x'; 64]).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Protocol);
    assert!(records.has("event:pi_rpc.frame.over_cap"));
    assert!(records.has("rpc.cap=16"));
}

#[tokio::test(flavor = "current_thread")]
async fn closed_wait_reports_failed_not_timeout() {
    let (records, _guard) = capture("pi_rpc=trace");
    let (transport, _peer) = pair();
    let client = Client::with_config(transport, fast_config());
    let waiter = tokio::spawn({
        let client = client.clone();
        async move { client.wait_for_idle(None).await }
    });
    tokio::task::yield_now().await;
    client.close().await.unwrap();
    let err = waiter.await.unwrap().unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Closed);
    assert!(records.has("event:pi_rpc.wait.failed"));
    assert!(records.has("error.type=Closed"));
    assert!(
        !records
            .lines()
            .iter()
            .any(|line| line.contains("pi_rpc.wait.timeout")),
        "closed wait misreported as timeout"
    );
}
