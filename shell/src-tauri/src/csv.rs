//! Reads a CSV file on this machine for a list run (TD 6). Nothing leaves the device and nothing is logged.
//! Errors name a row number or a column name, never a cell's content (PDPA).
//!
//! Excel (.xlsx) is not read: Shin closed #11 as "Numbers and Sheets only". Save the sheet as CSV.

use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

pub const MAX_FILE_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_ROWS: usize = 5000;
pub const MAX_COLUMNS: usize = 64;
/// Same as the executor's `RunInputs.maxValueLength`: a longer value would be refused there anyway.
pub const MAX_VALUE_CHARS: usize = 256;

#[derive(Debug, PartialEq)]
pub struct Csv {
    pub header: Vec<String>,
    /// Data rows, trimmed. Row `n` of the run is `rows[n - 1]`.
    pub rows: Vec<Vec<String>>,
}

/// What the task asks for, per input.
#[derive(Debug, Clone, PartialEq)]
pub struct InputSpec {
    pub name: String,
    pub choices: Option<Vec<String>>,
}

/// The inputs a saved task declares, from its body.
pub fn task_inputs(task: &Value) -> Vec<InputSpec> {
    task["inputs"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|i| {
                    let name = i["name"].as_str()?.to_string();
                    let choices = if i["type"] == "choice" {
                        i["choices"].as_array().map(|c| c.iter().filter_map(|x| x.as_str().map(String::from)).collect())
                    } else {
                        None
                    };
                    Some(InputSpec { name, choices })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Parses CSV text: comma separated, `"` quoting with `""` for a quote, line breaks allowed inside quotes,
/// a leading byte-order mark ignored, blank lines skipped. Every row must have the header's width.
pub fn parse(bytes: &[u8]) -> Result<Csv, String> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(format!("the file is larger than {} MB", MAX_FILE_BYTES / 1024 / 1024));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "the file is not UTF-8 text; save the sheet as CSV (UTF-8)".to_string())?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);

    let mut records: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut cell = String::new();
    let mut in_quotes = false;
    let mut quoted = false; // this cell had quotes: a quoted empty cell is still a cell
    let mut chars = text.chars().peekable();
    let end_cell =|row: &mut Vec<String>, cell: &mut String, quoted: &mut bool| {
        row.push(cell.trim().to_string());
        cell.clear();
        *quoted = false;
    };
    while let Some(c) = chars.next() {
        if in_quotes {
            match c {
                '"' if chars.peek() == Some(&'"') => {
                    chars.next();
                    cell.push('"');
                }
                '"' => in_quotes = false,
                _ => cell.push(c),
            }
            continue;
        }
        match c {
            '"' if cell.trim().is_empty() => {
                in_quotes = true;
                quoted = true;
                cell.clear();
            }
            ',' => end_cell(&mut row, &mut cell, &mut quoted),
            '\r' | '\n' => {
                if c == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                let blank = row.is_empty() && cell.is_empty() && !quoted;
                if !blank {
                    end_cell(&mut row, &mut cell, &mut quoted);
                    records.push(std::mem::take(&mut row));
                }
            }
            _ => cell.push(c),
        }
        if records.len() > MAX_ROWS + 1 {
            return Err(format!("more than {MAX_ROWS} rows"));
        }
    }
    if in_quotes {
        return Err("a quoted value is never closed".into());
    }
    if !(row.is_empty() && cell.is_empty() && !quoted) {
        end_cell(&mut row, &mut cell, &mut quoted);
        records.push(row);
    }

    let mut it = records.into_iter();
    let header = it.next().ok_or("the file is empty")?;
    if header.len() > MAX_COLUMNS {
        return Err(format!("more than {MAX_COLUMNS} columns"));
    }
    if header.iter().any(|h| h.is_empty()) {
        return Err("a column has no name in the first row".into());
    }
    let mut seen = std::collections::BTreeSet::new();
    if let Some(d) = header.iter().find(|h| !seen.insert(h.as_str())) {
        return Err(format!("two columns are both called \"{d}\""));
    }
    let rows: Vec<Vec<String>> = it.collect();
    if rows.is_empty() {
        return Err("the file has a header but no rows".into());
    }
    for (i, r) in rows.iter().enumerate() {
        if r.len() != header.len() {
            return Err(format!("row {} has {} values but the header has {}", i + 1, r.len(), header.len()));
        }
        if r.iter().any(|v| v.chars().any(|c| c.is_control())) {
            return Err(format!("row {} holds a control character", i + 1));
        }
    }
    Ok(Csv { header, rows })
}

/// Reads and parses a file. `.xlsx` and `.xls` are refused in plain words.
pub fn read_file(path: &std::path::Path) -> Result<(Csv, Vec<u8>), String> {
    let ext = path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase());
    if matches!(ext.as_deref(), Some("xlsx") | Some("xls")) {
        return Err("Excel files are not read yet. Save the sheet as CSV and pick that.".into());
    }
    let meta = std::fs::metadata(path).map_err(|_| "cannot open the file".to_string())?;
    if !meta.is_file() {
        return Err("that is not a file".into());
    }
    if meta.len() as usize > MAX_FILE_BYTES {
        return Err(format!("the file is larger than {} MB", MAX_FILE_BYTES / 1024 / 1024));
    }
    let bytes = std::fs::read(path).map_err(|_| "cannot read the file".to_string())?;
    let csv = parse(&bytes)?;
    Ok((csv, bytes))
}

/// SHA-256 of the file's bytes, as lowercase hex. A run remembers this so a resume can refuse a file that changed.
pub fn fingerprint(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Maps each task input to a column of the same name, when there is one.
pub fn suggest_mapping(header: &[String], inputs: &[InputSpec]) -> BTreeMap<String, String> {
    inputs
        .iter()
        .filter(|i| header.iter().any(|h| h == &i.name))
        .map(|i| (i.name.clone(), i.name.clone()))
        .collect()
}

/// One value that cannot be typed. Names the row and the input, never the value.
#[derive(Debug, PartialEq)]
pub struct Problem {
    pub row: usize,
    pub input: String,
    pub why: &'static str,
}

/// The inputs object for each row, in row order. `mapping` is input name to column name. Refuses before any
/// row runs: an unmapped input, an unknown column, or any row whose value is empty, too long or not one of
/// a choice input's choices. Up to 20 problems are listed.
pub fn build_rows(csv: &Csv, mapping: &BTreeMap<String, String>, inputs: &[InputSpec]) -> Result<Vec<Value>, String> {
    let mut cols: Vec<(&InputSpec, usize)> = Vec::new();
    for spec in inputs {
        let col = mapping.get(&spec.name).ok_or_else(|| format!("the input \"{}\" is not matched to a column", spec.name))?;
        let idx = csv
            .header
            .iter()
            .position(|h| h == col)
            .ok_or_else(|| format!("the column \"{col}\" is not in the file"))?;
        cols.push((spec, idx));
    }
    if let Some(extra) = mapping.keys().find(|k| !inputs.iter().any(|i| &i.name == *k)) {
        return Err(format!("\"{extra}\" is not an input of this task"));
    }
    let mut problems: Vec<Problem> = Vec::new();
    let mut out = Vec::with_capacity(csv.rows.len());
    for (r, row) in csv.rows.iter().enumerate() {
        let mut obj = Map::new();
        for (spec, idx) in &cols {
            let v = &row[*idx];
            let why = if v.is_empty() {
                Some("is empty")
            } else if v.chars().count() > MAX_VALUE_CHARS {
                Some("is too long")
            } else if spec.choices.as_ref().is_some_and(|c| !c.contains(v)) {
                Some("is not one of the choices")
            } else {
                None
            };
            if let Some(why) = why {
                problems.push(Problem { row: r + 1, input: spec.name.clone(), why });
            }
            obj.insert(spec.name.clone(), json!(v));
        }
        out.push(Value::Object(obj));
    }
    if !problems.is_empty() {
        let shown: Vec<String> = problems.iter().take(20).map(|p| format!("row {} {} {}", p.row, p.input, p.why)).collect();
        let more = if problems.len() > 20 { format!(" (and {} more)", problems.len() - 20) } else { String::new() };
        return Err(format!("fix the file first: {}{more}", shown.join("; ")));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(n: &str) -> InputSpec {
        InputSpec { name: n.into(), choices: None }
    }

    #[test]
    fn parses_quotes_commas_newlines_bom_and_crlf() {
        let c = parse("\u{feff}a,b\r\n\"x, y\",\"he said \"\"hi\"\"\"\r\nplain, z \r\n\r\n".as_bytes()).unwrap();
        assert_eq!(c.header, vec!["a", "b"]);
        assert_eq!(c.rows[0], vec!["x, y", "he said \"hi\""]);
        assert_eq!(c.rows[1], vec!["plain", "z"]);
        assert_eq!(c.rows.len(), 2);
    }

    #[test]
    fn a_line_break_inside_a_value_is_refused_not_typed_into_a_field() {
        assert!(parse(b"a\n\"x\ny\"\n").unwrap_err().contains("row 1"));
    }

    #[test]
    fn a_file_without_a_final_newline_keeps_its_last_row() {
        assert_eq!(parse(b"a,b\n1,2").unwrap().rows, vec![vec!["1", "2"]]);
    }

    #[test]
    fn refusals_name_the_row_never_the_value() {
        let e = parse(b"a,b\n1,2\nsecret-only\n").unwrap_err();
        assert!(e.contains("row 2") && !e.contains("secret"));
        assert!(parse(b"a,a\n1,2\n").is_err());
        assert!(parse(b"a,b\n").is_err());
        assert!(parse(b"").is_err());
        assert!(parse(b"a,b\n\"1,2\n").is_err());
        assert!(parse(b"a\n\x07\n").unwrap_err().contains("row 1"));
        assert!(parse(&[0xff, 0xfe]).is_err());
    }

    #[test]
    fn size_and_row_limits_hold() {
        let mut big = String::from("a\n");
        for _ in 0..(MAX_ROWS + 1) {
            big.push_str("x\n");
        }
        assert!(parse(big.as_bytes()).is_err());
        assert!(parse(&vec![b'a'; MAX_FILE_BYTES + 1]).is_err());
    }

    #[test]
    fn excel_files_are_refused_in_plain_words() {
        let e = read_file(std::path::Path::new("/nonexistent/x.xlsx")).unwrap_err();
        assert!(e.contains("CSV"));
    }

    #[test]
    fn mapping_builds_one_inputs_object_per_row_and_checks_every_value_first() {
        let c = parse(b"Name,Mail\nAnn Test,ann@example.test\nBo Test,bo@example.test\n").unwrap();
        let inputs = [spec("full_name"), spec("email")];
        let map: BTreeMap<_, _> = [("full_name".to_string(), "Name".to_string()), ("email".to_string(), "Mail".to_string())].into();
        let rows = build_rows(&c, &map, &inputs).unwrap();
        assert_eq!(rows[1], json!({"full_name": "Bo Test", "email": "bo@example.test"}));

        // an unmapped input, an unknown column, an extra mapping
        let mut m2 = map.clone();
        m2.remove("email");
        assert!(build_rows(&c, &m2, &inputs).is_err());
        m2.insert("email".into(), "Nope".into());
        assert!(build_rows(&c, &m2, &inputs).is_err());
        let mut m3 = map.clone();
        m3.insert("other".into(), "Name".into());
        assert!(build_rows(&c, &m3, &inputs).is_err());
    }

    #[test]
    fn bad_values_stop_the_run_before_it_starts_and_the_message_holds_no_value() {
        let c = parse(b"n,r\nAnn,Teacher\n,Teacher\nBo,Wizard\n").unwrap();
        let inputs = [spec("n"), InputSpec { name: "r".into(), choices: Some(vec!["Teacher".into()]) }];
        let map = suggest_mapping(&c.header, &inputs);
        let e = build_rows(&c, &map, &inputs).unwrap_err();
        assert!(e.contains("row 2 n is empty") && e.contains("row 3 r is not one of the choices"));
        assert!(!e.contains("Wizard") && !e.contains("Ann"));
    }

    #[test]
    fn the_fingerprint_changes_with_one_byte() {
        assert_ne!(fingerprint(b"a,b\n1,2\n"), fingerprint(b"a,b\n1,3\n"));
        assert_eq!(fingerprint(b"x").len(), 64);
    }

    #[test]
    fn task_inputs_read_names_and_choices() {
        let t = json!({"inputs":[{"name":"a","type":"text"},{"name":"r","type":"choice","choices":["x","y"]}]});
        let i = task_inputs(&t);
        assert_eq!(i[0], spec("a"));
        assert_eq!(i[1].choices, Some(vec!["x".to_string(), "y".to_string()]));
    }
}
