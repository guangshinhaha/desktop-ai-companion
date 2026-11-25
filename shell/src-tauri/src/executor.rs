//! Starts the executor sidecar on demand and talks JSON-RPC to it over stdio (TD 3.2).
//! The executor exists only for the length of a call: stdin closes, it exits, nothing idles.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// Ceiling for one reply. A bound on waiting, not a pause: the reply is awaited on a channel.
const REPLY_CEILING: Duration = Duration::from_secs(10);

/// Where the executor binary lives: `EXECUTOR_BIN`, else next to this app's own binary.
pub fn executor_path() -> PathBuf {
    if let Some(p) = std::env::var_os("EXECUTOR_BIN") {
        return PathBuf::from(p);
    }
    let mut p = std::env::current_exe().unwrap_or_default();
    p.set_file_name("executor-macos");
    p
}

/// Builds a JSON-RPC 2.0 request line.
pub fn request(id: u64, method: &str) -> String {
    format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"{method}"}}"#)
}

/// Checks a reply line answers request `id` with a pong. Returns the executor version.
pub fn parse_pong(line: &str, id: u64) -> Result<String, String> {
    let v: serde_json::Value = serde_json::from_str(line).map_err(|e| format!("bad reply: {e}"))?;
    if v["id"] != id {
        return Err(format!("reply id {} does not match {id}", v["id"]));
    }
    if let Some(e) = v.get("error") {
        return Err(format!("executor error: {e}"));
    }
    if v["result"]["pong"] != true {
        return Err("reply is not a pong".into());
    }
    Ok(v["result"]["version"].as_str().unwrap_or("unknown").to_string())
}

/// Starts the executor, pings it, shuts it down. Returns its version.
pub fn ping(bin: &Path) -> Result<String, String> {
    let mut child = Command::new(bin)
        .arg("serve")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("cannot start {}: {e}", bin.display()))?;
    let mut stdin = child.stdin.take().ok_or("no stdin")?;
    let stdout = child.stdout.take().ok_or("no stdout")?;

    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let _ = BufReader::new(stdout).read_line(&mut line);
        let _ = tx.send(line);
    });

    let result = (|| {
        writeln!(stdin, "{}", request(1, "ping")).map_err(|e| format!("write failed: {e}"))?;
        let line = rx
            .recv_timeout(REPLY_CEILING)
            .map_err(|_| "no reply from the executor within the ceiling".to_string())?;
        parse_pong(line.trim(), 1)
    })();

    drop(stdin); // EOF tells the executor to exit
    if result.is_err() {
        let _ = child.kill();
    }
    let _ = child.wait();
    result
}

/// Checks an audit export with `executor-macos verify-audit`. The export goes to its standard input, so
/// the approved values never touch the disk. Ok holds the verifier's line; Err holds why it failed, which
/// names the broken sequence number.
pub fn verify_audit(bin: &Path, export: &str) -> Result<String, String> {
    let mut child = Command::new(bin)
        .arg("verify-audit")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot start {}: {}", bin.display(), e.kind()))?;
    let mut stdin = child.stdin.take().ok_or("no stdin")?;
    let data = export.to_string();
    // Written on a thread so a large trail cannot fill the pipe while we wait for the verdict.
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(data.as_bytes());
    });
    let out = child.wait_with_output().map_err(|e| format!("verifier failed: {}", e.kind()))?;
    let _ = writer.join();
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if out.status.success() {
        Ok(text)
    } else if text.is_empty() {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    } else {
        Err(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_is_json_rpc() {
        let v: serde_json::Value = serde_json::from_str(&request(3, "ping")).unwrap();
        assert_eq!(v["jsonrpc"], "2.0");
        assert_eq!(v["id"], 3);
        assert_eq!(v["method"], "ping");
    }

    #[test]
    fn pong_is_accepted() {
        let r = r#"{"id":1,"jsonrpc":"2.0","result":{"pong":true,"version":"0.1.0"}}"#;
        assert_eq!(parse_pong(r, 1).unwrap(), "0.1.0");
    }

    #[test]
    fn wrong_id_error_and_garbage_are_rejected() {
        assert!(parse_pong(r#"{"id":2,"result":{"pong":true}}"#, 1).is_err());
        assert!(parse_pong(r#"{"id":1,"error":{"code":-32601,"message":"x"}}"#, 1).is_err());
        assert!(parse_pong("nope", 1).is_err());
    }

    #[test]
    fn missing_binary_is_an_error() {
        assert!(ping(Path::new("/nonexistent/executor")).is_err());
    }

    /// Runs against the real Swift executor. scripts/test.sh builds it first and sets EXECUTOR_BIN.
    #[test]
    fn pings_the_real_executor() {
        let Some(bin) = std::env::var_os("EXECUTOR_BIN") else {
            eprintln!("EXECUTOR_BIN not set: skipping (scripts/test.sh sets it)");
            return;
        };
        assert_eq!(ping(Path::new(&bin)).unwrap(), "0.1.0");
    }
}
