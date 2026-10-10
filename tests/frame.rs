use pi_rpc::error::ErrorKind;
use pi_rpc::frame::{Decoder, encode_line};

#[test]
fn split_chunk_reassembles_one_line() {
    let mut dec = Decoder::new();
    assert!(dec.push(b"{\"type\":\"ab").unwrap().is_empty());
    let lines = dec.push(b"ort\"}\n").unwrap();
    assert_eq!(lines, vec!["{\"type\":\"abort\"}"]);
    dec.end().unwrap();
}

#[test]
fn merged_chunk_yields_all_lines() {
    let mut dec = Decoder::new();
    let lines = dec.push(b"{\"a\":1}\n{\"b\":2}\n").unwrap();
    assert_eq!(lines, vec!["{\"a\":1}", "{\"b\":2}"]);
    dec.end().unwrap();
}

#[test]
fn crlf_pairs_accept() {
    let mut dec = Decoder::new();
    let lines = dec.push(b"{\"a\":1}\r\n").unwrap();
    assert_eq!(lines, vec!["{\"a\":1}"]);
    dec.end().unwrap();
}

#[test]
fn unicode_separators_never_split() {
    let wire = format!(
        "{{\"type\":\"prompt\",\"message\":\"a{}b{}c\"}}\n",
        ' ', ' '
    );
    assert!(wire.contains(' ') && wire.contains(' '));
    assert!(wire.matches('\n').count() == 1);
    let mut dec = Decoder::new();
    let mid = wire.len() / 2;
    let first = dec.push(&wire.as_bytes()[..mid]).unwrap();
    assert!(first.is_empty());
    let rest = dec.push(&wire.as_bytes()[mid..]).unwrap();
    assert_eq!(rest.len(), 1);
    assert!(rest[0].contains(' ') && rest[0].contains(' '));
    dec.end().unwrap();
}

#[test]
fn line_over_cap_fails() {
    let mut dec = Decoder::with_max_line(8);
    let err = dec.push(b"{\"toolong\":1}\n").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Protocol);
}

#[test]
fn buffered_bytes_past_cap_fail_without_lf() {
    let mut dec = Decoder::with_max_line(8);
    let err = dec.push(b"123456789").unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Protocol);
}

#[test]
fn trailing_partial_fails_end() {
    let mut dec = Decoder::new();
    assert!(dec.push(b"{\"a\":1}\n{\"half").unwrap().len() == 1);
    let err = dec.end().unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Protocol);
}

#[test]
fn empty_stream_ends_clean() {
    Decoder::new().end().unwrap();
}

#[test]
fn encode_then_decode_roundtrips() {
    let wire = encode_line(&serde_json::json!({"id": "req_1", "type": "abort"})).unwrap();
    let mut dec = Decoder::new();
    let lines = dec.push(wire.as_bytes()).unwrap();
    assert_eq!(lines.len(), 1);
    let back: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
    assert_eq!(back["id"], "req_1");
    dec.end().unwrap();
}
