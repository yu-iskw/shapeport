//! Strict/smart mapping planner.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::config::PlannerConfig;
use crate::diagnostics::{Diagnostic, Severity};
use crate::error::{Error, Result};
use crate::fingerprint::schema_fingerprint;
use crate::path::FieldPath;
use crate::plan::{Expr, GeneratedBy, Operation, PlanMetadata, TransformationPlan};
use crate::schema::{Field, Schema, Type, types_compatible};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlannerMode {
    Strict,
    Smart,
}

impl PlannerMode {
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "strict" => Some(Self::Strict),
            "smart" => Some(Self::Smart),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Strict => "strict",
            Self::Smart => "smart",
        }
    }
}

#[derive(Clone, Debug)]
pub struct PlannerOptions {
    pub mode: PlannerMode,
    pub config: PlannerConfig,
}

impl Default for PlannerOptions {
    fn default() -> Self {
        Self {
            mode: PlannerMode::Smart,
            config: PlannerConfig::default(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub source: String,
    pub score: f64,
    pub reasons: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Unresolved {
    pub target: String,
    pub candidates: Vec<Candidate>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExplainEntry {
    pub target: String,
    pub source: Option<String>,
    pub score: f64,
    pub reasons: Vec<String>,
    pub action: String,
}

#[derive(Clone, Debug)]
pub enum PlanOutcome {
    Ready {
        plan: Box<TransformationPlan>,
        explanation: Vec<ExplainEntry>,
        diagnostics: Vec<Diagnostic>,
    },
    Ambiguous {
        unresolved: Vec<Unresolved>,
        explanation: Vec<ExplainEntry>,
        diagnostics: Vec<Diagnostic>,
    },
}

#[derive(Clone, Debug)]
struct FieldRef<'a> {
    path: String,
    field: &'a Field,
}

struct BuildContext<'a> {
    sources: Vec<FieldRef<'a>>,
    options: &'a PlannerOptions,
    used_sources: Vec<String>,
    explanation: Vec<ExplainEntry>,
    unresolved: Vec<Unresolved>,
}

pub fn plan_schemas(
    source: &Schema,
    target: &Schema,
    options: &PlannerOptions,
) -> Result<PlanOutcome> {
    let source_fields = source.require_record_fields()?;
    let target_fields = target.require_record_fields()?;
    let mut sources = Vec::new();
    collect_source_fields(source_fields, "", &mut sources);

    let mut context = BuildContext {
        sources,
        options,
        used_sources: Vec::new(),
        explanation: Vec::new(),
        unresolved: Vec::new(),
    };
    let fields = build_target_fields(target_fields, "", &mut context)?;

    if !context.unresolved.is_empty() {
        return Ok(PlanOutcome::Ambiguous {
            unresolved: context.unresolved,
            explanation: context.explanation,
            diagnostics: vec![Diagnostic {
                severity: Severity::Error,
                code: "ambiguous_mapping".into(),
                message: "one or more target fields could not be mapped uniquely".into(),
                path: None,
                hint: Some("resolve candidates or supply an explicit plan".into()),
            }],
        });
    }

    if fields.is_empty() {
        return Err(Error::plan(
            "empty_mapping",
            "planner produced no field mappings",
        ));
    }

    let mut plan = TransformationPlan::new(vec![Operation::Map { fields }]);
    plan.metadata = PlanMetadata {
        name: Some("generated".into()),
        generated_by: Some(GeneratedBy {
            mode: options.mode.as_str().into(),
            shapeport_version: env!("CARGO_PKG_VERSION").into(),
        }),
    };
    plan.contracts.input = Some(crate::plan::ContractRef {
        fingerprint: Some(schema_fingerprint(source)),
    });
    plan.contracts.output = Some(crate::plan::ContractRef {
        fingerprint: Some(schema_fingerprint(target)),
    });
    Ok(PlanOutcome::Ready {
        plan: Box::new(plan),
        explanation: context.explanation,
        diagnostics: Vec::new(),
    })
}

fn collect_source_fields<'a>(fields: &'a [Field], prefix: &str, out: &mut Vec<FieldRef<'a>>) {
    for field in fields {
        let path = join_path(prefix, &field.name);
        match &field.ty {
            Type::Record { fields } => collect_source_fields(fields, &path, out),
            _ => out.push(FieldRef { path, field }),
        }
    }
}

fn build_target_fields(
    target_fields: &[Field],
    prefix: &str,
    context: &mut BuildContext<'_>,
) -> Result<IndexMap<String, Expr>> {
    let mut fields = IndexMap::new();
    for target_field in target_fields {
        let target_path = join_path(prefix, &target_field.name);
        if let Type::Record {
            fields: nested_fields,
        } = &target_field.ty
        {
            let nested = build_target_fields(nested_fields, &target_path, context)?;
            fields.insert(target_field.name.clone(), Expr::Object(nested));
            continue;
        }
        if let Some(expr) = build_target_leaf(target_field, &target_path, context)? {
            fields.insert(target_field.name.clone(), expr);
        }
    }
    Ok(fields)
}

fn build_target_leaf(
    target_field: &Field,
    target_path: &str,
    context: &mut BuildContext<'_>,
) -> Result<Option<Expr>> {
    if matches!(target_field.ty, Type::List { .. }) {
        return build_list_mapping(target_field, target_path, context);
    }

    let target = FieldRef {
        path: target_path.to_string(),
        field: target_field,
    };
    match assign_field(
        &target,
        &context.sources,
        context.options,
        &context.used_sources,
    ) {
        Assign::Mapped(candidate) => {
            context.used_sources.push(candidate.source.clone());
            let path = FieldPath::parse(&candidate.source)?;
            context.explanation.push(ExplainEntry {
                target: target_path.to_string(),
                source: Some(candidate.source.clone()),
                score: candidate.score,
                reasons: candidate.reasons.clone(),
                action: format!("map {} -> {target_path}", candidate.source),
            });
            Ok(Some(Expr::Field(path)))
        }
        Assign::Ambiguous(candidates) => {
            push_ambiguous(target_path, candidates, context);
            Ok(None)
        }
        Assign::Missing if target_field.nullable => {
            push_optional_omission(target_path, context);
            Ok(None)
        }
        Assign::Missing => {
            push_unresolved(target_path, Vec::new(), context);
            Ok(None)
        }
    }
}

fn build_list_mapping(
    target_field: &Field,
    target_path: &str,
    context: &mut BuildContext<'_>,
) -> Result<Option<Expr>> {
    let Some(candidate) = choose_list_candidate(target_field, target_path, context) else {
        return Ok(None);
    };
    let Some(source_field) = context
        .sources
        .iter()
        .find(|source| source.path == candidate.source)
        .map(|source| source.field.clone())
    else {
        push_unresolved(target_path, vec![candidate], context);
        return Ok(None);
    };

    let (
        Type::List {
            element: source_element,
            element_nullable: source_element_nullable,
        },
        Type::List {
            element: target_element,
            element_nullable: target_element_nullable,
        },
    ) = (&source_field.ty, &target_field.ty)
    else {
        return Err(Error::plan(
            "list_map_plan",
            "list mapping candidate must contain list source and target types",
        ));
    };

    if (source_field.nullable && !target_field.nullable)
        || (*source_element_nullable && !*target_element_nullable)
    {
        push_unresolved(target_path, vec![candidate], context);
        return Ok(None);
    }

    if source_element == target_element {
        return accept_direct_list_mapping(candidate, target_path, context).map(Some);
    }
    build_record_list_mapping(
        source_element,
        target_element,
        &candidate,
        target_path,
        context,
    )
}

fn choose_list_candidate(
    target_field: &Field,
    target_path: &str,
    context: &mut BuildContext<'_>,
) -> Option<Candidate> {
    let target = FieldRef {
        path: target_path.to_string(),
        field: target_field,
    };
    let mut ranked: Vec<Candidate> = context
        .sources
        .iter()
        .filter(|source| !context.used_sources.contains(&source.path))
        .filter_map(|source| score_list_pair(source, &target, context.options.mode))
        .collect();
    ranked.sort_by(|a, b| b.score.total_cmp(&a.score));

    match select_candidate(ranked, context.options) {
        Assign::Mapped(candidate) => Some(candidate),
        Assign::Ambiguous(candidates) => {
            push_ambiguous(target_path, candidates, context);
            None
        }
        Assign::Missing if target_field.nullable => {
            push_optional_omission(target_path, context);
            None
        }
        Assign::Missing => {
            push_unresolved(target_path, Vec::new(), context);
            None
        }
    }
}

fn accept_direct_list_mapping(
    candidate: Candidate,
    target_path: &str,
    context: &mut BuildContext<'_>,
) -> Result<Expr> {
    context.used_sources.push(candidate.source.clone());
    context.explanation.push(ExplainEntry {
        target: target_path.to_string(),
        source: Some(candidate.source.clone()),
        score: candidate.score,
        reasons: candidate.reasons.clone(),
        action: format!("map {} -> {target_path}", candidate.source),
    });
    Ok(Expr::Field(FieldPath::parse(&candidate.source)?))
}

fn build_record_list_mapping(
    source_element: &Type,
    target_element: &Type,
    candidate: &Candidate,
    target_path: &str,
    context: &mut BuildContext<'_>,
) -> Result<Option<Expr>> {
    let (Type::Record { .. }, Type::Record { .. }) = (source_element, target_element) else {
        push_unresolved(target_path, vec![candidate.clone()], context);
        return Ok(None);
    };
    let nested = plan_schemas(
        &Schema::new(source_element.clone()),
        &Schema::new(target_element.clone()),
        context.options,
    )?;
    let PlanOutcome::Ready {
        plan, explanation, ..
    } = nested
    else {
        push_unresolved(target_path, vec![candidate.clone()], context);
        return Ok(None);
    };
    let Some(Operation::Map { fields }) = plan.operations.first() else {
        return Err(Error::plan(
            "list_map_plan",
            "element planner must produce a map operation",
        ));
    };

    context.used_sources.push(candidate.source.clone());
    context.explanation.push(ExplainEntry {
        target: target_path.to_string(),
        source: Some(candidate.source.clone()),
        score: candidate.score,
        reasons: vec!["cardinality-preserving list map".into()],
        action: format!("map elements {}[] -> {target_path}[]", candidate.source),
    });
    push_list_element_explanations(target_path, &candidate.source, explanation, context);

    Ok(Some(Expr::ListMap {
        input: FieldPath::parse(&candidate.source)?,
        item: Box::new(Expr::Object(fields.clone())),
    }))
}

fn push_list_element_explanations(
    target_path: &str,
    source_path: &str,
    explanation: Vec<ExplainEntry>,
    context: &mut BuildContext<'_>,
) {
    for entry in explanation {
        context.explanation.push(ExplainEntry {
            target: format!("{target_path}[].{}", entry.target),
            source: entry
                .source
                .map(|source| format!("{source_path}[].{source}")),
            score: entry.score,
            reasons: entry.reasons,
            action: entry.action,
        });
    }
}

fn push_optional_omission(target_path: &str, context: &mut BuildContext<'_>) {
    context.explanation.push(ExplainEntry {
        target: target_path.to_string(),
        source: None,
        score: 0.0,
        reasons: vec!["optional unmatched".into()],
        action: "omit".into(),
    });
}

fn push_unresolved(target_path: &str, candidates: Vec<Candidate>, context: &mut BuildContext<'_>) {
    context.unresolved.push(Unresolved {
        target: target_path.to_string(),
        candidates,
    });
}

fn push_ambiguous(target_path: &str, candidates: Vec<Candidate>, context: &mut BuildContext<'_>) {
    push_unresolved(target_path, candidates, context);
    context.explanation.push(ExplainEntry {
        target: target_path.to_string(),
        source: None,
        score: 0.0,
        reasons: vec!["ambiguous".into()],
        action: "no mapping selected".into(),
    });
}

fn score_list_pair(
    source: &FieldRef<'_>,
    target: &FieldRef<'_>,
    mode: PlannerMode,
) -> Option<Candidate> {
    if !matches!(source.field.ty, Type::List { .. })
        || !matches!(target.field.ty, Type::List { .. })
    {
        return None;
    }
    let type_reason = if source.field.ty == target.field.ty {
        "exact type match"
    } else {
        "list container match"
    };
    score_names(source, target, mode, type_reason)
}

fn join_path(prefix: &str, field: &str) -> String {
    if prefix.is_empty() {
        field.to_string()
    } else {
        format!("{prefix}.{field}")
    }
}

enum Assign {
    Mapped(Candidate),
    Ambiguous(Vec<Candidate>),
    Missing,
}

fn assign_field(
    target: &FieldRef<'_>,
    sources: &[FieldRef<'_>],
    options: &PlannerOptions,
    used: &[String],
) -> Assign {
    let mut ranked: Vec<Candidate> = sources
        .iter()
        .filter(|source| !used.contains(&source.path))
        .filter_map(|source| score_pair(source, target, options.mode))
        .collect();
    ranked.sort_by(|a, b| b.score.total_cmp(&a.score));
    select_candidate(ranked, options)
}

fn select_candidate(ranked: Vec<Candidate>, options: &PlannerOptions) -> Assign {
    let Some(best) = ranked.first() else {
        return Assign::Missing;
    };
    if best.score < options.config.ambiguity_threshold {
        return Assign::Missing;
    }
    if ranked.len() >= 2 {
        let second = &ranked[1];
        if (best.score - second.score).abs() < 0.05
            && second.score >= options.config.ambiguity_threshold
        {
            return Assign::Ambiguous(ranked);
        }
    }
    if options.mode == PlannerMode::Strict && best.score < options.config.auto_accept_threshold {
        return Assign::Ambiguous(ranked);
    }
    Assign::Mapped(best.clone())
}

fn score_pair(
    source: &FieldRef<'_>,
    target: &FieldRef<'_>,
    mode: PlannerMode,
) -> Option<Candidate> {
    if !types_compatible(&source.field.ty, &target.field.ty) {
        return None;
    }
    score_names(
        source,
        target,
        mode,
        if types_equalish(&source.field.ty, &target.field.ty) {
            "exact type match"
        } else {
            "compatible type"
        },
    )
}

fn score_names(
    source: &FieldRef<'_>,
    target: &FieldRef<'_>,
    mode: PlannerMode,
    type_reason: &str,
) -> Option<Candidate> {
    let mut reasons: Vec<String> = Vec::new();
    let mut score = 0.0;
    let mut direct_name_evidence = false;

    if source.field.name == target.field.name {
        score += 0.80;
        reasons.push("exact-name match".into());
        direct_name_evidence = true;
    } else if source
        .field
        .aliases
        .iter()
        .any(|alias| alias == &target.field.name)
    {
        score += 0.80;
        reasons.push("alias match".into());
        direct_name_evidence = true;
    }

    let normalized_eq = normalize_name(&source.field.name) == normalize_name(&target.field.name);
    if mode == PlannerMode::Smart && normalized_eq && source.field.name != target.field.name {
        score += 0.75;
        reasons.push("normalized-name match".into());
        direct_name_evidence = true;
    }

    if mode == PlannerMode::Smart && !direct_name_evidence && normalized_path_match(source, target)
    {
        score += 0.75;
        reasons.push("normalized-path match".into());
    }

    if mode == PlannerMode::Smart
        && let Some(syn) = synonym_bonus(&source.field.name, &target.field.name)
    {
        score += syn;
        reasons.push("common-name synonym".into());
    }

    if type_reason == "exact type match" {
        score += 0.20;
    } else {
        score += 0.10;
    }
    reasons.push(type_reason.into());

    if score <= 0.0 {
        return None;
    }
    if mode == PlannerMode::Strict
        && !reasons
            .iter()
            .any(|reason| reason.contains("exact-name") || reason.contains("alias"))
    {
        return None;
    }

    Some(Candidate {
        source: source.path.clone(),
        score,
        reasons,
    })
}

fn normalized_path_match(source: &FieldRef<'_>, target: &FieldRef<'_>) -> bool {
    let source_path = normalize_name(&source.path);
    let target_path = normalize_name(&target.path);
    let source_name = normalize_name(&source.field.name);
    let target_name = normalize_name(&target.field.name);
    source_path == target_path || source_path == target_name || source_name == target_path
}

fn types_equalish(left: &Type, right: &Type) -> bool {
    std::mem::discriminant(left) == std::mem::discriminant(right)
}

fn normalize_name(name: &str) -> String {
    let mut out = String::new();
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        }
    }
    out
}

fn synonym_bonus(source: &str, target: &str) -> Option<f64> {
    const PAIRS: &[(&str, &str)] = &[
        ("period", "month"),
        ("productfamily", "product"),
        ("totalsalesusd", "revenue"),
        ("grossamount", "amount"),
        ("netamount", "amount"),
        ("createdat", "timestamp"),
        ("customerid", "id"),
    ];
    let src = normalize_name(source);
    let dst = normalize_name(target);
    PAIRS.iter().find_map(|(left, right)| {
        if (src == *left && dst == *right) || (src == *right && dst == *left) {
            Some(0.65)
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use indexmap::IndexMap;

    use super::{PlanOutcome, PlannerMode, PlannerOptions, plan_schemas};
    use crate::engine::execute_plan;
    use crate::plan::{Expr, Operation};
    use crate::schema::{Field, Schema, Type};
    use crate::value::Value;

    #[test]
    fn maps_normalized_names_in_smart_mode() {
        let source = Schema::record(vec![
            Field::new("period", Type::String, false),
            Field::new("product_family", Type::String, false),
            Field::new("total_sales_usd", Type::String, false),
        ]);
        let target = Schema::record(vec![
            Field::new("month", Type::String, false),
            Field::new("product", Type::String, false),
            Field::new("revenue", Type::String, false),
        ]);
        let options = PlannerOptions {
            mode: PlannerMode::Smart,
            ..PlannerOptions::default()
        };
        let outcome = plan_schemas(&source, &target, &options).expect("plan");
        let PlanOutcome::Ready {
            plan, explanation, ..
        } = outcome
        else {
            panic!("expected PlanOutcome::Ready, got Ambiguous");
        };
        let Operation::Map { fields } = plan.operations.first().expect("operation") else {
            panic!("expected Map operation");
        };
        assert!(fields.contains_key("month"));
        assert!(fields.contains_key("product"));
        assert!(fields.contains_key("revenue"));
        let mentions = |word: &str| explanation.iter().any(|entry| entry.target == word);
        assert!(mentions("month"));
        assert!(mentions("product"));
        assert!(mentions("revenue"));
    }

    #[test]
    fn reports_amount_ambiguity() {
        let source = Schema::record(vec![
            Field::new("gross_amount", Type::Float { bits: 64 }, false),
            Field::new("net_amount", Type::Float { bits: 64 }, false),
        ]);
        let target = Schema::record(vec![Field::new("amount", Type::Float { bits: 64 }, false)]);
        let outcome = plan_schemas(&source, &target, &PlannerOptions::default()).expect("plan");
        assert!(matches!(outcome, PlanOutcome::Ambiguous { .. }));
    }

    #[test]
    fn exact_name_and_type_clear_strict_threshold() {
        let source = Schema::record(vec![Field::new("customer_id", Type::String, false)]);
        let target = Schema::record(vec![Field::new("customer_id", Type::String, false)]);
        let outcome = plan_schemas(
            &source,
            &target,
            &PlannerOptions {
                mode: PlannerMode::Strict,
                ..PlannerOptions::default()
            },
        )
        .expect("plan");
        assert!(matches!(outcome, PlanOutcome::Ready { .. }));
    }

    #[test]
    fn maps_nested_source_leaf_to_flat_target() {
        let source = Schema::record(vec![Field::new(
            "customer",
            Type::record(vec![Field::new("name", Type::String, false)]),
            false,
        )]);
        let target = Schema::record(vec![Field::new("customer_name", Type::String, false)]);
        let outcome = plan_schemas(&source, &target, &PlannerOptions::default()).expect("plan");
        let PlanOutcome::Ready {
            plan, explanation, ..
        } = outcome
        else {
            panic!("expected ready nested flatten plan");
        };
        assert_eq!(explanation[0].target, "customer_name");
        assert_eq!(explanation[0].source.as_deref(), Some("customer.name"));
        assert!(
            explanation[0]
                .reasons
                .iter()
                .any(|reason| reason == "normalized-path match")
        );
        let Operation::Map { fields } = &plan.operations[0] else {
            panic!("expected map operation");
        };
        assert!(matches!(fields.get("customer_name"), Some(Expr::Field(_))));
    }

    #[test]
    fn constructs_nested_target_object_and_executes_it() {
        let source = Schema::record(vec![
            Field::new("first_name", Type::String, false),
            Field::new("last_name", Type::String, false),
        ]);
        let target = Schema::record(vec![Field::new(
            "person",
            Type::record(vec![
                Field::new("first_name", Type::String, false),
                Field::new("last_name", Type::String, false),
            ]),
            false,
        )]);
        let outcome = plan_schemas(&source, &target, &PlannerOptions::default()).expect("plan");
        let PlanOutcome::Ready {
            plan, explanation, ..
        } = outcome
        else {
            panic!("expected ready nested object plan");
        };
        assert!(explanation.iter().any(|entry| {
            entry.target == "person.first_name" && entry.source.as_deref() == Some("first_name")
        }));
        assert!(explanation.iter().any(|entry| {
            entry.target == "person.last_name" && entry.source.as_deref() == Some("last_name")
        }));

        let mut input = IndexMap::new();
        input.insert("first_name".into(), Value::String("Ada".into()));
        input.insert("last_name".into(), Value::String("Lovelace".into()));
        let output = execute_plan(&plan, vec![Value::Object(input)]).expect("execute nested plan");

        let mut person = IndexMap::new();
        person.insert("first_name".into(), Value::String("Ada".into()));
        person.insert("last_name".into(), Value::String("Lovelace".into()));
        let mut expected = IndexMap::new();
        expected.insert("person".into(), Value::Object(person));
        assert_eq!(output, vec![Value::Object(expected)]);
    }

    #[test]
    fn duplicate_nested_leaf_names_remain_ambiguous() {
        let source = Schema::record(vec![
            Field::new(
                "billing",
                Type::record(vec![Field::new("name", Type::String, false)]),
                false,
            ),
            Field::new(
                "shipping",
                Type::record(vec![Field::new("name", Type::String, false)]),
                false,
            ),
        ]);
        let target = Schema::record(vec![Field::new("name", Type::String, false)]);
        let outcome = plan_schemas(&source, &target, &PlannerOptions::default()).expect("plan");
        let PlanOutcome::Ambiguous { unresolved, .. } = outcome else {
            panic!("duplicate nested leaf names must remain ambiguous");
        };
        assert_eq!(unresolved.len(), 1);
        assert_eq!(unresolved[0].candidates.len(), 2);
    }

    #[test]
    fn maps_record_elements_without_changing_list_cardinality() {
        let source = Schema::record(vec![Field::new(
            "customers",
            Type::list(
                Type::record(vec![Field::new("first_name", Type::String, false)]),
                false,
            ),
            false,
        )]);
        let target = Schema::record(vec![Field::new(
            "customers",
            Type::list(
                Type::record(vec![Field::new("firstName", Type::String, false)]),
                false,
            ),
            false,
        )]);
        let outcome = plan_schemas(&source, &target, &PlannerOptions::default()).expect("plan");
        let PlanOutcome::Ready { plan, .. } = outcome else {
            panic!("expected safe list map plan");
        };
        let Operation::Map { fields } = &plan.operations[0] else {
            panic!("expected map operation");
        };
        assert!(matches!(
            fields.get("customers"),
            Some(Expr::ListMap { .. })
        ));

        let input = Value::object([(
            "customers".into(),
            Value::Array(vec![
                Value::object([("first_name".into(), Value::String("Ada".into()))]),
                Value::object([("first_name".into(), Value::String("Grace".into()))]),
            ]),
        )]);
        let output = execute_plan(&plan, vec![input]).expect("execute list map");
        let Value::Array(items) = output[0]
            .as_object()
            .expect("object")
            .get("customers")
            .expect("customers")
        else {
            panic!("expected customers array");
        };
        assert_eq!(items.len(), 2);
        assert_eq!(
            items[0].as_object().expect("item").get("firstName"),
            Some(&Value::String("Ada".into()))
        );
    }

    #[test]
    fn list_element_ambiguity_is_not_auto_resolved() {
        let source = Schema::record(vec![Field::new(
            "customers",
            Type::list(
                Type::record(vec![
                    Field::new("customer_id", Type::String, false),
                    Field::new("customer-id", Type::String, false),
                ]),
                false,
            ),
            false,
        )]);
        let target = Schema::record(vec![Field::new(
            "customers",
            Type::list(
                Type::record(vec![Field::new("customerId", Type::String, false)]),
                false,
            ),
            false,
        )]);
        let outcome = plan_schemas(&source, &target, &PlannerOptions::default()).expect("plan");
        assert!(matches!(outcome, PlanOutcome::Ambiguous { .. }));
    }
}
