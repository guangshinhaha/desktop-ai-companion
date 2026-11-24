//! Review of a fresh recording (PRD R-4, R-5, R-6; TD 5.1 step 6).
//!
//! A `Draft` is a task/v1 document held in memory. The recorder's file holds descriptors and input
//! names only, so the one place a typed value can enter is a constant the officer types here. A
//! constant lives in the draft until save; discard drops the draft and deletes the recorder's file.
//! Nothing here writes a draft anywhere before save, and no error or log line carries a value.

use crate::store::{Store, StoreError};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

const MAX_CONSTANT_CHARS: usize = 500;

pub struct Draft {
    task: Value,
    /// The recorder's output file, deleted on save and on discard.
    source: Option<PathBuf>,
}

fn valid_name(s: &str) -> bool {
    let mut c = s.chars();
    matches!(c.next(), Some('a'..='z')) && c.all(|c| matches!(c, 'a'..='z' | '0'..='9' | '_'))
}

fn steps(task: &Value) -> &[Value] {
    task["steps"].as_array().map_or(&[], |v| v.as_slice())
}

/// Input names a step reads or writes (value, read target, expect checks). Summary text is not
/// a use: it only mentions inputs.
fn step_inputs(step: &Value) -> Vec<String> {
    let mut out = vec![];
    for v in [&step["value"]["input"], &step["into"]] {
        if let Some(s) = v.as_str() {
            out.push(s.to_string());
        }
    }
    for c in step["expect"].as_array().into_iter().flatten() {
        if let Some(s) = c["input"].as_str() {
            out.push(s.to_string());
        }
    }
    out
}

fn used_inputs(task: &Value) -> BTreeSet<String> {
    steps(task).iter().flat_map(step_inputs).collect()
}

fn summary_refs(summary: &str) -> Vec<String> {
    let mut out = vec![];
    let mut rest = summary;
    while let Some(i) = rest.find('{') {
        let after = &rest[i + 1..];
        match after.find('}') {
            Some(j) => {
                out.push(after[..j].to_string());
                rest = &after[j + 1..];
            }
            None => break,
        }
    }
    out
}

/// Takes `{name}` out of a summary and tidies the joiner around it.
fn remove_token(summary: &str, name: &str) -> String {
    let t = format!("{{{name}}}");
    for (pat, rep) in [
        (format!(", {t}"), ""),
        (format!(" with {t}, "), " with "),
        (format!(" with {t}"), ""),
        (t.clone(), ""),
    ] {
        if summary.contains(&pat) {
            return summary.replacen(&pat, rep, 1);
        }
    }
    summary.to_string()
}

fn no_braces(s: &str) -> String {
    s.replace('{', "(").replace('}', ")")
}

/// Structural problems that would make a task unsafe or unreplayable. Messages carry JSON paths,
/// never values. The schema (scripts/validate-task) is the full check; this is the part the
/// backend must hold even if the UI is bypassed.
pub fn check_task(task: &Value) -> Vec<String> {
    let mut e = vec![];
    if task["format"] != "task/v1" {
        e.push("/format: must be task/v1".into());
    }
    let declared: BTreeSet<&str> =
        task["inputs"].as_array().into_iter().flatten().filter_map(|i| i["name"].as_str()).collect();
    let ss = steps(task);
    if ss.is_empty() {
        e.push("/steps: a task needs at least one step".into());
    }
    let mut ids = BTreeSet::new();
    for (i, s) in ss.iter().enumerate() {
        let at = format!("/steps/{i}");
        if !s["id"].as_str().is_some_and(|id| ids.insert(id.to_string())) {
            e.push(format!("{at}/id: missing or duplicate"));
        }
        if !s["say"].as_str().is_some_and(|t| !t.is_empty()) {
            e.push(format!("{at}/say: missing"));
        }
        if !s["expect"].as_array().is_some_and(|c| !c.is_empty()) {
            e.push(format!("{at}/expect: every step needs a check"));
        }
        match s["consequential"].as_str() {
            Some("yes") | Some("unknown") if s["approval"].is_null() => {
                e.push(format!("{at}/approval: needed while consequential is yes or unknown"))
            }
            Some("yes") | Some("no") | Some("unknown") => {}
            _ => e.push(format!("{at}/consequential: must be yes, no or unknown")),
        }
        let v = &s["value"];
        if !v.is_null() {
            let by_input = v["input"].is_string() && v.as_object().is_some_and(|o| o.len() == 1);
            let by_constant = v["confirmed_no_personal_data"] == true
                && v["constant"].is_string()
                && v.as_object().is_some_and(|o| o.len() == 2);
            if !by_input && !by_constant {
                e.push(format!("{at}/value: must be an input or a confirmed constant"));
            }
        }
        for (j, c) in s["expect"].as_array().into_iter().flatten().enumerate() {
            if c.get("constant").is_some() && c["confirmed_no_personal_data"] != true {
                e.push(format!("{at}/expect/{j}: a constant needs the no-personal-data confirmation"));
            }
        }
        for n in step_inputs(s) {
            if !declared.contains(n.as_str()) {
                e.push(format!("{at}: uses an input that is not declared"));
            }
        }
        if let Some(sum) = s["approval"]["summary"].as_str() {
            for n in summary_refs(sum) {
                if !declared.contains(n.as_str()) {
                    e.push(format!("{at}/approval/summary: names an input that is not declared"));
                }
            }
        }
    }
    e
}

impl Draft {
    /// Parses recorder output. Refuses anything that is not a task/v1 document.
    pub fn from_json(text: &str, source: Option<PathBuf>) -> Result<Draft, String> {
        let task: Value = serde_json::from_str(text).map_err(|_| "not a recorded task (bad JSON)".to_string())?;
        if task["format"] != "task/v1" || !task["steps"].is_array() || !task["inputs"].is_array() {
            return Err("not a recorded task (task/v1)".into());
        }
        Ok(Draft { task, source })
    }

    pub fn from_file(path: &Path) -> Result<Draft, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read the recording: {}", e.kind()))?;
        Draft::from_json(&text, Some(path.to_path_buf()))
    }

    /// What the review screen shows: plain-language steps, inputs, what is unknown, what commits.
    pub fn view(&self) -> Value {
        let t = &self.task;
        let mut used_by: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for s in steps(t) {
            for n in step_inputs(s) {
                let id = s["id"].as_str().unwrap_or("").to_string();
                let list = used_by.entry(n).or_default();
                if !list.contains(&id) {
                    list.push(id);
                }
            }
        }
        let rows: Vec<Value> = steps(t)
            .iter()
            .map(|s| {
                let c = s["consequential"].as_str().unwrap_or("unknown");
                let constant = s["value"]["constant"].as_str();
                json!({
                    "id": s["id"],
                    "say": s["say"],
                    "action": s["action"],
                    "consequential": c,
                    "flag": match c { "yes" => "consequential", "no" => "none", _ => "unknown" },
                    "input": s["value"]["input"],
                    "is_constant": constant.is_some(),
                    "constant": constant,
                    "can_be_constant": constant.is_none()
                        && matches!(s["action"].as_str(), Some("set_value" | "select" | "type_keys"))
                        && s["value"]["input"].is_string(),
                })
            })
            .collect();
        let inputs: Vec<Value> = t["inputs"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|i| {
                let n = i["name"].as_str().unwrap_or("");
                json!({ "name": n, "type": i["type"], "source": i["source"],
                        "steps": used_by.get(n).cloned().unwrap_or_default() })
            })
            .collect();
        let count = |c: &str| rows.iter().filter(|r| r["consequential"] == c).count();
        let preconditions: Vec<String> = t["preconditions"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|p| {
                let who = p["check"]["target"]["label"].as_str().or(p["check"]["matches"].as_str()).unwrap_or("something");
                format!("{}: {} {}", p["app"].as_str().unwrap_or("app"), p["check"]["kind"].as_str().unwrap_or("check"), who)
            })
            .collect();
        json!({
            "id": t["id"], "name": t["name"], "author": t["author"],
            "apps": t["apps"], "preconditions": preconditions,
            "inputs": inputs, "steps": rows,
            "unknown_count": count("unknown"), "consequential_count": count("yes"),
            "problems": check_task(t),
        })
    }

    /// Applies one officer edit. `edit` is `{op: ..., ...}`. Unknown ops are refused.
    pub fn apply(&mut self, edit: &Value) -> Result<(), String> {
        let text = |k: &str| edit[k].as_str().ok_or_else(|| format!("{k} is required"));
        match edit["op"].as_str().ok_or("op is required")? {
            "rename_input" => self.rename_input(text("from")?, text("to")?),
            "delete_step" => self.delete_step(text("step")?),
            "set_consequential" => self.set_consequential(text("step")?, text("value")?),
            "mark_constant" => {
                self.mark_constant(text("step")?, text("value")?, edit["confirmed_no_personal_data"] == true)
            }
            "rename_task" => {
                let n = text("name")?.trim();
                if n.is_empty() {
                    return Err("a task needs a name".into());
                }
                self.task["name"] = json!(n);
                Ok(())
            }
            _ => Err("unknown edit".into()),
        }
    }

    fn step_index(&self, id: &str) -> Result<usize, String> {
        steps(&self.task).iter().position(|s| s["id"] == id).ok_or_else(|| "no such step".to_string())
    }

    fn rename_input(&mut self, from: &str, to: &str) -> Result<(), String> {
        if !valid_name(to) {
            return Err("input names use lower-case letters, digits and underscores, starting with a letter".into());
        }
        let declared = |n: &str| self.task["inputs"].as_array().into_iter().flatten().any(|i| i["name"] == n);
        if !declared(from) {
            return Err("no such input".into());
        }
        if from == to {
            return Ok(());
        }
        if declared(to) {
            return Err("another input already has that name".into());
        }
        let swap = |v: &mut Value, k: &str| {
            if v[k] == from {
                v[k] = json!(to);
            }
        };
        for i in self.task["inputs"].as_array_mut().into_iter().flatten() {
            swap(i, "name");
        }
        for s in self.task["steps"].as_array_mut().into_iter().flatten() {
            if s["value"]["input"] == from {
                s["value"]["input"] = json!(to);
            }
            swap(s, "into");
            for c in s["expect"].as_array_mut().into_iter().flatten() {
                swap(c, "input");
            }
            if let Some(sum) = s["approval"]["summary"].as_str() {
                s["approval"]["summary"] = json!(sum.replace(&format!("{{{from}}}"), &format!("{{{to}}}")));
            }
        }
        Ok(())
    }

    /// Drops inputs that a step used before an edit and none uses now, and fixes the approval
    /// summaries that named them (`replacement` for a constant, else the token is removed).
    fn settle(&mut self, before: &BTreeSet<String>, replacement: &BTreeMap<String, String>) {
        let after = used_inputs(&self.task);
        let gone: Vec<String> = before.difference(&after).cloned().collect();
        if gone.is_empty() {
            return;
        }
        if let Some(inputs) = self.task["inputs"].as_array_mut() {
            inputs.retain(|i| !i["name"].as_str().is_some_and(|n| gone.iter().any(|g| g == n)));
        }
        for s in self.task["steps"].as_array_mut().into_iter().flatten() {
            let Some(mut sum) = s["approval"]["summary"].as_str().map(str::to_string) else { continue };
            for g in &gone {
                sum = match replacement.get(g) {
                    Some(r) => sum.replace(&format!("{{{g}}}"), &no_braces(r)),
                    None => remove_token(&sum, g),
                };
            }
            s["approval"]["summary"] = json!(sum.trim());
        }
    }

    fn delete_step(&mut self, id: &str) -> Result<(), String> {
        let i = self.step_index(id)?;
        if steps(&self.task).len() == 1 {
            return Err("a task needs at least one step; discard the recording instead".into());
        }
        let before = used_inputs(&self.task);
        self.task["steps"].as_array_mut().unwrap().remove(i);
        self.settle(&before, &BTreeMap::new());
        Ok(())
    }

    fn set_consequential(&mut self, id: &str, value: &str) -> Result<(), String> {
        // The officer decides yes or no. "unknown" is only what the recorder could not tell.
        if value != "yes" && value != "no" {
            return Err("a step is marked yes or no".into());
        }
        let i = self.step_index(id)?;
        let s = &mut self.task["steps"][i];
        s["consequential"] = json!(value);
        if value == "no" {
            s.as_object_mut().unwrap().remove("approval");
        } else if s["approval"].is_null() {
            let say = no_braces(s["say"].as_str().unwrap_or("This step"));
            s["approval"] = json!({ "summary": say, "undoable": false });
        }
        Ok(())
    }

    /// The backend gate for constants: refuses unless the officer confirmed the value holds no
    /// personal data. The UI also asks, but this is the check that counts.
    fn mark_constant(&mut self, id: &str, value: &str, confirmed: bool) -> Result<(), String> {
        if !confirmed {
            return Err("confirm that the value contains no personal data first".into());
        }
        if value.is_empty() || value.chars().count() > MAX_CONSTANT_CHARS {
            return Err("a constant is between 1 and 500 characters".into());
        }
        let i = self.step_index(id)?;
        let s = &self.task["steps"][i];
        if !matches!(s["action"].as_str(), Some("set_value" | "select" | "type_keys")) {
            return Err("only a typed or chosen value can be a constant".into());
        }
        let Some(name) = s["value"]["input"].as_str().map(str::to_string) else {
            return Err("this step's value is already a constant".into());
        };
        let before = used_inputs(&self.task);
        let s = &mut self.task["steps"][i];
        s["value"] = json!({ "constant": value, "confirmed_no_personal_data": true });
        for c in s["expect"].as_array_mut().into_iter().flatten() {
            if c["kind"] == "value_equals" && c["input"] == name.as_str() {
                *c = json!({ "kind": "value_equals", "constant": value, "confirmed_no_personal_data": true });
            }
        }
        let mut rep = BTreeMap::new();
        rep.insert(name, value.to_string());
        self.settle(&before, &rep);
        Ok(())
    }

    /// Writes the task as the next version in the store, with author and date, and the
    /// `task_saved` audit record. Refuses a task that fails `check_task`. On success the
    /// recorder's file is deleted. Returns (task id, version).
    pub fn save(self, store: &mut Store, author: &str, now: i64) -> Result<(String, i64), String> {
        let author = author.trim();
        if author.is_empty() {
            return Err("an author is needed to save".into());
        }
        let problems = check_task(&self.task);
        if !problems.is_empty() {
            return Err(format!("the task is not ready to save: {}", problems.join("; ")));
        }
        let id = self.task["id"].as_str().ok_or("the task has no id")?.to_string();
        let name = self.task["name"].as_str().filter(|n| !n.is_empty()).ok_or("a task needs a name")?.to_string();
        let mut task = self.task.clone();
        let created = iso_utc(now);
        let version = store
            .save_task_audited(&id, &name, author, now, |v| {
                let o: &mut Map<String, Value> = task.as_object_mut().unwrap();
                o.insert("version".into(), json!(v));
                o.insert("author".into(), json!(author));
                o.insert("created".into(), json!(created));
                serde_json::to_string_pretty(&task).map_err(|e| StoreError::Db(e.to_string()))
            })
            .map_err(|e| e.to_string())?;
        self.remove_source();
        Ok((id, version))
    }

    /// Drops the draft. The recorder's file goes too: the recording was refused.
    pub fn discard(self) {
        self.remove_source();
    }

    fn remove_source(&self) {
        if let Some(p) = &self.source {
            let _ = std::fs::remove_file(p);
        }
    }

    /// A copy kept while a save is attempted, so a failure (store locked, task not ready) does
    /// not cost the officer their edits. Memory only.
    pub fn clone_for_retry(&self) -> Draft {
        Draft { task: self.task.clone(), source: self.source.clone() }
    }

    pub fn source(&self) -> Option<&Path> {
        self.source.as_deref()
    }
}

/// `2026-11-24T10:14:00Z` from epoch seconds (UTC; no date library in the shell).
pub fn iso_utc(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Civil-from-days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}
