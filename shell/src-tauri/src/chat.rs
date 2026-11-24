//! Chat (TD 8, #31). The shell sends the conversation to the companion service's `POST /chat`
//! and streams the reply back. Chat history is written only to the local `chat` table
//! (encrypted store); this module writes nowhere else and logs nothing. The service address is
//! `COMPANION_SERVICE_URL`; the client holds no model credential and no model URL (decision 0003).
//!
//! The classification is the officer's. It is never changed, defaulted or lowered here: a chat
//! with none set is not sent, and a chat above the route's ceiling is refused by the service.

use crate::store::Store;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader};
use std::time::Duration;

/// Classification levels the officer can pick, lowest first. The service decides the ceiling;
/// anything it does not recognise it treats as above the ceiling.
pub const LEVELS: [&str; 5] = ["official-open", "official-closed", "restricted", "confidential", "secret"];

#[derive(Debug, PartialEq, Eq)]
pub enum SendError {
    /// No service address is configured, or it is not a safe one.
    NotConfigured,
    /// The officer has not set a classification for this chat.
    NoClassification,
    /// The service refused: the chat is above the route's ceiling. Nothing was sent to a model.
    AboveCeiling,
    BadMessages,
    TooLong,
    Unreachable,
    /// The route failed or the reply stopped before it finished.
    ReplyFailed,
    /// The reply arrived but could not be saved to the local history.
    NotSaved,
}

impl SendError {
    pub fn kind(&self) -> &'static str {
        match self {
            SendError::NotConfigured => "not_configured",
            SendError::NoClassification => "no_classification",
            SendError::AboveCeiling => "above_ceiling",
            SendError::BadMessages => "bad_messages",
            SendError::TooLong => "too_long",
            SendError::Unreachable => "unreachable",
            SendError::ReplyFailed => "reply_failed",
            SendError::NotSaved => "not_saved",
        }
    }

    /// What the officer reads. Plain words, no codes, no echo of what they wrote.
    pub fn plain(&self) -> &'static str {
        match self {
            SendError::NotConfigured => "Chat is not set up on this laptop yet. Nothing was sent.",
            SendError::NoClassification => "Choose a classification for this chat first. Nothing was sent.",
            SendError::AboveCeiling => "This chat is classified higher than the model is cleared for, so it was not sent. Your text is still here. Lower the classification only if it is truly lower, or do this another way.",
            SendError::BadMessages => "That message could not be sent. Nothing was sent.",
            SendError::TooLong => "That is too long to send. Try a shorter text or document.",
            SendError::Unreachable => "Could not reach the companion service. Your text is still here. Try again in a moment.",
            SendError::ReplyFailed => "The reply stopped early. Your text is still here. Try again.",
            SendError::NotSaved => "The reply could not be saved to your chat history on this laptop.",
        }
    }

    pub fn to_json(&self) -> Value {
        json!({ "kind": self.kind(), "message": self.plain() })
    }
}

/// The companion service's chat endpoint. Calls `on_text` with each reply chunk, in order.
pub trait Service {
    fn chat(&self, body: &Value, on_text: &mut dyn FnMut(&str)) -> Result<(), SendError>;
}

/// Plain http is allowed only to this machine (a local service during development).
/// Anything else must be https, so chat content never crosses a network unencrypted.
pub fn service_url_ok(url: &str) -> bool {
    if url.starts_with("https://") {
        return true;
    }
    let Some(rest) = url.strip_prefix("http://") else { return false };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit('@').next().unwrap_or("");
    host == "localhost"
        || host.starts_with("localhost:")
        || host == "127.0.0.1"
        || host.starts_with("127.0.0.1:")
        || host == "[::1]"
        || host.starts_with("[::1]:")
}

pub struct HttpService {
    base: String,
}

impl HttpService {
    pub fn new(base: &str) -> Result<HttpService, SendError> {
        let base = base.trim_end_matches('/');
        if !service_url_ok(base) {
            return Err(SendError::NotConfigured);
        }
        Ok(HttpService { base: base.to_string() })
    }

    pub fn from_env() -> Result<HttpService, SendError> {
        match std::env::var("COMPANION_SERVICE_URL") {
            Ok(u) if !u.is_empty() => HttpService::new(&u),
            _ => Err(SendError::NotConfigured),
        }
    }
}

impl Service for HttpService {
    fn chat(&self, body: &Value, on_text: &mut dyn FnMut(&str)) -> Result<(), SendError> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(10)))
            .timeout_recv_response(Some(Duration::from_secs(60)))
            .max_redirects(0)
            .build()
            .into();
        let resp = agent
            .post(&format!("{}/chat", self.base))
            .header("content-type", "application/json")
            .send(body.to_string().as_bytes())
            .map_err(|_| SendError::Unreachable)?;
        match resp.status().as_u16() {
            200 => {}
            403 => return Err(SendError::AboveCeiling),
            413 => return Err(SendError::TooLong),
            400 => return Err(SendError::BadMessages),
            408 | 502 | 504 => return Err(SendError::ReplyFailed),
            _ => return Err(SendError::Unreachable),
        }
        read_stream(BufReader::new(resp.into_body().into_reader()), on_text)
    }
}

/// Reads the service's newline-delimited reply: `{"text":..}` chunks, then `{"done":true}`.
/// A reply that ends without `done`, or carries an error, is a failure.
pub fn read_stream(reader: impl BufRead, on_text: &mut dyn FnMut(&str)) -> Result<(), SendError> {
    for line in reader.lines() {
        let line = line.map_err(|_| SendError::ReplyFailed)?;
        if line.trim().is_empty() {
            continue;
        }
        let v: Value = serde_json::from_str(&line).map_err(|_| SendError::ReplyFailed)?;
        if v.get("error").is_some() {
            return Err(SendError::ReplyFailed);
        }
        if let Some(t) = v.get("text").and_then(Value::as_str) {
            on_text(t);
        }
        if v.get("done") == Some(&Value::Bool(true)) {
            return Ok(());
        }
    }
    Err(SendError::ReplyFailed)
}

/// Checks the shape of the conversation the canvas sends: a non-empty list of user/assistant
/// messages with string content, ending on a user message.
pub fn check_messages(messages: &Value) -> Result<&Vec<Value>, SendError> {
    let list = messages.as_array().filter(|l| !l.is_empty()).ok_or(SendError::BadMessages)?;
    for m in list {
        let role = m.get("role").and_then(Value::as_str);
        let ok = matches!(role, Some("user") | Some("assistant")) && m.get("content").map_or(false, Value::is_string);
        if !ok {
            return Err(SendError::BadMessages);
        }
    }
    if list.last().and_then(|m| m.get("role")).and_then(Value::as_str) != Some("user") {
        return Err(SendError::BadMessages);
    }
    Ok(list)
}

/// One chat turn. Sends the conversation, streams chunks to `on_text`, and on a complete reply
/// writes the last user message and the reply to the local `chat` table. A refused or failed
/// turn writes nothing, so history holds only what the officer saw answered.
pub fn chat_turn(
    service: &dyn Service,
    store: &Store,
    classification: &str,
    messages: &Value,
    now: i64,
    on_text: &mut dyn FnMut(&str),
) -> Result<(), SendError> {
    if classification.trim().is_empty() {
        return Err(SendError::NoClassification);
    }
    let list = check_messages(messages)?;
    let body = json!({ "classification": classification, "messages": list });
    let mut reply = String::new();
    service.chat(&body, &mut |t| {
        reply.push_str(t);
        on_text(t);
    })?;
    let user = list.last().and_then(|m| m["content"].as_str()).unwrap_or("");
    store.add_chat(now, "user", user).map_err(|_| SendError::NotSaved)?;
    store.add_chat(now, "assistant", &reply).map_err(|_| SendError::NotSaved)?;
    Ok(())
}
