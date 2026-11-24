// Usage: scripts/validate-task <file>
// Exit 0 if valid; 1 with "path: message" lines if invalid; 2 on usage or unreadable input.
import { readFileSync } from "node:fs";
import { validateTask } from "./validate.js";

const file = process.argv[2];
if (!file || process.argv.length > 3) {
  console.error("usage: validate-task <file>");
  process.exit(2);
}
let doc;
try {
  doc = JSON.parse(readFileSync(file, "utf8"));
} catch (e) {
  console.error(`${file}: cannot read as JSON: ${e.message}`);
  process.exit(2);
}
const { valid, errors } = validateTask(doc);
if (valid) {
  console.log(`${file}: valid`);
} else {
  console.error(`${file}: INVALID`);
  for (const e of errors) console.error(`  ${e.path}: ${e.message}`);
  process.exit(1);
}
