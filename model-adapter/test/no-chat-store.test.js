import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

// #31: chat history lives only in the device's local chat table. The service keeps no table,
// file or log of chat content. The log path is covered by logging.test.js; this pins the rest:
// the adapter source has no database, no file write, and logs only through the metadata logger.
const src = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../src");

test("the service side of chat has no database, no file write and no raw console output", () => {
  const forbidden = [
    /sqlite/i, /CREATE\s+TABLE/i, /INSERT\s+INTO/i, /\bpg\b.*from/, /mongodb|redis|dynamodb/i,
    /writeFile|appendFile|createWriteStream|\bwriteSync\b/, /from\s+["']node:fs/, /require\(["']fs/,
  ];
  for (const f of fs.readdirSync(src)) {
    const text = fs.readFileSync(path.join(src, f), "utf8");
    for (const re of forbidden) assert.ok(!re.test(text), `${f} matches ${re}`);
    if (f !== "log.ts") assert.ok(!/console\./.test(text), `${f} writes to the console directly`);
  }
});
