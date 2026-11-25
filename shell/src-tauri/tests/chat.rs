//! Chat tests (#31). Synthetic data only. The service is a fake or a local test server on
//! 127.0.0.1; no real network, no model, no keychain (the store key is injected).

use companion_shell::chat::{chat_turn, read_stream, service_url_ok, HttpService, SendError, Service};
use companion_shell::extract::{extract_text, xml_text, ExtractError, MAX_TEXT_CHARS};
use companion_shell::store::{Fixed, Store};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

const SECRET: &str = "Zebra-Quokka-7731"; // synthetic stand-in for something the officer typed

fn tmp(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let d = std::env::temp_dir().join(format!("companion-chat-{name}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn open(dir: &Path) -> Store {
    Store::open(&dir.join("store.db"), &Fixed([3u8; 32])).unwrap()
}

fn side(dir: &Path) -> Connection {
    let c = Connection::open(dir.join("store.db")).unwrap();
    c.pragma_update(None, "key", format!("x'{}'", "03".repeat(32))).unwrap();
    c
}

fn chat_rows(dir: &Path) -> Vec<(String, String)> {
    let c = side(dir);
    let mut st = c.prepare("SELECT role, content FROM chat ORDER BY id").unwrap();
    let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().collect::<Result<_, _>>().unwrap();
    rows
}

/// A fake service: records what it was sent, replies in chunks or fails.
struct Fake {
    seen: RefCell<Vec<Value>>,
    result: Result<Vec<&'static str>, fn() -> SendError>,
}
impl Fake {
    fn replying(chunks: &[&'static str]) -> Fake {
        Fake { seen: RefCell::new(vec![]), result: Ok(chunks.to_vec()) }
    }
    fn failing(f: fn() -> SendError) -> Fake {
        Fake { seen: RefCell::new(vec![]), result: Err(f) }
    }
}
impl Service for Fake {
    fn chat(&self, body: &Value, on_text: &mut dyn FnMut(&str)) -> Result<(), SendError> {
        self.seen.borrow_mut().push(body.clone());
        match &self.result {
            Ok(chunks) => {
                chunks.iter().for_each(|c| on_text(c));
                Ok(())
            }
            Err(f) => Err(f()),
        }
    }
}

fn convo(text: &str) -> Value {
    json!([{ "role": "user", "content": text }])
}

#[test]
fn a_turn_streams_chunks_in_order_and_saves_both_sides_locally() {
    let d = tmp("turn");
    let store = open(&d);
    let svc = Fake::replying(&["Dear ", "colleague, ", "thanks."]);
    let mut got = vec![];
    chat_turn(&svc, &store, "official-open", &convo(&format!("Draft a reply about {SECRET}")), 5, &mut |t| got.push(t.to_string())).unwrap();
    assert_eq!(got, ["Dear ", "colleague, ", "thanks."]);
    let sent = &svc.seen.borrow()[0];
    assert_eq!(sent["classification"], "official-open");
    assert_eq!(sent["messages"][0]["content"], format!("Draft a reply about {SECRET}"));
    assert_eq!(
        chat_rows(&d),
        vec![
            ("user".to_string(), format!("Draft a reply about {SECRET}")),
            ("assistant".to_string(), "Dear colleague, thanks.".to_string())
        ]
    );
}

#[test]
fn chat_content_is_in_the_chat_table_and_no_other_table() {
    let d = tmp("only");
    let store = open(&d);
    chat_turn(&Fake::replying(&["ok"]), &store, "official-open", &convo(SECRET), 5, &mut |_| {}).unwrap();
    let c = side(&d);
    let tables: Vec<String> = c
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    for t in tables.iter().filter(|t| *t != "chat") {
        let cols: Vec<String> = c.prepare(&format!("SELECT name FROM pragma_table_info('{t}')")).unwrap().query_map([], |r| r.get(0)).unwrap().collect::<Result<_, _>>().unwrap();
        for col in cols {
            let hits: i64 = c.query_row(&format!("SELECT count(*) FROM \"{t}\" WHERE CAST(\"{col}\" AS TEXT) LIKE '%{SECRET}%'"), [], |r| r.get(0)).unwrap();
            assert_eq!(hits, 0, "{t}.{col} holds chat text");
        }
    }
}

#[test]
fn a_refused_turn_saves_nothing_and_says_so_plainly() {
    let d = tmp("refused");
    let store = open(&d);
    let e = chat_turn(&Fake::failing(|| SendError::AboveCeiling), &store, "secret", &convo(SECRET), 5, &mut |_| {}).unwrap_err();
    assert_eq!(e, SendError::AboveCeiling);
    assert!(e.plain().contains("not sent"));
    assert!(!e.plain().contains(SECRET), "the refusal never echoes what was typed");
    assert!(chat_rows(&d).is_empty());
}

#[test]
fn no_classification_is_not_sent() {
    let d = tmp("nocls");
    let store = open(&d);
    let svc = Fake::replying(&["x"]);
    for c in ["", "  "] {
        assert_eq!(chat_turn(&svc, &store, c, &convo("hi"), 5, &mut |_| {}), Err(SendError::NoClassification));
    }
    assert!(svc.seen.borrow().is_empty());
    assert!(chat_rows(&d).is_empty());
}

#[test]
fn bad_conversations_are_not_sent() {
    let d = tmp("bad");
    let store = open(&d);
    let svc = Fake::replying(&["x"]);
    for m in [
        json!([]),
        json!("hi"),
        json!([{ "role": "system", "content": "x" }]),
        json!([{ "role": "user", "content": 3 }]),
        json!([{ "role": "user", "content": "a" }, { "role": "assistant", "content": "b" }]),
    ] {
        assert_eq!(chat_turn(&svc, &store, "official-open", &m, 5, &mut |_| {}), Err(SendError::BadMessages));
    }
    assert!(svc.seen.borrow().is_empty());
}

#[test]
fn a_reply_that_stops_early_saves_nothing() {
    let d = tmp("cut");
    let store = open(&d);
    let r = chat_turn(&Fake::failing(|| SendError::ReplyFailed), &store, "official-open", &convo("hi"), 5, &mut |_| {});
    assert_eq!(r, Err(SendError::ReplyFailed));
    assert!(chat_rows(&d).is_empty());
}

#[test]
fn stream_parser_needs_done_and_rejects_errors() {
    let mut got = String::new();
    let ok = "{\"text\":\"a\"}\n\n{\"text\":\"b\"}\n{\"done\":true}\n";
    read_stream(Cursor::new(ok), &mut |t| got.push_str(t)).unwrap();
    assert_eq!(got, "ab");
    for bad in ["{\"text\":\"a\"}\n", "{\"text\":\"a\"}\n{\"error\":\"route_error\"}\n", "not json\n", ""] {
        assert_eq!(read_stream(Cursor::new(bad), &mut |_| {}), Err(SendError::ReplyFailed), "{bad:?}");
    }
}

#[test]
fn only_https_or_this_machine_is_a_service_address() {
    for ok in ["https://service.example.test", "http://localhost:8787", "http://127.0.0.1:8787/x", "http://[::1]:1"] {
        assert!(service_url_ok(ok), "{ok}");
    }
    for bad in ["http://service.example.test", "http://localhost.evil.test", "http://127.0.0.1.evil.test", "http://localhost@evil.test", "ftp://x", "", "service"] {
        assert!(!service_url_ok(bad), "{bad}");
    }
    assert_eq!(HttpService::new("http://service.example.test").err(), Some(SendError::NotConfigured));
}

/// A local server standing in for the companion service. Serves one canned reply per request.
fn serve(status: u16, body: &'static str) -> (String, std::thread::JoinHandle<String>) {
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let url = format!("http://127.0.0.1:{}", server.server_addr().to_ip().unwrap().port());
    let h = std::thread::spawn(move || {
        let mut req = server.recv().unwrap();
        let mut got = String::new();
        std::io::Read::read_to_string(req.as_reader(), &mut got).unwrap();
        let line = format!("{} {}", req.method(), req.url());
        req.respond(tiny_http::Response::from_string(body).with_status_code(status)).unwrap();
        format!("{line}\n{got}")
    });
    (url, h)
}

#[test]
fn http_service_posts_to_chat_and_reads_the_stream() {
    let (url, h) = serve(200, "{\"text\":\"Hello \"}\n{\"text\":\"there\"}\n{\"done\":true}\n");
    let svc = HttpService::new(&url).unwrap();
    let mut got = String::new();
    svc.chat(&json!({ "classification": "official-open", "messages": convo("hi") }), &mut |t| got.push_str(t)).unwrap();
    assert_eq!(got, "Hello there");
    let seen = h.join().unwrap();
    assert!(seen.starts_with("POST /chat\n"));
    assert!(seen.contains("\"classification\":\"official-open\""));
}

#[test]
fn http_403_is_the_ceiling_refusal_and_other_statuses_are_plain_failures() {
    for (status, want) in [(403, SendError::AboveCeiling), (413, SendError::TooLong), (400, SendError::BadMessages), (502, SendError::ReplyFailed), (500, SendError::Unreachable)] {
        let (url, h) = serve(status, "{\"error\":\"above_ceiling\",\"reason\":\"x\"}");
        let e = HttpService::new(&url).unwrap().chat(&json!({}), &mut |_| {}).unwrap_err();
        h.join().unwrap();
        assert_eq!(e, want, "status {status}");
    }
}

#[test]
fn an_unreachable_service_is_a_plain_failure() {
    // Port 9 (discard) on this machine; nothing listens.
    let e = HttpService::new("http://127.0.0.1:9").unwrap().chat(&json!({}), &mut |_| {}).unwrap_err();
    assert_eq!(e, SendError::Unreachable);
}

// ---- document extraction ----

fn docx(dir: &Path, name: &str, body_xml: &str) -> PathBuf {
    let path = dir.join(name);
    let mut z = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    z.start_file("word/document.xml", opts).unwrap();
    z.write_all(format!("<?xml version=\"1.0\"?><w:document><w:body>{body_xml}</w:body></w:document>").as_bytes()).unwrap();
    z.finish().unwrap();
    path
}

/// A one-page PDF with correct offsets, showing `text` (no parentheses or backslashes in it).
fn pdf(dir: &Path, name: &str, text: &str) -> PathBuf {
    let stream = format!("BT /F1 18 Tf 72 700 Td ({text}) Tj ET");
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_string(),
        format!("<< /Length {} >>\nstream\n{stream}\nendstream", stream.len()),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
    ];
    let mut out = String::from("%PDF-1.4\n");
    let mut offsets = vec![];
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.push_str(&format!("{} 0 obj\n{o}\nendobj\n", i + 1));
    }
    let xref = out.len();
    out.push_str(&format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1));
    for o in offsets {
        out.push_str(&format!("{o:010} 00000 n \n"));
    }
    out.push_str(&format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1));
    let path = dir.join(name);
    std::fs::write(&path, out).unwrap();
    path
}

#[test]
fn txt_is_read_as_is() {
    let d = tmp("txt");
    std::fs::write(d.join("a.TXT"), "  Meeting notes: budget review.\nAction: Test Person to confirm.  \n").unwrap();
    assert_eq!(extract_text(&d.join("a.TXT")).unwrap(), "Meeting notes: budget review.\nAction: Test Person to confirm.");
}

#[test]
fn docx_text_keeps_paragraphs_and_decodes_entities() {
    let d = tmp("docx");
    let p = docx(&d, "m.docx", "<w:p><w:r><w:t>Minutes &amp; actions</w:t></w:r></w:p><w:p><w:r><w:t xml:space=\"preserve\">Item 1</w:t></w:r><w:r><w:tab/><w:t>done &lt;ok&gt; &#65;</w:t></w:r></w:p>");
    assert_eq!(extract_text(&p).unwrap(), "Minutes & actions\nItem 1\tdone <ok> A");
    assert_eq!(xml_text("<w:p>a<w:br/>b</w:p>"), "a\nb\n");
}

#[test]
fn pdf_text_is_extracted() {
    let d = tmp("pdf");
    let p = pdf(&d, "r.pdf", "Quarterly summary for Test Ministry");
    assert!(extract_text(&p).unwrap().contains("Quarterly summary for Test Ministry"));
}

#[test]
fn unreadable_unsupported_empty_and_oversize_files_get_plain_refusals() {
    let d = tmp("bad-files");
    std::fs::write(d.join("a.doc"), "x").unwrap();
    assert_eq!(extract_text(&d.join("a.doc")), Err(ExtractError::Unsupported));
    assert_eq!(extract_text(&d.join("none.txt")), Err(ExtractError::Unreadable));
    std::fs::write(d.join("junk.pdf"), "not a pdf at all").unwrap();
    assert_eq!(extract_text(&d.join("junk.pdf")), Err(ExtractError::Unreadable));
    std::fs::write(d.join("junk.docx"), "not a zip").unwrap();
    assert_eq!(extract_text(&d.join("junk.docx")), Err(ExtractError::Unreadable));
    std::fs::write(d.join("bin.txt"), [0xff, 0xfe, 0x00, 0x9f]).unwrap();
    assert_eq!(extract_text(&d.join("bin.txt")), Err(ExtractError::Unreadable));
    std::fs::write(d.join("blank.txt"), "  \n ").unwrap();
    assert_eq!(extract_text(&d.join("blank.txt")), Err(ExtractError::Empty));
    std::fs::write(d.join("long.txt"), "a".repeat(MAX_TEXT_CHARS + 1)).unwrap();
    assert_eq!(extract_text(&d.join("long.txt")), Err(ExtractError::TooLong));
    let big = std::fs::File::create(d.join("big.txt")).unwrap();
    big.set_len(11 * 1024 * 1024).unwrap();
    assert_eq!(extract_text(&d.join("big.txt")), Err(ExtractError::TooBig));
    assert_eq!(extract_text(&d), Err(ExtractError::Unsupported));
    for e in [ExtractError::Unsupported, ExtractError::TooBig, ExtractError::TooLong, ExtractError::Empty, ExtractError::Unreadable] {
        assert!(!e.plain().is_empty());
    }
}

#[test]
fn extraction_writes_no_file() {
    let d = tmp("nowrite");
    let p = docx(&d, "m.docx", "<w:p><w:r><w:t>hello</w:t></w:r></w:p>");
    let before: Vec<_> = std::fs::read_dir(&d).unwrap().map(|e| e.unwrap().file_name()).collect();
    extract_text(&p).unwrap();
    let after: Vec<_> = std::fs::read_dir(&d).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(before, after);
}
