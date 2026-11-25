//! What the canvas needs to route a request (#32, TD 8): the saved tasks, and the inputs a task
//! needs from the officer. Routing itself is deterministic and has no model call. This module reads
//! the local store only; it sends nothing anywhere and holds no input value.

use crate::store::Store;
use crate::verification;
use serde_json::{json, Value};

/// The saved tasks (newest version of each) as the canvas needs them: id, version, name, and the
/// inputs the officer must supply (`source: ask`). `column` and `read` inputs are filled by the run.
pub fn catalog(store: &Store) -> Result<Vec<Value>, String> {
    catalog_with(store, &crate::reuse::LocalReuse(store))
}

/// As [`catalog`], with where the reuse counts come from (#43): this device now, the shared library later.
pub fn catalog_with(store: &Store, reuse: &dyn crate::reuse::ReuseBackend) -> Result<Vec<Value>, String> {
    let mut out = Vec::new();
    for (id, latest, latest_body) in store.latest_tasks().map_err(|e| e.to_string())? {
        // The version offered is not always the newest: after a flag the older one stays until a newer one passes (#42).
        let st = verification::status(store, &id)?;
        let version = st.as_ref().map_or(latest, |s| s.default_version);
        let body = if version == latest {
            latest_body
        } else {
            store.task_body(&id, version).map_err(|e| e.to_string())?.unwrap_or_default()
        };
        let Ok(task) = serde_json::from_str::<Value>(&body) else { continue };
        let Some(name) = task["name"].as_str() else { continue };
        let mut entry = json!({ "id": id, "version": version, "name": name, "inputs": ask_inputs(&task) });
        if let Some(s) = &st {
            entry["status"] = json!(s.standing.as_str());
            if s.standing == verification::Standing::NeedsVerification {
                entry["flagged_step"] = json!(s.flagged_step);
            }
            // A newer recording that has not passed: offered on its own, with its own inputs, to be verified.
            // The author's name is not in the catalog (the canvas does not need it).
            if let Some(cv) = s.candidate_version {
                let cb = store.task_body(&id, cv).map_err(|e| e.to_string())?;
                if let Some(Ok(ct)) = cb.map(|b| serde_json::from_str::<Value>(&b)) {
                    entry["candidate"] = json!({ "version": cv, "inputs": ask_inputs(&ct) });
                }
            }
        }
        let r = reuse.reuse(&id)?;
        entry["reuse"] = json!({ "replays": r.replays, "last_success_at": r.last_success_at, "scope": r.scope });
        out.push(entry);
    }
    Ok(out)
}

fn ask_inputs(task: &Value) -> Vec<Value> {
    task["inputs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|i| i["source"] == "ask")
        .filter_map(|i| {
            let name = i["name"].as_str()?;
            let mut v = json!({ "name": name, "type": i["type"].as_str().unwrap_or("text") });
            if let Some(c) = i.get("choices") {
                v["choices"] = c.clone();
            }
            Some(v)
        })
        .collect()
}

/// The `ask` inputs a run is missing: not given, not a string, or blank. A start with any of these is
/// refused. Nothing is defaulted, trimmed into place or guessed. Returns the names only, never values.
pub fn missing_inputs(task: &Value, given: &Value) -> Vec<String> {
    ask_inputs(task)
        .iter()
        .filter_map(|i| i["name"].as_str())
        .filter(|n| !given.get(*n).and_then(Value::as_str).is_some_and(|s| !s.trim().is_empty()))
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task() -> Value {
        json!({ "name": "Add test account", "inputs": [
            { "name": "full_name", "type": "text", "source": "ask" },
            { "name": "role", "type": "choice", "choices": ["Teacher", "Admin"], "source": "ask" },
            { "name": "row_id", "type": "text", "source": "column" } ] })
    }

    #[test]
    fn only_ask_inputs_are_for_the_officer() {
        let names: Vec<_> = ask_inputs(&task()).iter().map(|i| i["name"].as_str().unwrap().to_string()).collect();
        assert_eq!(names, ["full_name", "role"]);
        assert_eq!(ask_inputs(&task())[1]["choices"], json!(["Teacher", "Admin"]));
    }

    #[test]
    fn missing_means_absent_blank_or_not_text() {
        assert_eq!(missing_inputs(&task(), &json!({})), ["full_name", "role"]);
        assert_eq!(missing_inputs(&task(), &json!({ "full_name": "  ", "role": "Admin" })), ["full_name"]);
        assert_eq!(missing_inputs(&task(), &json!({ "full_name": 5, "role": null })), ["full_name", "role"]);
        assert!(missing_inputs(&task(), &json!({ "full_name": "Test Person", "role": "Admin" })).is_empty());
    }
}
