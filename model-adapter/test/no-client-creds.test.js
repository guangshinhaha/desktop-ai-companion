import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

// The client (shell/, executor-macos/) holds no model credentials and no direct model URL (decision 0003.1).
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const DIRS = ["shell", "executor-macos"];
const SKIP_DIRS = new Set(["node_modules", "target", ".build", ".git", "gen", "dist"]);
const FORBIDDEN = [
  /api\.anthropic\.com/i, /api\.openai\.com/i, /generativelanguage\.googleapis\.com/i, /aiplatform\.googleapis\.com/i,
  /bedrock(-runtime)?[.\w-]*\.amazonaws\.com/i, /openai\.azure\.com/i, /api\.cohere\./i, /api\.mistral\.ai/i,
  /\/v1\/messages\b/, /\/v1\/chat\/completions\b/, /\/model\/[^"'\s]+\/(invoke|converse)/,
  /\b(ANTHROPIC|OPENAI|AZURE_OPENAI|COHERE|MISTRAL|GEMINI|GOOGLE_API)_(API_)?KEY\b/,
  /\bAWS_(ACCESS_KEY_ID|SECRET_ACCESS_KEY|SESSION_TOKEN)\b/,
  /\bsk-ant-[A-Za-z0-9_-]{8,}/, /\bsk-[A-Za-z0-9]{20,}/, /\bAKIA[0-9A-Z]{16}\b/, /\bAIza[0-9A-Za-z_-]{35}\b/,
  /x-api-key/i, /anthropic-version/i,
];

export function* files(dir) {
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    if (SKIP_DIRS.has(e.name)) continue;
    const p = path.join(dir, e.name);
    if (e.isDirectory()) yield* files(p);
    else if (e.isFile()) yield p;
  }
}

export function scan(text) {
  return FORBIDDEN.filter((re) => re.test(text)).map(String);
}

test("shell/ and executor-macos/ hold no model credential or direct model URL", () => {
  const hits = [];
  let scanned = 0;
  for (const d of DIRS) {
    assert.ok(fs.existsSync(path.join(root, d)), `${d} exists`);
    for (const f of files(path.join(root, d))) {
      if (fs.statSync(f).size > 5_000_000) continue;
      const buf = fs.readFileSync(f);
      if (buf.includes(0)) continue; // binary
      scanned++;
      for (const h of scan(buf.toString("utf8"))) hits.push(`${path.relative(root, f)}: ${h}`);
    }
  }
  assert.ok(scanned > 10, "scanned a real tree");
  assert.deepEqual(hits, []);
});

test("the scanner catches what it is meant to catch", () => {
  assert.ok(scan('url = "https://api.anthropic.com/v1/messages"').length);
  assert.ok(scan("let k = ProcessInfo.processInfo.environment[\"OPENAI_API_KEY\"]").length);
  assert.ok(scan("https://bedrock-runtime.ap-southeast-1.amazonaws.com/model/x/invoke").length);
  assert.ok(scan("key: sk-ant-api03-abcdefghij").length);
  assert.deepEqual(scan("let url = \"http://127.0.0.1:7777/chat\""), []);
});
