// Usage: scripts/validate-task <file>
// Exit 0 if valid; 1 with "path: message" lines if invalid; 2 on usage or unreadable input.
import { readFileSync } from "node:fs";
import { validateText } from "./validate.js";

const file = process.argv[2];
if (!file || process.argv.length > 3) {
  console.error("usage: validate-task <file>");
  process.exit(2);
}
let result;
try {
  result = validateText(readFileSync(file, "utf8"));
} catch (e) {
  console.error(`${file}: cannot read as JSON: ${e.message}`);
  process.exit(2);
}
const { valid, errors } = result;
if (valid) {
  console.log(`${file}: valid`);
} else {
  console.error(`${file}: INVALID`);
  for (const e of errors) console.error(`  ${e.path}: ${e.message}`);
  process.exit(1);
}
