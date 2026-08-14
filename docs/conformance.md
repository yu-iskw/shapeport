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

The initial corpus covers:

- exact and normalized names;
- aliases and schema-evolution renames;
- compatible and incompatible type behavior;
- decimal preservation;
- timestamp preservation;
- nullability and missing required fields;
- default-bearing fields;
- enum-domain preservation;
- arrays;
- flattening safety;
- nested object construction safety;
- ambiguous mappings and adversarial collisions;
- real integration-shaped Flint, MCP, and warehouse contracts.

The purpose of a family is to pin safe planner behavior even when synthesis for that operation does not exist yet. For example, the flattening and nested-construction fixtures currently require ambiguity instead of allowing a plausible structural guess. Likewise, the default fixture verifies safe omission of an optional default-bearing target; it does not claim that the planner synthesizes literal defaults yet.

When a follow-up capability adds structural planning such as nested object construction, flattening, enum-domain conversion, timestamp parsing, defaults, or array explosion, update the relevant fixture in the same change that implements the behavior. Execution-level fixtures can additionally carry input and expected output once the planner synthesizes the required operation.

## Metrics

The runner prints a JSON summary containing:

- `mappingPrecision`;
- `mappingRecall`;
- `exactPlanSuccessRate`;
- `unsafeAutoMapRate`;
- `ambiguityRecall`;
- `falseAmbiguityRate`;
- raw mapping and ambiguity counts.

No arbitrary global percentage threshold is enforced yet. At this stage every declarative fixture is an exact behavioral assertion, while aggregate metrics establish a baseline. Once the corpus is sufficiently representative, CI can add monotonic regression gates, especially for `unsafeAutoMapRate`.

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

`mapping_conformance` is an ordinary Rust integration test, so the existing workspace test workflow runs it automatically. This is **Stage 1** of the issue's CI strategy: malformed fixtures, planner crashes, and fixture expectation regressions fail CI, while the aggregate metrics are reported as a baseline.

A future Stage 2 should persist/review the baseline and fail monotonic safety regressions, with unsafe automatic mappings weighted more severely than unresolved cases.
