// JSON.parse keeps the LAST of a repeated key, so a literal can hide in the file bytes while the
// parsed value looks clean. This scanner finds repeated keys at any depth. It validates syntax
// too, and throws SyntaxError on bad JSON, so it never disagrees with JSON.parse about what is JSON.
// Returns { doc, duplicates: [{ path, key }] } where path is the JSON Pointer of the repeated member.

const esc = (k) => k.replace(/~/g, "~0").replace(/\//g, "~1");

export function parseJson(text) {
  const doc = JSON.parse(text); // throws on invalid JSON; the scanner below assumes valid input
  const duplicates = [];
  let i = 0;
  const ws = () => {
    while (text[i] === " " || text[i] === "\t" || text[i] === "\n" || text[i] === "\r") i++;
  };
  const str = () => {
    const start = i++;
    while (text[i] !== '"') i += text[i] === "\\" ? 2 : 1;
    i++;
    return JSON.parse(text.slice(start, i));
  };
  const value = (path) => {
    ws();
    const c = text[i];
    if (c === "{") {
      i++;
      const seen = new Set();
      ws();
      if (text[i] === "}") return void i++;
      for (;;) {
        ws();
        const key = str();
        const at = `${path}/${esc(key)}`;
        if (seen.has(key)) duplicates.push({ path: at, key });
        seen.add(key);
        ws();
        i++; // ':'
        value(at);
        ws();
        if (text[i++] === "}") return;
      }
    } else if (c === "[") {
      i++;
      ws();
      if (text[i] === "]") return void i++;
      for (let n = 0; ; n++) {
        value(`${path}/${n}`);
        ws();
        if (text[i++] === "]") return;
      }
    } else if (c === '"') {
      str();
    } else {
      while (i < text.length && !/[\s,\]}]/.test(text[i])) i++;
    }
  };
  value("");
  return { doc, duplicates };
}
