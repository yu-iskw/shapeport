//! Declarative planner conformance and adversarial benchmark harness.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use shapeport_core::planner::{PlanOutcome, plan_schemas};
use shapeport_core::{PlannerMode, PlannerOptions, Schema};

#[derive(Debug, Deserialize)]
struct Corpus {
    version: u32,
    cases: Vec<Case>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Case {
    name: String,
    family: String,
    mode: String,
    source_schema: Schema,
    target_schema: Schema,
    expect: Expectation,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Expectation {
    status: String,
    #[serde(default)]
    mappings: BTreeMap<String, String>,
    #[serde(default)]
    ambiguous_targets: Vec<String>,
    #[serde(default)]
    omitted_targets: Vec<String>,
    #[serde(default)]
    acceptable_candidates: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    reason_kinds: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    unsafe_auto_mapping: bool,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct Summary {
    corpus_version: u32,
    cases: usize,
    selected_mappings: usize,
    expected_mappings: usize,
    correct_mappings: usize,
    unsafe_auto_mappings: usize,
    expected_ambiguities: usize,
    reported_ambiguities: usize,
    correct_ambiguities: usize,
    false_ambiguities: usize,
    exact_plan_successes: usize,
    mapping_precision: f64,
    mapping_recall: f64,
    exact_plan_success_rate: f64,
    unsafe_auto_map_rate: f64,
    ambiguity_recall: f64,
    false_ambiguity_rate: f64,
}

#[derive(Debug)]
struct Observed {
    status: &'static str,
    mappings: BTreeMap<String, String>,
    unresolved: BTreeMap<String, Vec<String>>,
    omitted: BTreeSet<String>,
    reasons: BTreeMap<String, BTreeSet<String>>,
}

fn corpus_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/conformance/mapping/cases.yaml")
}

fn load_corpus() -> Corpus {
    let path = corpus_path();
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
    serde_norway::from_str(&text)
        .unwrap_or_else(|error| panic!("failed to parse {}: {error}", path.display()))
}

fn observe(case: &Case) -> Observed {
    let mode = PlannerMode::parse(&case.mode)
        .unwrap_or_else(|| panic!("{}: unsupported planner mode {}", case.name, case.mode));
    let outcome = plan_schemas(
        &case.source_schema,
        &case.target_schema,
        &PlannerOptions {
            mode,
            ..PlannerOptions::default()
        },
    )
    .unwrap_or_else(|error| panic!("{}: planner failed: {error}", case.name));

    match outcome {
        PlanOutcome::Ready { explanation, .. } => Observed {
            status: "ready",
            mappings: explanation_mappings(&explanation),
            unresolved: BTreeMap::new(),
            omitted: explanation_omitted(&explanation),
            reasons: explanation_reasons(&explanation),
        },
        PlanOutcome::Ambiguous {
            unresolved,
            explanation,
            ..
        } => Observed {
            status: "ambiguous",
            mappings: explanation_mappings(&explanation),
            unresolved: unresolved
                .into_iter()
                .map(|entry| {
                    (
                        entry.target,
                        entry
                            .candidates
                            .into_iter()
                            .map(|candidate| candidate.source)
                            .collect(),
                    )
                })
                .collect(),
            omitted: explanation_omitted(&explanation),
            reasons: explanation_reasons(&explanation),
        },
    }
}

fn explanation_mappings(
    explanation: &[shapeport_core::planner::ExplainEntry],
) -> BTreeMap<String, String> {
    explanation
        .iter()
        .filter_map(|entry| {
            entry
                .source
                .as_ref()
                .map(|source| (entry.target.clone(), source.clone()))
        })
        .collect()
}

fn explanation_omitted(
    explanation: &[shapeport_core::planner::ExplainEntry],
) -> BTreeSet<String> {
    explanation
        .iter()
        .filter(|entry| entry.action == "omit")
        .map(|entry| entry.target.clone())
        .collect()
}

fn explanation_reasons(
    explanation: &[shapeport_core::planner::ExplainEntry],
) -> BTreeMap<String, BTreeSet<String>> {
    explanation
        .iter()
        .map(|entry| (entry.target.clone(), entry.reasons.iter().cloned().collect()))
        .collect()
}

fn assert_case(case: &Case, observed: &Observed) {
    assert_eq!(
        observed.status, case.expect.status,
        "{}: planner status mismatch",
        case.name
    );
    assert_eq!(
        observed.mappings, case.expect.mappings,
        "{}: selected mappings mismatch",
        case.name
    );

    let actual_ambiguous: BTreeSet<_> = observed.unresolved.keys().cloned().collect();
    let expected_ambiguous: BTreeSet<_> =
        case.expect.ambiguous_targets.iter().cloned().collect();
    assert_eq!(
        actual_ambiguous, expected_ambiguous,
        "{}: ambiguous targets mismatch",
        case.name
    );

    let expected_omitted: BTreeSet<_> = case.expect.omitted_targets.iter().cloned().collect();
    assert_eq!(
        observed.omitted, expected_omitted,
        "{}: omitted targets mismatch",
        case.name
    );

    for (target, acceptable) in &case.expect.acceptable_candidates {
        let actual = observed
            .unresolved
            .get(target)
            .unwrap_or_else(|| panic!("{}: unresolved target {target} missing", case.name));
        let actual: BTreeSet<_> = actual.iter().cloned().collect();
        let acceptable: BTreeSet<_> = acceptable.iter().cloned().collect();
        assert_eq!(
            actual, acceptable,
            "{}: candidate set mismatch for {target}",
            case.name
        );
    }

    for (target, expected_reasons) in &case.expect.reason_kinds {
        let actual = observed
            .reasons
            .get(target)
            .unwrap_or_else(|| panic!("{}: explanation for {target} missing", case.name));
        for reason in expected_reasons {
            assert!(
                actual.contains(reason),
                "{}: expected reason {reason:?} for {target}; actual={actual:?}",
                case.name
            );
        }
    }

    let unsafe_mapping = observed.mappings.iter().any(|(target, source)| {
        case.expect.mappings.get(target) != Some(source)
    });
    assert_eq!(
        unsafe_mapping, case.expect.unsafe_auto_mapping,
        "{}: unsafe auto-mapping expectation mismatch",
        case.name
    );
}

fn add_metrics(summary: &mut Summary, case: &Case, observed: &Observed) {
    summary.cases += 1;
    summary.selected_mappings += observed.mappings.len();
    summary.expected_mappings += case.expect.mappings.len();
    summary.correct_mappings += observed
        .mappings
        .iter()
        .filter(|(target, source)| case.expect.mappings.get(*target) == Some(*source))
        .count();
    summary.unsafe_auto_mappings += observed
        .mappings
        .iter()
        .filter(|(target, source)| case.expect.mappings.get(*target) != Some(*source))
        .count();

    let expected_ambiguities: BTreeSet<_> =
        case.expect.ambiguous_targets.iter().cloned().collect();
    let actual_ambiguities: BTreeSet<_> = observed.unresolved.keys().cloned().collect();
    summary.expected_ambiguities += expected_ambiguities.len();
    summary.reported_ambiguities += actual_ambiguities.len();
    summary.correct_ambiguities += expected_ambiguities
        .intersection(&actual_ambiguities)
        .count();
    summary.false_ambiguities += actual_ambiguities
        .difference(&expected_ambiguities)
        .count();

    if observed.status == case.expect.status
        && observed.mappings == case.expect.mappings
        && actual_ambiguities == expected_ambiguities
    {
        summary.exact_plan_successes += 1;
    }
}

fn ratio(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        1.0
    } else {
        numerator as f64 / denominator as f64
    }
}

fn finish_metrics(summary: &mut Summary) {
    summary.mapping_precision = ratio(summary.correct_mappings, summary.selected_mappings);
    summary.mapping_recall = ratio(summary.correct_mappings, summary.expected_mappings);
    summary.exact_plan_success_rate = ratio(summary.exact_plan_successes, summary.cases);
    summary.unsafe_auto_map_rate = ratio(summary.unsafe_auto_mappings, summary.selected_mappings);
    summary.ambiguity_recall = ratio(summary.correct_ambiguities, summary.expected_ambiguities);
    summary.false_ambiguity_rate = ratio(summary.false_ambiguities, summary.reported_ambiguities);
}

#[test]
fn mapping_conformance_corpus() {
    let corpus = load_corpus();
    assert_eq!(corpus.version, 1, "unsupported corpus version");

    let family_filter = std::env::var("SHAPEPORT_CONFORMANCE_FAMILY").ok();
    let mut summary = Summary {
        corpus_version: corpus.version,
        ..Summary::default()
    };

    for case in corpus.cases.iter().filter(|case| {
        family_filter
            .as_ref()
            .is_none_or(|family| case.family.starts_with(family))
    }) {
        let observed = observe(case);
        assert_case(case, &observed);
        add_metrics(&mut summary, case, &observed);
    }

    assert!(summary.cases > 0, "conformance filter selected no cases");
    finish_metrics(&mut summary);

    let json = serde_json::to_string_pretty(&summary).expect("serialize benchmark summary");
    println!("SHAPEPORT_CONFORMANCE_SUMMARY={json}");

    if let Ok(path) = std::env::var("SHAPEPORT_CONFORMANCE_JSON") {
        std::fs::write(&path, format!("{json}\n"))
            .unwrap_or_else(|error| panic!("failed to write {path}: {error}"));
    }
}
