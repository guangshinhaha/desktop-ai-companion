// Writes staff.csv and staff.xlsx from a fixed list of made-up names.
// Run: npm run generate (from stand-in/). Output is deterministic.
// Columns: full_name, email, school, role. No NRIC, phone or address.
import { writeFileSync } from "node:fs";
import { crc32 } from "node:zlib";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

export const COLUMNS = ["full_name", "email", "school", "role"] as const;

export const STAFF: readonly (readonly [string, string, string, string])[] = [
  ["Aiden Tan Example", "aiden.example@staff.example.test", "Northgate Primary", "Teacher"],
  ["Bella Koh Example", "bella.example@staff.example.test", "Riverside Secondary", "Form Teacher"],
  ["Chen Wei Example", "chen.example@staff.example.test", "Hillcrest Junior College", "Head of Department"],
  ["Divya Rao Example", "divya.example@staff.example.test", "Lakeview Primary", "Admin Clerk"],
  ["Evan Lim Example", "evan.example@staff.example.test", "Eastbrook Secondary", "Teacher"],
];

const xml = (s: string) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

export function toCsv(): string {
  return [COLUMNS.join(","), ...STAFF.map((r) => r.join(","))].join("\n") + "\n";
}

function sheetXml(): string {
  const rows: (readonly string[])[] = [COLUMNS, ...STAFF];
  const cell = (v: string, c: number, r: number) =>
    `<c r="${String.fromCharCode(65 + c)}${r + 1}" t="inlineStr"><is><t>${xml(v)}</t></is></c>`;
  const body = rows
    .map((row, r) => `<row r="${r + 1}">${row.map((v, c) => cell(v, c, r)).join("")}</row>`)
    .join("");
  return `<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>${body}</sheetData></worksheet>`;
}

// Minimal zip writer (stored, fixed timestamp so output is reproducible).
function zip(files: [string, string][]): Buffer {
  const parts: Buffer[] = [];
  const central: Buffer[] = [];
  let offset = 0;
  for (const [name, text] of files) {
    const raw = Buffer.from(text, "utf8");
    const data = raw; // stored, not deflated: byte-identical on any zlib build
    const nameBuf = Buffer.from(name);
    const crc = crc32(raw);
    const local = Buffer.alloc(30);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(20, 4);
    local.writeUInt16LE(0, 8);
    local.writeUInt16LE(0x21, 12); // date 1980-01-01
    local.writeUInt32LE(crc, 14);
    local.writeUInt32LE(data.length, 18);
    local.writeUInt32LE(raw.length, 22);
    local.writeUInt16LE(nameBuf.length, 26);
    parts.push(local, nameBuf, data);
    const cd = Buffer.alloc(46);
    cd.writeUInt32LE(0x02014b50, 0);
    cd.writeUInt16LE(20, 4);
    cd.writeUInt16LE(20, 6);
    cd.writeUInt16LE(0, 10);
    cd.writeUInt16LE(0x21, 14);
    cd.writeUInt32LE(crc, 16);
    cd.writeUInt32LE(data.length, 20);
    cd.writeUInt32LE(raw.length, 24);
    cd.writeUInt16LE(nameBuf.length, 28);
    cd.writeUInt32LE(offset, 42);
    central.push(cd, nameBuf);
    offset += 30 + nameBuf.length + data.length;
  }
  const cdBuf = Buffer.concat(central);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(files.length, 8);
  end.writeUInt16LE(files.length, 10);
  end.writeUInt32LE(cdBuf.length, 12);
  end.writeUInt32LE(offset, 16);
  return Buffer.concat([...parts, cdBuf, end]);
}

const NS_REL = "http://schemas.openxmlformats.org/package/2006/relationships";
const NS_DOC = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const HDR = `<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n`;

export function toXlsx(): Buffer {
  return zip([
    ["[Content_Types].xml", `${HDR}<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/></Types>`],
    ["_rels/.rels", `${HDR}<Relationships xmlns="${NS_REL}"><Relationship Id="rId1" Type="${NS_DOC}/officeDocument" Target="xl/workbook.xml"/></Relationships>`],
    ["xl/workbook.xml", `${HDR}<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="${NS_DOC}"><sheets><sheet name="Staff" sheetId="1" r:id="rId1"/></sheets></workbook>`],
    ["xl/_rels/workbook.xml.rels", `${HDR}<Relationships xmlns="${NS_REL}"><Relationship Id="rId1" Type="${NS_DOC}/worksheet" Target="worksheets/sheet1.xml"/></Relationships>`],
    ["xl/worksheets/sheet1.xml", sheetXml()],
  ]);
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const dir = dirname(fileURLToPath(import.meta.url));
  writeFileSync(join(dir, "staff.csv"), toCsv());
  writeFileSync(join(dir, "staff.xlsx"), toXlsx());
  console.log(`wrote ${STAFF.length} rows to staff.csv and staff.xlsx`);
}
