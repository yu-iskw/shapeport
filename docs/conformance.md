# Planner Conformance Corpus

ShapePort's schema-mapping corpus is the primary regression harness for deterministic planner correctness.

The design principle is:

```text
safe correct mapping
        >
reported ambiguity
        >
unmappable
        >>>
confidently incorrect mapping
```

The corpus intentionally treats an unresolved mapping as safer than a plausible but semantically incorrect automatic mapping.

## Run the suite

```bash
make conformance
```

The integration test is also included in the normal workspace test run:

```bash
make test
```

Run only a family:

```bash
SHAPEPORT_CONFORMANCE_FAMILY=ambiguous make conformance
SHAPEPORT_CONFORMANCE_FAMILY=integrations/flint make conformance
```

Write the machine-readable summary to a file:

```bash
SHAPEPORT_CONFORMANCE_JSON=target/conformance.json make conformance
```

## Fixture format

Fixtures live under `tests/conformance/mapping/`. YAML files contain versioned collections of declarative cases. The harness loads the corpus groups together. The format deliberately describes public planner behavior rather than serializing planner-internal Rust types.

```yaml
version: 1
cases:
  - name: normalized-snake-to-camel
    family: naming-convention
    mode: smart
    sourceSchema:
      root:
        kind: record
        fields:
          - name: customer_id
            type: { kind: string }
            nullable: false
    targetSchema:
      root:
        kind: record
        fields:
          - name: customerId
            type: { kind: string }
            nullable: false
    expect:
      status: ready
      mappings:
        customerId: customer_id
      reasonKinds:
        customerId:
          - normalized-name match
          - exact type match
```

Supported expectations are:

- `status`: `ready` or `ambiguous` for the current planner API;
- `mappings`: exact target-to-source mappings that may be selected;
- `ambiguousTargets`: targets that must remain unresolved;
- `omittedTargets`: nullable unmatched targets that should be omitted;
- `acceptableCandidates`: the exact candidate set for an unresolved target when candidate behavior matters;
- `reasonKinds`: stable explanation reason categories to assert without coupling tests to prose formatting;
- `unsafeAutoMapping`: whether the fixture intentionally expects an unsafe automatic mapping. This should normally remain `false` and exists so the harness can explicitly describe a known baseline rather than hide it.

## Scenario families

The current corpus covers:

- exact and normalized names;
- aliases and schema-evolution renames;
- compatible and incompatible type behavior;
- decimal preservation;
- timestamp preservation;
- nullability and missing required fields;
- default-bearing fields;
- enum-domain preservation;
- cardinality-preserving list mapping and adversarial list-element ambiguity;
- flattening safety;
- nested object construction safety;
- ambiguous mappings and adversarial collisions;
- real integration-shaped Flint, MCP, and warehouse contracts.

The purpose of a family is to pin safe planner behavior even when synthesis for that operation does not exist yet. The default fixture verifies safe omission of an optional default-bearing target; it does not claim that the planner synthesizes literal defaults yet.

### List mapping invariant

List element synthesis is deliberately narrower than generic "array support." ShapePort may infer `List<S> -> List<T>` only when the list container maps unambiguously and the element transformation is itself deterministic. A generated list map preserves both order and cardinality:

```text
len(output) == len(input)
order(output) == order(input)
```

Identical element types may be preserved directly. Unequal record element types recurse through the normal record planner. Child ambiguity is propagated to the outer list target instead of falling back to whole-list passthrough. Nullable source lists or elements are not inferred into stricter non-null target contracts.

The planner does **not** infer filtering, explosion, reduction, aggregation, or joins from list schemas. Those operations change cardinality or require semantic intent and remain explicit operations.

When a follow-up capability adds structural planning such as enum-domain conversion, timestamp parsing, defaults, or cardinality-changing collection operations, update the relevant fixture in the same change that implements the behavior. Execution-level fixtures can additionally carry input and expected output once the planner synthesizes the required operation.

## Metrics

The runner prints a JSON summary containing:

- `mappingPrecision`;
- `mappingRecall`;
- `exactPlanSuccessRate`;
- `unsafeAutoMapRate`;
- `ambiguityRecall`;
- `falseAmbiguityRate`;
- raw mapping and ambiguity counts.

The aggregate rates remain descriptive rather than arbitrary percentage gates. Stage 2 instead uses reviewed monotonic raw-count invariants so corpus growth does not make the safety policy depend on shifting denominators.

## Reviewed regression baseline

The full corpus is compared with `tests/conformance/mapping/baseline.json`. The baseline is intentionally small and human-reviewable:

```json
{
  "corpusVersion": 1,
  "cases": 25,
  "correctMappings": 22,
  "unsafeAutoMappings": 0,
  "correctAmbiguities": 8,
  "falseAmbiguities": 0,
  "exactPlanSuccesses": 25
}
```

A full-corpus run enforces these monotonic rules:

- `cases` must not decrease;
- `correctMappings` must not decrease;
- `unsafeAutoMappings` must not increase;
- `correctAmbiguities` must not decrease;
- `falseAmbiguities` must not increase;
- `exactPlanSuccesses` must not decrease;
- the corpus version must match the reviewed baseline version.

This is deliberately asymmetric. Improvements pass without changing the baseline. A safety regression fails even when aggregate percentages might still look acceptable. If a deliberate semantic change legitimately lowers one of the protected counts, the fixture expectations and `baseline.json` must be changed together so reviewers can see that the safety contract itself is changing.

Family-filtered developer runs do not apply the aggregate baseline because their counts represent only a subset. Individual fixture assertions still apply exactly.

## Rebaselining policy

Do **not** update `baseline.json` merely to make CI green. Rebaseline only when the planner contract intentionally changes or when the corpus is deliberately restructured. The same pull request should explain:

1. which protected metric changes;
2. which fixture or planner behavior causes the change;
3. why the new behavior is safer or otherwise intentional;
4. why an unsafe automatic mapping is not being hidden by the baseline update.

A baseline update is therefore a policy review point, not a generated snapshot refresh.

## Adding a case

A new case should:

1. model a concrete contract-adaptation behavior or failure mode;
2. distinguish safe ambiguity from an unsafe guess;
3. prefer schema/contract examples resembling real data tools;
4. assert reason categories where they are part of the behavior under test;
5. avoid relying on candidate scores unless a threshold itself is the behavior being tested;
6. avoid network access, LLM credentials, or nondeterministic services.

For bugs, add the failing fixture first, then make the smallest planner change that satisfies the intended semantics. This prevents speculative planner behavior from becoming unmeasured policy.

## CI policy

`mapping_conformance` is an ordinary Rust integration test, so the existing workspace test workflow runs it automatically.

**Stage 1** established the deterministic fixture corpus and machine-readable metric baseline.

**Stage 2** is now active for full-corpus runs: fixture behavior remains an exact regression gate and the reviewed aggregate baseline additionally enforces monotonic safety invariants. CI also publishes the current JSON summary so changes can be compared during review without introducing percentage targets that have no statistical basis yet.

Potential later stages can add richer per-mode baselines, target-schema/execution metrics for executable fixtures, and statistically meaningful thresholds once the corpus is large and representative enough to justify them.
