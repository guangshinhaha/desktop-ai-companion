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
use std::net::{Ipv4Addr, Ipv6Addr};
use std::time::Duration;
use url::{Host, Url};

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
/// Anything else must be https, so chat content never crosses a network unencrypted. No user info.
/// Returns the parsed URL: requests are built from it, so the host checked is the host called.
fn parse_service_url(url: &str) -> Option<Url> {
    let url = Url::parse(url).ok()?;
    if !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    let ok = match (url.scheme(), url.host()?) {
        ("https", _) => true,
        ("http", Host::Domain(d)) => d == "localhost",
        ("http", Host::Ipv4(ip)) => ip == Ipv4Addr::LOCALHOST,
        ("http", Host::Ipv6(ip)) => ip == Ipv6Addr::LOCALHOST,
        _ => false,
    };
    ok.then_some(url)
}

pub fn service_url_ok(url: &str) -> bool {
    parse_service_url(url).is_some()
}

pub struct HttpService {
    base: Url,
}

impl HttpService {
    pub fn new(base: &str) -> Result<HttpService, SendError> {
        let base = parse_service_url(base).ok_or(SendError::NotConfigured)?;
        Ok(HttpService { base })
    }

    fn endpoint(&self, name: &str) -> String {
        let mut url = self.base.clone();
        // http and https URLs always have path segments.
        if let Ok(mut path) = url.path_segments_mut() {
            path.pop_if_empty().push(name);
        }
        url.into()
    }

    /// Plain http is only ever this machine, so it connects directly: a proxy variable would carry
    /// the request, in clear text, to wherever it points. https still follows the proxy settings.
    fn proxy(&self) -> Option<ureq::Proxy> {
        match self.base.scheme() {
            "https" => ureq::Proxy::try_from_env(),
            _ => None,
        }
    }

    pub fn from_env() -> Result<HttpService, SendError> {
        match std::env::var("COMPANION_SERVICE_URL") {
            Ok(u) if !u.is_empty() => HttpService::new(&u),
            _ => Err(SendError::NotConfigured),
        }
    }

    /// Posts a replay's recovery request to `POST /recover` (#34). Returns the answer object, or None for
    /// anything else: unreachable, refused above the ceiling, a route error, or a reply that is not an object.
    /// The executor checks the answer again; None makes it ask the officer.
    pub fn recover(&self, body: &Value) -> Option<Value> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(10)))
            .timeout_global(Some(Duration::from_secs(45)))
            .max_redirects(0)
            .proxy(self.proxy())
            .build()
            .into();
        let mut resp = agent
            .post(&self.endpoint("recover"))
            .header("content-type", "application/json")
            .send(body.to_string().as_bytes())
            .ok()?;
        if resp.status().as_u16() != 200 {
            return None;
        }
        let text = resp.body_mut().with_config().limit(64 * 1024).read_to_string().ok()?;
        serde_json::from_str::<Value>(&text).ok().filter(Value::is_object)
    }
}

impl Service for HttpService {
    fn chat(&self, body: &Value, on_text: &mut dyn FnMut(&str)) -> Result<(), SendError> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(10)))
            .timeout_recv_response(Some(Duration::from_secs(60)))
            .max_redirects(0)
            .proxy(self.proxy())
            .build()
            .into();
        let mut resp = agent
            .post(&self.endpoint("chat"))
            .header("content-type", "application/json")
            .send(body.to_string().as_bytes())
            .map_err(|_| SendError::Unreachable)?;
        match resp.status().as_u16() {
            200 => {}
            // Only the ceiling refusal may suggest the classification. The service also answers 403
            // for other reasons (an `Origin` header), and any proxy can.
            403 => {
                let text = resp.body_mut().with_config().limit(64 * 1024).read_to_string().unwrap_or_default();
                let ceiling = serde_json::from_str::<Value>(&text).is_ok_and(|v| v["error"] == "above_ceiling");
                return Err(if ceiling { SendError::AboveCeiling } else { SendError::Unreachable });
            }
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
