//! Job-graph checks (ADR-0006 step 5): `needs:` referencing undefined jobs, and cycles in
//! the `needs` dependency graph.
//!
//! Operates on the parsed workflow JSON. Diagnostics are anchored at the job node via its
//! JSON pointer (`/jobs/<id>`), resolved through the span tree by the caller-provided lookup.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

/// A graph finding: a message plus the JSON pointer it should anchor at.
#[derive(Debug, Clone, PartialEq)]
pub struct GraphFinding {
    pub pointer: String,
    pub message: String,
}

/// Check the `jobs` graph of a workflow. Returns findings (empty = clean).
pub fn check(workflow: &Value) -> Vec<GraphFinding> {
    let mut findings = Vec::new();
    let Some(jobs) = workflow.get("jobs").and_then(|j| j.as_object()) else {
        return findings; // no jobs (or malformed) — structural layer handles that
    };

    let job_ids: BTreeSet<&str> = jobs.keys().map(|k| k.as_str()).collect();

    // Build the needs map (job -> its declared needs), reporting undefined references. The
    // map owns the dep strings so it outlives the per-job temporaries.
    let mut needs: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for (id, job) in jobs {
        let deps = extract_needs(job);
        for dep in &deps {
            if !job_ids.contains(dep.as_str()) {
                findings.push(GraphFinding {
                    pointer: format!("/jobs/{}", escape(id)),
                    message: format!("job `{id}` needs `{dep}`, which is not a defined job"),
                });
            }
        }
        needs.insert(id.as_str(), deps);
    }

    // Detect cycles over the (defined-job) edges via DFS with a recursion stack.
    detect_cycles(&needs, &job_ids, &mut findings);

    findings
}

/// Extract a job's `needs` as a list of job ids. `needs` may be a single string or an array
/// of strings; other shapes are left to the structural layer.
fn extract_needs(job: &Value) -> Vec<String> {
    match job.get("needs") {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect(),
        _ => Vec::new(),
    }
}

/// DFS cycle detection. Reports one finding per job that participates in a back-edge, naming
/// the cycle path. Only edges to defined jobs are followed (undefined ones are already
/// reported above and can't form a real cycle).
fn detect_cycles<'a>(
    needs: &'a BTreeMap<&'a str, Vec<String>>,
    defined: &BTreeSet<&'a str>,
    findings: &mut Vec<GraphFinding>,
) {
    #[derive(Clone, Copy, PartialEq)]
    enum State {
        Visiting,
        Done,
    }
    let mut state: BTreeMap<&str, State> = BTreeMap::new();
    let mut reported: BTreeSet<&str> = BTreeSet::new();

    // Iterate job ids in sorted order for deterministic output.
    let mut ids: Vec<&str> = needs.keys().copied().collect();
    ids.sort_unstable();

    for &start in &ids {
        if state.get(start).is_some() {
            continue;
        }
        // Iterative DFS carrying the current path.
        let mut stack: Vec<(&str, usize)> = vec![(start, 0)];
        let mut path: Vec<&str> = vec![start];
        state.insert(start, State::Visiting);

        while let Some(&mut (node, ref mut idx)) = stack.last_mut() {
            let edges: &[String] = needs.get(node).map(|v| v.as_slice()).unwrap_or(&[]);
            let mut advanced = false;
            while *idx < edges.len() {
                let next: &str = edges[*idx].as_str();
                *idx += 1;
                if !defined.contains(next) {
                    continue; // undefined edge — already reported
                }
                match state.get(next) {
                    Some(State::Visiting) => {
                        // Back-edge → cycle. Report the cycle path once per cycle.
                        report_cycle(&path, next, findings, &mut reported);
                    }
                    Some(State::Done) => {}
                    None => {
                        // Resolve `next` to the map's own key so lifetimes match `path`.
                        if let Some((&key, _)) = needs.get_key_value(next) {
                            state.insert(key, State::Visiting);
                            stack.push((key, 0));
                            path.push(key);
                            advanced = true;
                            break;
                        }
                    }
                }
            }
            if !advanced {
                // Node exhausted — pop.
                if let Some((done, _)) = stack.pop() {
                    state.insert(done, State::Done);
                    path.pop();
                }
            }
        }
    }
}

fn report_cycle<'a>(
    path: &[&'a str],
    back_to: &'a str,
    findings: &mut Vec<GraphFinding>,
    reported: &mut BTreeSet<&'a str>,
) {
    // The cycle is path[pos..] + back_to, where path[pos] == back_to.
    let Some(pos) = path.iter().position(|&n| n == back_to) else {
        return;
    };
    let cycle: Vec<&str> = path[pos..].to_vec();
    // Anchor on the lexicographically-first member to keep it stable, and report once.
    let mut members = cycle.clone();
    members.sort_unstable();
    let anchor = members[0];
    if !reported.insert(anchor) {
        return;
    }
    let mut chain = cycle.clone();
    chain.push(back_to); // close the loop for display
    findings.push(GraphFinding {
        pointer: format!("/jobs/{}", escape(anchor)),
        message: format!("`needs` cycle detected: {}", chain.join(" -> ")),
    });
}

fn escape(token: &str) -> String {
    token.replace('~', "~0").replace('/', "~1")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn messages(wf: Value) -> Vec<String> {
        let mut m: Vec<String> = check(&wf).into_iter().map(|f| f.message).collect();
        m.sort();
        m
    }

    #[test]
    fn valid_needs_graph_is_clean() {
        let wf = json!({
            "jobs": {
                "build": { "runs-on": "x" },
                "test": { "runs-on": "x", "needs": "build" },
                "deploy": { "runs-on": "x", "needs": ["build", "test"] }
            }
        });
        assert!(messages(wf).is_empty());
    }

    #[test]
    fn undefined_need_is_flagged() {
        let wf = json!({
            "jobs": {
                "test": { "runs-on": "x", "needs": "biuld" }
            }
        });
        let m = messages(wf);
        assert_eq!(m.len(), 1, "{m:?}");
        assert!(m[0].contains("needs `biuld`, which is not a defined job"));
    }

    #[test]
    fn undefined_need_in_array_is_flagged() {
        let wf = json!({
            "jobs": {
                "a": { "runs-on": "x" },
                "b": { "runs-on": "x", "needs": ["a", "ghost"] }
            }
        });
        let m = messages(wf);
        assert_eq!(m.len(), 1);
        assert!(m[0].contains("ghost"));
    }

    #[test]
    fn direct_cycle_is_flagged() {
        let wf = json!({
            "jobs": {
                "a": { "runs-on": "x", "needs": "b" },
                "b": { "runs-on": "x", "needs": "a" }
            }
        });
        let m = messages(wf);
        assert_eq!(m.len(), 1, "{m:?}");
        assert!(m[0].contains("cycle detected"), "{m:?}");
    }

    #[test]
    fn self_cycle_is_flagged() {
        let wf = json!({
            "jobs": { "a": { "runs-on": "x", "needs": "a" } }
        });
        let m = messages(wf);
        assert_eq!(m.len(), 1);
        assert!(m[0].contains("cycle"));
    }

    #[test]
    fn longer_cycle_is_flagged_once() {
        let wf = json!({
            "jobs": {
                "a": { "runs-on": "x", "needs": "c" },
                "b": { "runs-on": "x", "needs": "a" },
                "c": { "runs-on": "x", "needs": "b" }
            }
        });
        let m = messages(wf);
        assert_eq!(m.len(), 1, "one finding for the whole cycle: {m:?}");
        assert!(m[0].contains("cycle detected"));
    }

    #[test]
    fn diamond_is_not_a_cycle() {
        // a -> b, a -> c, b -> d, c -> d  (DAG, shared dependency)
        let wf = json!({
            "jobs": {
                "d": { "runs-on": "x" },
                "b": { "runs-on": "x", "needs": "d" },
                "c": { "runs-on": "x", "needs": "d" },
                "a": { "runs-on": "x", "needs": ["b", "c"] }
            }
        });
        assert!(messages(wf).is_empty(), "diamond DAG is valid");
    }

    #[test]
    fn no_jobs_is_clean() {
        assert!(messages(json!({})).is_empty());
        assert!(messages(json!({ "jobs": "not-a-map" })).is_empty());
    }

    #[test]
    fn needs_wrong_shape_is_left_to_structural_layer() {
        let wf = json!({ "jobs": { "a": { "runs-on": "x", "needs": 42 } } });
        assert!(messages(wf).is_empty());
    }
}
