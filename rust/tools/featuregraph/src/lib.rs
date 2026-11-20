mod model;
mod page;
mod yaml;

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
};

pub use model::{
    Evidence, Feature, FeatureView, Graph, LevelResult, Limit, ScenarioRef, ScenarioView,
};
pub use page::render;
pub use yaml::{Head, parse_head};

#[derive(Debug)]
pub struct Checker {
    pub root: PathBuf,
    pub graph: Graph,
    pub problems: Vec<String>,
}

#[derive(serde::Deserialize)]
struct TestSelection {
    scenarios: Vec<SelectionEntry>,
}

#[derive(serde::Deserialize)]
struct SelectionEntry {
    id: String,
    path: String,
    levels: Vec<String>,
}

impl Checker {
    pub fn load(root: &Path) -> Result<Self, String> {
        let raw = fs::read_to_string(root.join("features.yaml")).map_err(|e| e.to_string())?;
        let graph = yaml::parse_graph(&raw).map_err(|e| format!("features.yaml: {e}"))?;
        Ok(Self::new(root, graph))
    }

    #[must_use]
    pub fn new(root: &Path, graph: Graph) -> Self {
        Self {
            root: root.to_path_buf(),
            graph,
            problems: Vec::new(),
        }
    }

    fn fail(&mut self, message: String) {
        self.problems.push(message);
    }

    fn read_lab_json<T: serde::de::DeserializeOwned>(&mut self, name: &str) -> Option<T> {
        let path = self.root.join("lab").join(name);
        let result = match fs::read(&path) {
            Ok(raw) => serde_json::from_slice(&raw).map_err(|e| e.to_string()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
            Err(e) => Err(e.to_string()),
        };
        match result {
            Ok(value) => Some(value),
            Err(e) => {
                self.fail(format!("lab/{name}: {e}"));
                None
            }
        }
    }

    #[allow(clippy::too_many_lines)]
    pub fn check(&mut self) -> Vec<FeatureView> {
        let selection = self.read_lab_json::<TestSelection>("test-selection.json");
        let sim_selection = selection.map(|selection| {
            selection
                .scenarios
                .into_iter()
                .filter(|entry| entry.levels.iter().any(|level| level == "sim"))
                .map(|entry| (entry.id, entry.path))
                .collect::<BTreeSet<_>>()
        });
        let scenario_files = self.index_scenario_files(sim_selection.as_ref());
        let no_runner: BTreeMap<String, String> =
            self.read_lab_json("no-runner.json").unwrap_or_default();
        let mut ids = BTreeSet::new();
        let mut owner: BTreeMap<String, String> = BTreeMap::new();
        let statuses: BTreeSet<String> = self.graph.statuses.iter().cloned().collect();
        let features = self.graph.features.clone();
        for f in &features {
            if !ids.insert(f.id.clone()) {
                self.fail(format!("feature {} defined twice", f.id));
            }
        }
        let mut views = Vec::new();
        for f in &features {
            let mut v = FeatureView {
                feature: f.clone(),
                views: Vec::new(),
            };
            if !statuses.contains(&f.status) {
                self.fail(format!("{}: unknown status {}", f.id, go_quote(&f.status)));
            }
            if f.status == "dropped" && f.notes.trim().is_empty() {
                self.fail(format!("{}: dropped feature has no reason", f.id));
            }
            if f.scenarios.is_empty() && f.status != "dropped" {
                self.fail(format!("{}: no scenarios", f.id));
            }
            if f.requirements.is_empty() {
                self.fail(format!("{}: no requirements", f.id));
            }
            for r in &f.requirements {
                if !self.graph.requirements.contains_key(r) {
                    self.fail(format!("{}: unknown requirement {r}", f.id));
                }
            }
            for d in &f.depends_on {
                if !ids.contains(d) {
                    self.fail(format!("{}: depends on unknown feature {d}", f.id));
                }
            }
            for s in &f.scenarios {
                if !s.status.is_empty() && s.status != "dropped" {
                    self.fail(format!(
                        "{}: scenario {} has unknown status {}",
                        f.id,
                        s.id,
                        go_quote(&s.status)
                    ));
                }
                if s.status == "dropped" && s.reason.trim().is_empty() {
                    self.fail(format!("{}: dropped scenario {} has no reason", f.id, s.id));
                }
                if let Some(prev) = owner.get(&s.id) {
                    self.fail(format!(
                        "scenario {} owned by both {prev} and {}",
                        s.id, f.id
                    ));
                }
                owner.insert(s.id.clone(), f.id.clone());
                let file = scenario_files.get(&s.id).cloned().unwrap_or_default();
                let mut sv = ScenarioView {
                    id: s.id.clone(),
                    file,
                    status: s.status.clone(),
                    reason: s.reason.clone(),
                    results: Vec::new(),
                };
                let unavailable = no_runner.get(&s.id);
                if sv.reason.is_empty()
                    && let Some(reason) = unavailable
                {
                    sv.reason.clone_from(reason);
                }
                if sv.file.is_empty() && f.status != "todo" {
                    self.fail(format!(
                        "{}: scenario {} has no file under lab/scenarios",
                        f.id, s.id
                    ));
                }
                for l in &s.levels {
                    if !self.graph.levels.contains_key(l) {
                        self.fail(format!("{}: scenario {} has unknown level {l}", f.id, s.id));
                    }
                    let state = if s.status == "dropped" || f.status == "dropped" {
                        "dropped".to_owned()
                    } else if unavailable.is_some() {
                        "no-runner".to_owned()
                    } else {
                        self.evidence_state(&s.id, l)
                    };
                    sv.results.push(LevelResult {
                        level: l.clone(),
                        state,
                    });
                }
                v.views.push(sv);
            }
            let mut limited = BTreeSet::new();
            for l in &f.limits {
                if l.reason.is_empty() {
                    self.fail(format!(
                        "{}: limit on {}/{} has no reason",
                        f.id, l.scenario, l.level
                    ));
                }
                limited.insert(format!("{}/{}", l.scenario, l.level));
            }
            if f.status == "limited" && f.limits.is_empty() {
                self.fail(format!("{} is limited but lists no limits", f.id));
            }
            if f.status == "done" || f.status == "limited" {
                for sv in &v.views {
                    for r in &sv.results {
                        let excused = f.status == "limited"
                            && limited.contains(&format!("{}/{}", sv.id, r.level));
                        if r.state != "passed" && r.state != "dropped" && !excused {
                            self.fail(format!(
                                "{} is {} but {} at {} is {}",
                                f.id, f.status, sv.id, r.level, r.state
                            ));
                        }
                    }
                }
            }
            views.push(v);
        }
        for (id, file) in &scenario_files {
            if !owner.contains_key(id) {
                self.fail(format!(
                    "scenario file {file} ({id}) is not linked to any feature"
                ));
            }
        }
        self.check_acyclic();
        views
    }

    fn index_scenario_files(
        &mut self,
        sim_selection: Option<&BTreeSet<(String, String)>>,
    ) -> BTreeMap<String, String> {
        let mut files: BTreeMap<String, String> = BTreeMap::new();
        let mut features: BTreeMap<String, String> = BTreeMap::new();
        let mut levels: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut paths = Vec::new();
        collect_yaml(&self.root.join("lab").join("scenarios"), &mut paths);
        for path in paths {
            let Ok(raw) = fs::read_to_string(&path) else {
                continue;
            };
            let head = parse_head(&raw).unwrap_or_default();
            let id = head.id;
            if id.is_empty() {
                self.fail(format!("scenario file {} has no id", display(&path)));
                continue;
            }
            let rel = display(path.strip_prefix(&self.root).unwrap_or(&path));
            if let Some(selection) = sim_selection
                && head.levels.iter().any(|level| level == "sim")
                && !selection.contains(&(id.clone(), rel.clone()))
            {
                self.fail(format!(
                    "sim scenario {id} ({rel}) missing from lab/test-selection.json"
                ));
            }
            let feature = head.feature;
            let head_levels = head.levels;
            if let Some(prev) = files.get(&id).cloned() {
                if features.get(&id) != Some(&feature) || head_levels.is_empty() {
                    self.fail(format!(
                        "scenario {id} has incompatible definitions in {prev} and {rel}"
                    ));
                }
                for level in &head_levels {
                    if levels.get(&id).is_some_and(|set| set.contains(level)) {
                        self.fail(format!(
                            "scenario {id} at {level} defined in {prev} and {rel}"
                        ));
                    }
                }
                files.insert(id.clone(), format!("{prev}, {rel}"));
            } else {
                files.insert(id.clone(), rel);
                features.insert(id.clone(), feature);
                levels.insert(id.clone(), BTreeSet::new());
            }
            if let Some(set) = levels.get_mut(&id) {
                set.extend(head_levels);
            }
        }
        files
    }

    fn evidence_state(&self, id: &str, level: &str) -> String {
        let path = self
            .root
            .join("lab")
            .join("evidence")
            .join(level)
            .join(format!("{id}.json"));
        let Ok(raw) = fs::read(path) else {
            return "missing".to_owned();
        };
        match serde_json::from_slice::<Evidence>(&raw) {
            Err(_) => "corrupt",
            Ok(e) if e.passed == Some(true) => "passed",
            Ok(_) => "failed",
        }
        .to_owned()
    }

    fn check_acyclic(&mut self) {
        let mut deps: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for f in &self.graph.features {
            deps.insert(f.id.clone(), f.depends_on.clone());
        }
        let mut state: BTreeMap<String, u8> = BTreeMap::new();
        let mut problems = Vec::new();
        let keys: Vec<String> = deps.keys().cloned().collect();
        for key in keys {
            visit(&key, &mut Vec::new(), &deps, &mut state, &mut problems);
        }
        self.problems.extend(problems);
    }
}

fn visit(
    id: &str,
    path: &mut Vec<String>,
    deps: &BTreeMap<String, Vec<String>>,
    state: &mut BTreeMap<String, u8>,
    problems: &mut Vec<String>,
) {
    match state.get(id).copied().unwrap_or(0) {
        1 => {
            let mut chain = path.clone();
            chain.push(id.to_owned());
            problems.push(format!("dependency cycle: {}", chain.join(" -> ")));
            return;
        }
        2 => return,
        _ => {}
    }
    state.insert(id.to_owned(), 1);
    if let Some(list) = deps.get(id) {
        for d in list {
            path.push(id.to_owned());
            visit(d, path, deps, state, problems);
            path.pop();
        }
    }
    state.insert(id.to_owned(), 2);
}

fn collect_yaml(directory: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            collect_yaml(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "yaml") {
            out.push(path);
        }
    }
}

fn display(path: &Path) -> String {
    let text = path.to_string_lossy();
    text.strip_prefix("./").unwrap_or(&text).to_owned()
}

fn go_quote(value: &str) -> String {
    let mut out = String::from("\"");
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[must_use]
pub fn summarize(views: &[FeatureView]) -> String {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    let (mut passed, mut total, mut no_runner) = (0, 0, 0);
    for v in views {
        *counts.entry(v.feature.status.as_str()).or_default() += 1;
        for s in &v.views {
            for r in &s.results {
                total += 1;
                match r.state.as_str() {
                    "passed" => passed += 1,
                    "no-runner" => no_runner += 1,
                    _ => {}
                }
            }
        }
    }
    let count = |name: &str| counts.get(name).copied().unwrap_or(0);
    format!(
        "features: {} (done {}, limited {}, in_progress {}, todo {}, blocked {}, dropped {}); scenario-levels passing {passed}/{total}, no-runner {no_runner}",
        views.len(),
        count("done"),
        count("limited"),
        count("in_progress"),
        count("todo"),
        count("blocked"),
        count("dropped"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_runner_does_not_credit_canonical_evidence() {
        let root = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let write = |name: &str, contents: &str| {
            let path = root.path().join(name);
            let parent = path.parent().unwrap_or_else(|| panic!("missing parent"));
            fs::create_dir_all(parent).unwrap_or_else(|e| panic!("{e}"));
            fs::write(path, contents).unwrap_or_else(|e| panic!("{e}"));
        };
        write(
            "features.yaml",
            "statuses: [done]\nlevels: {sim: simulation}\nrequirements: {R: behavior}\nfeatures:\n  - id: example\n    status: done\n    requirements: [R]\n    scenarios: [{id: EXAMPLE, levels: [sim]}]\n",
        );
        write(
            "lab/scenarios/EXAMPLE.yaml",
            "id: EXAMPLE\nfeature: example\nlevels: [sim]\n",
        );
        write("lab/evidence/sim/EXAMPLE.json", r#"{"passed":true}"#);
        let mut checker = Checker::load(root.path()).unwrap_or_else(|e| panic!("{e}"));
        let views = checker.check();
        assert_eq!(views[0].views[0].results[0].state, "passed");
        assert_eq!(checker.problems, [] as [String; 0]);

        write("lab/no-runner.json", r#"{"EXAMPLE":"runner removed"}"#);
        let views = checker.check();
        let scenario = &views[0].views[0];
        assert_eq!(scenario.results[0].state, "no-runner");
        assert_eq!(scenario.reason, "runner removed");
        assert_eq!(
            checker.problems,
            ["example is done but EXAMPLE at sim is no-runner"]
        );
        assert!(summarize(&views).contains("passing 0/1, no-runner 1"));
    }
}
