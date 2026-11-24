//! Extracts the text of a dropped `.txt`, `.docx` or `.pdf`, on the device (#31). Nothing here
//! touches the network or writes a file. The text goes into the chat message and nowhere else.

use std::io::Read;
use std::path::Path;

/// Largest file we open.
pub const MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;
/// Longest text we hand to chat. Over this we refuse rather than cut a document short unseen.
pub const MAX_TEXT_CHARS: usize = 100_000;
/// Cap on what a .docx body may expand to (a zip bomb guard).
const MAX_XML_BYTES: u64 = 30 * 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub enum ExtractError {
    Unsupported,
    TooBig,
    TooLong,
    Empty,
    Unreadable,
}

impl ExtractError {
    pub fn plain(&self) -> &'static str {
        match self {
            ExtractError::Unsupported => "I can read .docx, .pdf and .txt files.",
            ExtractError::TooBig => "That file is over 10 MB. Try a smaller one.",
            ExtractError::TooLong => "That document is too long to summarise in one go. Try a shorter one or part of it.",
            ExtractError::Empty => "I found no text in that file. A scanned PDF has none to read.",
            ExtractError::Unreadable => "I could not read that file. It may be damaged or protected.",
        }
    }
}

pub fn extract_text(path: &Path) -> Result<String, ExtractError> {
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
    if !matches!(ext.as_str(), "txt" | "docx" | "pdf") {
        return Err(ExtractError::Unsupported);
    }
    let meta = std::fs::metadata(path).map_err(|_| ExtractError::Unreadable)?;
    if !meta.is_file() {
        return Err(ExtractError::Unreadable);
    }
    if meta.len() > MAX_FILE_BYTES {
        return Err(ExtractError::TooBig);
    }
    let text = match ext.as_str() {
        "txt" => txt(path)?,
        "docx" => docx(path)?,
        _ => pdf(path)?,
    };
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err(ExtractError::Empty);
    }
    if text.chars().count() > MAX_TEXT_CHARS {
        return Err(ExtractError::TooLong);
    }
    Ok(text)
}

fn txt(path: &Path) -> Result<String, ExtractError> {
    let bytes = std::fs::read(path).map_err(|_| ExtractError::Unreadable)?;
    String::from_utf8(bytes).map_err(|_| ExtractError::Unreadable)
}

fn docx(path: &Path) -> Result<String, ExtractError> {
    let file = std::fs::File::open(path).map_err(|_| ExtractError::Unreadable)?;
    let mut zip = zip::ZipArchive::new(file).map_err(|_| ExtractError::Unreadable)?;
    let entry = zip.by_name("word/document.xml").map_err(|_| ExtractError::Unreadable)?;
    let mut xml = String::new();
    entry.take(MAX_XML_BYTES + 1).read_to_string(&mut xml).map_err(|_| ExtractError::Unreadable)?;
    if xml.len() as u64 > MAX_XML_BYTES {
        return Err(ExtractError::TooBig);
    }
    Ok(xml_text(&xml))
}

/// Text of a WordprocessingML body: paragraph ends and breaks become newlines, tabs stay tabs,
/// every other tag is dropped, entities are decoded.
pub fn xml_text(xml: &str) -> String {
    let mut out = String::new();
    let mut rest = xml;
    while let Some(lt) = rest.find('<') {
        out.push_str(&unescape(&rest[..lt]));
        let Some(gt) = rest[lt..].find('>') else { break };
        let tag = &rest[lt + 1..lt + gt];
        let name = tag.trim_start_matches('/').split(|c: char| c.is_whitespace() || c == '/').next().unwrap_or("");
        let closing = tag.starts_with('/');
        match (name, closing) {
            ("w:p", true) | ("w:br", false) | ("w:cr", false) => out.push('\n'),
            ("w:tab", false) => out.push('\t'),
            _ => {}
        }
        rest = &rest[lt + gt + 1..];
    }
    out
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let tail = &rest[amp..];
        match tail.find(';').filter(|&i| i <= 10) {
            Some(semi) => {
                let ent = &tail[1..semi];
                let ch = match ent {
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "quot" => Some('"'),
                    "apos" => Some('\''),
                    _ => ent
                        .strip_prefix("#x")
                        .and_then(|h| u32::from_str_radix(h, 16).ok())
                        .or_else(|| ent.strip_prefix('#').and_then(|d| d.parse().ok()))
                        .and_then(char::from_u32),
                };
                match ch {
                    Some(c) => out.push(c),
                    None => out.push_str(&tail[..=semi]),
                }
                rest = &tail[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn pdf(path: &Path) -> Result<String, ExtractError> {
    let p = path.to_path_buf();
    // The parser can panic on a hostile file. A panic is "unreadable", not a crash of the shell.
    let r = std::panic::catch_unwind(move || pdf_extract::extract_text(&p));
    match r {
        Ok(Ok(t)) => Ok(t),
        _ => Err(ExtractError::Unreadable),
    }
}
