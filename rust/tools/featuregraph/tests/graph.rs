use ab_featuregraph::{Checker, Feature, Graph, ScenarioRef, parse_head};
use std::{collections::BTreeMap, fs, path::Path};

fn repository() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../.."))
}

#[test]
fn repository_graph_loads_and_checks() {
    let mut checker = Checker::load(repository()).unwrap_or_else(|e| panic!("{e}"));
    let views = checker.check();
    assert!(!views.is_empty());
}

#[test]
fn web_scenario_levels_match_graph() {
    let checker = Checker::load(repository()).unwrap_or_else(|e| panic!("{e}"));
    let mut levels: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for feature in &checker.graph.features {
        for scenario in &feature.scenarios {
            levels.insert(scenario.id.clone(), scenario.levels.clone());
        }
    }
    let directory = repository().join("lab/scenarios/web");
    for entry in fs::read_dir(directory).unwrap_or_else(|e| panic!("{e}")) {
        let path = entry.unwrap_or_else(|e| panic!("{e}")).path();
        if path.extension().is_none_or(|ext| ext != "yaml") {
            continue;
        }
        let raw = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{e}"));
        let head = parse_head(&raw).unwrap_or_else(|| panic!("{} unreadable", path.display()));
        let id = head.id;
        let mut got = head.levels;
        let mut want = levels.get(&id).cloned().unwrap_or_default();
        want.sort();
        got.sort();
        assert_eq!(got, want, "{id}: manifest levels differ from graph");
    }
}

struct Case {
    name: &'static str,
    feature_status: &'static str,
    feature_reason: &'static str,
    scenario: bool,
    scenario_status: &'static str,
    scenario_reason: &'static str,
    want_problem: bool,
}

const CASES: [Case; 7] = [
    Case {
        name: "dropped_feature",
        feature_status: "dropped",
        feature_reason: "Outside scope",
        scenario: false,
        scenario_status: "",
        scenario_reason: "",
        want_problem: false,
    },
    Case {
        name: "feature_requires_reason",
        feature_status: "dropped",
        feature_reason: "",
        scenario: false,
        scenario_status: "",
        scenario_reason: "",
        want_problem: true,
    },
    Case {
        name: "active_feature_requires_scenarios",
        feature_status: "done",
        feature_reason: "",
        scenario: false,
        scenario_status: "",
        scenario_reason: "",
        want_problem: true,
    },
    Case {
        name: "dropped_scenario",
        feature_status: "done",
        feature_reason: "",
        scenario: true,
        scenario_status: "dropped",
        scenario_reason: "Restricted entitlement",
        want_problem: false,
    },
    Case {
        name: "scenario_requires_reason",
        feature_status: "done",
        feature_reason: "",
        scenario: true,
        scenario_status: "dropped",
        scenario_reason: "",
        want_problem: true,
    },
    Case {
        name: "active_scenario_requires_proof",
        feature_status: "done",
        feature_reason: "",
        scenario: true,
        scenario_status: "",
        scenario_reason: "",
        want_problem: true,
    },
    Case {
        name: "unknown_scenario_status",
        feature_status: "done",
        feature_reason: "",
        scenario: true,
        scenario_status: "ignored",
        scenario_reason: "",
        want_problem: true,
    },
];

#[test]
fn dropped_qualification() {
    for case in CASES {
        let root = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let mut feature = Feature {
            id: "example".to_owned(),
            status: case.feature_status.to_owned(),
            notes: case.feature_reason.to_owned(),
            requirements: vec!["R-API".to_owned()],
            ..Feature::default()
        };
        if case.scenario {
            let directory = root.path().join("lab/scenarios");
            fs::create_dir_all(&directory).unwrap_or_else(|e| panic!("{e}"));
            fs::write(
                directory.join("EXAMPLE.yaml"),
                "id: EXAMPLE\nfeature: example\nlevels: [sim]\n",
            )
            .unwrap_or_else(|e| panic!("{e}"));
            feature.scenarios = vec![ScenarioRef {
                id: "EXAMPLE".to_owned(),
                levels: vec!["sim".to_owned()],
                status: case.scenario_status.to_owned(),
                reason: case.scenario_reason.to_owned(),
            }];
        }
        let graph = Graph {
            statuses: vec!["done".to_owned(), "dropped".to_owned()],
            levels: BTreeMap::from([("sim".to_owned(), "simulation".to_owned())]),
            requirements: BTreeMap::from([("R-API".to_owned(), "API".to_owned())]),
            features: vec![feature],
        };
        let mut checker = Checker::new(root.path(), graph);
        let views = checker.check();
        assert_eq!(
            !checker.problems.is_empty(),
            case.want_problem,
            "{}: {:?}",
            case.name,
            checker.problems
        );
        if !case.want_problem && case.scenario {
            assert_eq!(
                views[0].views[0].results[0].state, "dropped",
                "{}",
                case.name
            );
        }
    }
}

#[test]
fn html_escapes_like_go_templates() {
    let root = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let graph = Graph {
        statuses: vec!["todo".to_owned()],
        requirements: BTreeMap::from([("R-API".to_owned(), "API".to_owned())]),
        features: vec![Feature {
            id: "a&b+c".to_owned(),
            title: "<t> \"q\" 'x' +".to_owned(),
            status: "todo".to_owned(),
            requirements: vec!["R-API".to_owned()],
            depends_on: vec!["a&b+c".to_owned()],
            ..Feature::default()
        }],
        ..Graph::default()
    };
    let mut checker = Checker::new(root.path(), graph);
    let views = checker.check();
    let html = ab_featuregraph::render(&checker.graph, &views);
    assert!(html.contains("id=\"a&amp;b&#43;c\""));
    assert!(html.contains("<div>&lt;t&gt; &#34;q&#34; &#39;x&#39; &#43;</div>"));
    assert!(html.contains("href=\"#a%26b%2bc\""));
}
