import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

// The client (shell/, executor-macos/) holds no model credentials and no direct model URL (decision 0003.1).
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const DIRS = ["shell", "executor-macos"];
const MAX_BYTES = 20_000_000;

const HOSTS = [
  "api.anthropic.com", "api.openai.com", "generativelanguage.googleapis.com", "aiplatform.googleapis.com",
  "openai.azure.com", "api.cohere.", "api.mistral.ai", "api.groq.com", "openrouter.ai", "api.x.ai",
  "api.together.xyz", "api.deepseek.com", "api.perplexity.ai", "api-inference.huggingface.co", "api.fireworks.ai",
];
const FORBIDDEN = [
  ...HOSTS.map((h) => new RegExp(h.replace(/\./g, "\\."), "i")),
  /bedrock[\w.-]*\.amazonaws\.com/i,
  /\/v1\/messages\b/, /\/v1\/chat\/completions\b/, /\/model\/[^"'\s]+\/(invoke|converse)/,
  /\b[A-Z][A-Z0-9_]*(API_KEY|AUTH_TOKEN|ACCESS_TOKEN)\b/,
  /\b(HF_TOKEN|HUGGINGFACE_TOKEN|HUGGING_FACE_HUB_TOKEN)\b/,
  /\bAWS_(ACCESS_KEY_ID|SECRET_ACCESS_KEY|SESSION_TOKEN)\b/,
  /\bsk-ant-[A-Za-z0-9_-]{8,}/, /\bsk-(proj|svcacct|admin)-[A-Za-z0-9_-]{16,}/, /\bsk-[A-Za-z0-9]{20,}/,
  /\bhf_[A-Za-z0-9]{20,}/, /\bgsk_[A-Za-z0-9]{20,}/, /\bxai-[A-Za-z0-9]{20,}/,
  /\bAKIA[0-9A-Z]{16}\b/, /\bAIza[0-9A-Za-z_-]{35}\b/,
  /x-api-key/i, /anthropic-version/i,
];

/** Views of a file's text: raw (NULs dropped so UTF-16 reads as ASCII), string concatenations joined, base64 tokens decoded. */
export function views(buf) {
  const raw = buf.toString("latin1").replace(/\0/g, "");
  const joined = raw.replace(/["'`]\s*(\+|\.\.|,)?\s*["'`]/g, "");
  const decoded = [];
  for (const tok of raw.match(/[A-Za-z0-9+/_-]{24,}={0,2}/g) ?? []) {
    try {
      const d = Buffer.from(tok.replace(/-/g, "+").replace(/_/g, "/"), "base64").toString("latin1");
      if (/^[\x20-\x7e\n\r\t]+$/.test(d)) decoded.push(d);
    } catch { /* not base64 */ }
  }
  return [raw, joined, ...decoded];
}

export function scan(input) {
  const buf = Buffer.isBuffer(input) ? input : Buffer.from(input, "utf8");
  const hits = new Set();
  for (const v of views(buf)) for (const re of FORBIDDEN) if (re.test(v)) hits.add(String(re));
  return [...hits];
}

/** Every file git knows in `dirs` (tracked, and untracked but not ignored), so no directory name can hide one. */
export function listFiles(repo, dirs) {
  const out = execFileSync("git", ["ls-files", "-z", "--cached", "--others", "--exclude-standard", "--", ...dirs], { cwd: repo, maxBuffer: 1 << 28 });
  return out.toString("utf8").split("\0").filter(Boolean);
}

/** Returns problems: forbidden content, plus anything we could not read with confidence. */
export function check(repo, dirs) {
  const problems = [];
  let scanned = 0;
  for (const rel of listFiles(repo, dirs)) {
    const p = path.join(repo, rel);
    let st;
    try { st = fs.lstatSync(p); } catch { continue; } // deleted in the working tree
    if (st.isSymbolicLink()) { problems.push(`${rel}: symlink (not followed; remove it or scan its target)`); continue; }
    if (!st.isFile()) continue;
    if (st.size > MAX_BYTES) { problems.push(`${rel}: ${st.size} bytes is over the ${MAX_BYTES} scan limit`); continue; }
    scanned++;
    for (const h of scan(fs.readFileSync(p))) problems.push(`${rel}: ${h}`);
  }
  return { problems, scanned };
}

test("shell/ and executor-macos/ hold no model credential or direct model URL", () => {
  const { problems, scanned } = check(root, DIRS);
  assert.ok(scanned > 10, "scanned a real tree");
  assert.deepEqual(problems, []);
});

test("the scanner catches what it is meant to catch", () => {
  const cases = {
    "anthropic url": 'url = "https://api.anthropic.com/v1/messages"',
    "env key": 'environment["OPENAI_API_KEY"]',
    "bedrock": "https://bedrock-runtime.ap-southeast-1.amazonaws.com/model/x/invoke",
    "bedrock short": "https://bedrock.us-east-1.amazonaws.com",
    "sk-ant": "key: sk-" + "ant-api03-abcdefghij",
    "sk-proj": 'let k = "sk-' + 'proj-abcdefghijklmnopqrstuvwxyz0123456789"',
    "auth token": 'env["ANTHROPIC_AUTH_TOKEN"]',
    "claude key": "CLAUDE_API" + "_KEY",
    "hf": "HF_TOKEN=hf" + "_abcdefghijklmnopqrstuvwxyz",
    "groq": "https://api.groq.com/openai/v1",
    "openrouter": "https://openrouter.ai/api/v1",
    "xai": "https://api.x.ai/v1",
    "split": '"https://api.anthr" + "opic.com"',
    "base64": Buffer.from("https://api.anthropic.com").toString("base64"),
  };
  for (const [name, text] of Object.entries(cases)) assert.ok(scan(text).length, name);
  assert.ok(scan(Buffer.from('let u = "https://api.anthropic.com"', "utf16le")).length, "utf-16");
  assert.ok(scan(Buffer.from('let u = "https://api.anthropic.com"\0')).length, "nul");
  assert.deepEqual(scan('let url = "http://127.0.0.1:7777/chat"'), []);
});

test("the walker sees files in gen/dist/anywhere, UTF-16 and NUL files, and flags symlinks and oversize files", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "creds-"));
  const git = (...a) => execFileSync("git", a, { cwd: dir, stdio: "pipe" });
  git("init", "-q");
  const bad = 'let u = "https://api.anthropic.com/v1/messages"';
  const put = (rel, data) => { fs.mkdirSync(path.dirname(path.join(dir, rel)), { recursive: true }); fs.writeFileSync(path.join(dir, rel), data); };
  put("shell/src-tauri/gen/a.swift", bad);
  put("shell/dist/b.js", bad);
  put("shell/u16.swift", Buffer.from(bad, "utf16le"));
  put("shell/nul.swift", bad + "\0");
  put("shell/ok.swift", "let a = 1");
  fs.symlinkSync("ok.swift", path.join(dir, "shell/link.swift"));
  put("shell/big.bin", Buffer.alloc(MAX_BYTES + 1));
  const { problems } = check(dir, ["shell"]);
  const text = problems.join("\n");
  for (const f of ["gen/a.swift", "dist/b.js", "u16.swift", "nul.swift", "link.swift", "big.bin"]) assert.ok(text.includes(f), f + " flagged:\n" + text);
  assert.ok(!text.includes("ok.swift:"), "clean file not flagged");
  fs.rmSync(dir, { recursive: true, force: true });
});
