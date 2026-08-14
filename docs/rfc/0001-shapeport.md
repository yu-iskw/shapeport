# RFC 0001: ShapePort — Schema-Driven Data Transformation Runtime

| Field | Value |
| --- | --- |
| **Status** | Accepted |
| **Authors** | ShapePort contributors |
| **Last Updated** | 2026-08-14 |
| **RFC Number** | 0001 |
| **Target** | v0.1 implementation contract |
| **Repository** | `shapeport` |
| **Primary language** | Rust (edition 2024) |
| **License** | Apache-2.0 |
| **Audience** | maintainers, contributors, implementers, security reviewers |

This document is the **normative implementation contract** for ShapePort v0.1. Where this RFC conflicts with earlier drafts, this document wins. Requirements use RFC 2119 language: **MUST**, **MUST NOT**, **SHOULD**, **SHOULD NOT**, **MAY**.

---

## Revision history

| Date | Change |
| --- | --- |
| 2026-08-14 | Initial draft (architecture proposal; dual DataFusion/document backends; nine-crate layout; open path/SQL/auth questions). |
| 2026-08-14 | **Accepted revision** after adversarial architecture review. Locked decisions R1–R18 below are now normative. Fake numeric decision-matrix scores removed. Crate layout reduced to three crates. Single document-VM execution backend. MCP tools renamed and fully contracted for MCP specification 2026-07-28 + `rmcp` 3.x. |

### Adversarial findings closed (normative lock)

| ID | Finding | Lock |
| --- | --- | --- |
| R1 | JSONPath in plans is underspecified and unsafe | FieldPath only: `field`, `field.sub`, `field[N]`. No `$`, no filters, no recursive descent. Plans MUST NOT store JSONPath. |
| R2 | YAML-as-canonical plan is ambiguous | Canonical plan serialization is JSON UTF-8. YAML is CLI convenience that round-trips through the JSON model. Plan JSON Schema envelope is documented. |
| R3 | Silent hybrid DataFusion/document execution | Single execution backend for this release: **document VM**. Columnar sources decode to record values then execute. No silent hybrid. `execution.backend` MAY exist later; default is `document`. |
| R4 | Result key order / decimal JSON encoding undefined | Object key order is insertion/plan order; JSON sinks emit plan field order; decimal as string in JSON; `decimal→float64` requires `policy: lossy`. |
| R5 | Non-determinism risks | Record order = input order unless `sort`; `sort` requires explicit keys + nulls policy (`nulls: last` default); no non-deterministic functions in default registry. |
| R6 | MCP tool names and schemas incomplete | Underscore namespaced tools with required `inputSchema` and `outputSchema`. |
| R7 | Ambient `file://` in MCP results | Content-addressed `shapeport-artifact://<sha256-hex>`; no ambient `file://` unless client capability `localFilesystem` is true. |
| R8 | HTTP auth/bind underspecified | Default bind `127.0.0.1`; loopback MAY run without auth; non-loopback REQUIRES Bearer token; Origin validated (mismatch → 403). |
| R9 | DataFusion SQL filesystem table functions bypass policy | Default query is bounded SQL subset over in-memory records (`sqlparser`). DataFusion is a future optional `QueryBackend`. |
| R10 | Plan op surface too large for v0.1 | Ops: `project`, `rename`, `drop`, `literal`, `cast`, `coalesce`, `object`, `map`, `filter`, `sort`, `explode`. Defer `join`/`aggregate` in plan IR (use `query` SQL). |
| R11 | CSV nested output policies invite silent loss | CSV nested output policy: `error` only in this release. |
| R12 | Protobuf auto-detect is unsafe | Protobuf deferred (not in this release). When added: descriptor required; one message per file; no auto-detect. |
| R13 | Inference can destroy identifiers | Conservative never promotes leading-zero numerics; empty CSV field → null iff `""` is a null spelling else string; mid-stream type conflict in conservative → fail. |
| R14 | Flint demo lossy float cast | Flint demo uses string or numeric JSON for revenue; float only with explicit `policy: lossy`. |
| R15 | Schema fingerprint undefined | SHA-256 over canonical JSON of schema AST excluding `metadata`. Format `sha256:<hex>`. |
| R16 | Self-scoring 96-everywhere matrix | Replaced with qualitative tradeoff table. |
| R17 | Normative plan still used `$` paths | One normative plan example using FieldPath only. |
| R18 | Clippy pedantic + complexity 10 | Design constraint: small functions, option structs, visitors/tables. |

---

## 1. Executive summary

ShapePort is a deterministic schema-driven data transformation runtime, CLI, and Model Context Protocol (MCP) server. It bridges incompatible data contracts without requiring humans or agents to generate ad hoc Python, JavaScript, shell, or `jq` programs.

Core flow:

```text
source data + source schema + target schema
                    |
                    v
             mapping planner
                    |
                    v
       typed Transformation Plan (JSON IR)
                    |
                    v
          document VM executor
                    |
                    v
        target-schema-valid data
```

ShapePort MUST:

- accept explicit schemas and infer schemas when absent;
- support formats listed in §6;
- compile mappings into a versioned Transformation Plan IR;
- execute plans deterministically on a **document VM**;
- expose the same application services through CLI and MCP;
- target MCP specification **2026-07-28** via official Rust SDK **`rmcp` 3.x** (published 3.1.2 at acceptance), remaining compatible with **2025-11-25** clients where the SDK supports dual protocol negotiation.

ShapePort MUST NOT:

- use generated `jq`, SQL, Python, JavaScript, or LLM-generated code as canonical execution semantics;
- silently hybridize columnar engines with the document VM in this release;
- expose legacy HTTP+SSE (2024-11-05) transport;
- auto-fetch remote JSON Schema `$ref` targets;
- execute arbitrary user code.

### 1.1 Architecture (v0.1)

```text
                  +-----------------------+
                  |      Interfaces       |
                  | CLI | MCP stdio | HTTP|
                  +-----------+-----------+
                              |
                              v
                  +-----------------------+
                  |   Application Layer   |
                  | inspect/schema/plan/  |
                  | transform/validate/   |
                  | query/convert/serve   |
                  +-----------+-----------+
                              |
              +---------------+----------------+
              |                                |
              v                                v
   +----------------------+          +----------------------+
   | Schema / Planner     |          | Format adapters      |
   | normalize / infer    |          | JSON/JSONL/YAML/CSV/ |
   | match / explain      |          | TSV/Parquet/Arrow IPC|
   +----------+-----------+          +----------+-----------+
              |                                 |
              v                                 v
        +---------------------------------------------+
        |     Transformation Plan IR (JSON UTF-8)     |
        +---------------------+-----------------------+
                              |
                              v
                    +------------------+
                    |   Document VM    |
                    | record Value tree|
                    +--------+---------+
                              |
                              v
                    validation + encoding
```

Columnar sources (CSV/JSONL/Parquet/Arrow IPC) MUST decode into ShapePort `Value` records (or streams of records) before plan execution. There is no silent DataFusion path in v0.1.

### 1.2 Non-goals (v0.1)

ShapePort v0.1 is NOT:

- a general-purpose programming language;
- a replacement for dbt, Spark, DuckDB, DataFusion, `jq`, or `yq`;
- a distributed warehouse or ETL orchestrator;
- an LLM-only semantic mapper;
- a schema registry, data catalog, or workflow scheduler;
- an arbitrary-code execution environment;
- a Protobuf/Avro/OpenAPI runtime (those are deferred).

---

## 2. Goals

### 2.1 Functional goals

ShapePort SHALL:

1. Inspect input data and identify its format.
2. Read explicitly supplied input schemas.
3. Infer schemas from data when schemas are unavailable (conservative by default).
4. Normalize heterogeneous schema systems into the Core Schema model.
5. Accept a target schema or structural contract.
6. Automatically propose source-to-target mappings in `strict` or `smart` modes.
7. Report ambiguous mappings instead of silently inventing semantics.
8. Compile accepted mappings into a deterministic Transformation Plan.
9. Validate plans before execution.
10. Execute plans on the document VM without arbitrary code execution.
11. Validate resulting data against the target contract.
12. Support streaming record workloads with bounded memory where feasible.
13. Query explicitly registered in-memory sources with a bounded SQL subset.
14. Support stdin/stdout Unix pipelines.
15. Support MCP over stdio and **stateless** Streamable HTTP.
16. Provide machine-readable diagnostics, provenance, statistics, and warnings.
17. Persist and reuse transformation plans as JSON.

### 2.2 Agent goals

An agent SHOULD be able to run `inspect → schema → plan → transform` (or `validate`/`query`/`convert`) without generating transformation source code. Planner output MUST be compact, structured, explainable, and reusable.

### 2.3 Engineering goals (R18)

Implementation MUST keep `cargo clippy --workspace --all-targets --all-features -- -D warnings` clean under workspace pedantic/cargo lints, with cognitive complexity ≤ 10. Design accordingly: small functions, option structs, visitor/table-driven dispatch—not large match monoliths.

---

## 3. Design principles

1. **Plans, not scripts.** The Transformation Plan is the durable artifact. Exporters MAY exist later; they MUST NOT define canonical semantics in v0.1.
2. **Determinism by default.** Same input + plan + config + function registry version → same ordered output.
3. **Explicit ambiguity.** Competing high-scoring candidates MUST surface as `ambiguous`, not silent picks.
4. **Loss awareness.** Lossy casts and representationally lossy sinks require explicit policy.
5. **Small MCP surface.** Tools express user intent; they do not expose every internal operator.
6. **Secure by construction.** Plans MUST NOT provide shell execution, dynamic native loading, arbitrary filesystem access, or arbitrary network access.
7. **Core independence from MCP.** `shapeport-core` MUST NOT depend on `rmcp`. MCP MUST NOT define plan semantics.

---

## 4. Qualitative tradeoff table (R16)

Scores-as-proof are forbidden. The following is a qualitative comparison of candidate architectures against ShapePort requirements:

| Approach | Schema→schema planning | Nested documents | Columnar formats | Deterministic reusable plans | Agent/MCP ergonomics | Safety / policy surface | Fit for v0.1 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Generated `jq` | Weak (external DSL) | Strong | Weak (forces JSON) | Weak | Medium | Medium | Poor as canonical IR |
| JSONata | Weak–medium | Strong | Weak | Medium | Medium | Medium | Poor as canonical IR |
| DuckDB / SQL-only | Weak for nested targets | Awkward | Strong | Medium (SQL text) | Medium | Needs strict source policy | Good for query, not plan IR |
| DataFusion SQL-only | Weak for nested targets | Awkward | Strong | Medium | Medium | Needs strict source policy | Future optional query backend |
| **ShapePort Plan IR + document VM** | **Primary strength** | **Native** | Decode-then-execute | **Native** | **Native** | **Controllable** | **Selected** |

**Selected architecture for v0.1:** ShapePort Core Schema + planner + versioned Plan IR + single document VM. Apache Arrow crates MAY be used as **format codecs** for Parquet/Arrow IPC. Apache DataFusion MUST NOT be required in the default build; it MAY appear later as an optional `QueryBackend`.

Falsification trigger: if production evidence shows nearly all workloads are tiny JSON-to-JSON transforms and columnar/file workloads are marginal, revisit a document-DSL-centered design.

---

## 5. Crate layout (MUST)

Do **not** use a nine-crate split. The workspace MUST contain exactly these product crates for v0.1:

```text
shapeport/
├── Cargo.toml
├── crates/
│   ├── shapeport-core/     # schema, value, path, diagnostics, fingerprint,
│   │                       # plan IR, planner, engine, formats, query,
│   │                       # app services, security
│   ├── shapeport-mcp/      # rmcp 3.x server (stdio + streamable HTTP),
│   │                       # tools, artifacts, auth
│   └── shapeport-cli/      # clap binary `shapeport`
├── tests/
│   ├── fixtures/
│   ├── golden/
│   └── e2e/
├── docs/
└── examples/
```

### 5.1 Dependency rules

| Crate | MAY depend on | MUST NOT depend on |
| --- | --- | --- |
| `shapeport-core` | serde, format codecs, sqlparser, tracing, etc. | `rmcp`, CLI frameworks as public API |
| `shapeport-mcp` | `shapeport-core`, `rmcp` 3.x, tokio, axum/hyper as needed by rmcp | plan semantics definitions of its own |
| `shapeport-cli` | `shapeport-core`, `shapeport-mcp` (for `serve`), clap | reimplementing core logic |

MCP tool handlers MUST call core application services. Plan IR types, validation, and execution semantics live only in `shapeport-core`.

### 5.2 Internal module sketch (`shapeport-core`)

Modules MAY be organized as:

```text
schema/, value/, path/, diagnostics/, fingerprint/,
plan/, planner/, engine/, formats/, query/, app/, security/
```

This is an internal layout guideline, not a requirement to publish separate crates.

---

## 6. In-scope and out-of-scope

### 6.1 In-scope for v0.1

| Area | Requirement |
| --- | --- |
| CLI | `inspect`, `schema`, `plan`, `transform`, `validate`, `query`, `convert`, `serve` |
| Formats | JSON, JSONL, YAML (**data-mode only**), CSV, TSV, Parquet, Arrow IPC |
| Schemas | JSON Schema 2020-12 practical subset; Arrow/Parquet-derived schemas; inferred schemas |
| Planner | Modes `strict`, `smart` only (no semantic/LLM provider) |
| Explain | Human and machine explain output for plans |
| Security | Filesystem roots + resource limits |
| Telemetry | `tracing` spans; OpenTelemetry exporter optional |
| Fixtures | Flint-shaped golden fixture |
| MCP | stdio + stateless Streamable HTTP as specified in §16 |

### 6.2 Out of scope (explicit)

- YAML comment/anchor/tag round-trip preservation
- Distributed execution
- Remote URL fetch / object stores
- Python/JS plugins or arbitrary user code
- External `$ref` fetch
- LLM / semantic planner provider
- UI / workflow scheduler
- Protobuf, Avro, OpenAPI
- DataFusion as default query/execution backend
- `jq` / SQL / JSONata exporters as canonical semantics
- Plan IR `join` and `aggregate` ops (joins/aggs via `query` SQL only in v0.1)
- Legacy HTTP+SSE transport (2024-11-05)

---

## 7. Core Schema model

### 7.1 Requirements

The internal schema model SHALL:

- represent scalar and nested types;
- preserve nullability;
- represent records, lists, maps, and unions;
- represent decimal precision/scale;
- represent timestamp units and optional timezone;
- preserve source-specific annotations in `metadata` without making them core semantics;
- support stable serialization and fingerprinting (R15);
- distinguish `unknown` from `any`.

### 7.2 Conceptual type system

Normative conceptual model (not Rust code):

```text
Schema = { root: Type, metadata: Metadata }

Type =
    Null
  | Bool
  | Int { bits, signed }
  | Float { bits }
  | Decimal { precision, scale }
  | String
  | Binary
  | Date
  | Time { unit }
  | Timestamp { unit, timezone? }
  | Duration { unit }
  | Record { fields: [Field...] }   # field order is significant
  | List { element: Type, element_nullable }
  | Map { key: Type, value: Type, value_nullable }
  | Union { variants: [Type...] }
  | Unknown
  | Any

Field = { name, type, nullable, aliases?, semantic?, metadata? }
```

Field order in `Record.fields` is schema field order and MUST be preserved through fingerprinting and JSON object emission when that schema governs output.

### 7.3 Schema adapters (v0.1)

| Adapter | Status |
| --- | --- |
| JSON Schema 2020-12 (practical subset) | Required |
| Arrow Schema | Required (for Parquet/Arrow IPC) |
| Parquet Schema | Required |
| Inferred delimited-record schema | Required |
| Inferred JSON/JSONL schema | Required |
| Protobuf descriptors | Deferred (R12) |
| Avro / OpenAPI | Out of scope |

### 7.4 JSON Schema support (practical subset)

v0.1 MUST support:

- `type`, `properties`, `required`, `items`, `prefixItems`
- `enum`, `const`
- numeric/string bounds
- `format` annotations (informational unless explicitly enforced by validation options)
- `$defs` and **local** `$ref` only
- `oneOf` / `anyOf` / `allOf`
- nullable unions
- `additionalProperties`

External `$ref` fetching MUST NOT happen implicitly. Remote resolution MAY be added later only behind an explicit allowlisted resolver.

### 7.5 Schema fingerprints (R15)

Every normalized schema MUST have a fingerprint:

```text
sha256:<hex>
```

Computation:

1. Build the schema AST.
2. Exclude all `metadata` objects/maps from the AST walk.
3. Serialize to **canonical JSON**:
   - UTF-8;
   - object keys sorted lexicographically **except** that `Record.fields` arrays preserve schema field order (fields are an ordered array, not a map);
   - no insignificant whitespace;
   - decimals/numbers in canonical JSON number form only where the AST uses JSON numbers (type tags remain strings/enums as defined by the schema serialization codec).
4. SHA-256 hash the canonical JSON bytes.
5. Emit `sha256:` + lowercase hex.

Fingerprints enable plan cache keys, compatibility checks, provenance, and stale-plan detection.

---

## 8. FieldPath (R1)

### 8.1 Grammar

Plans and diagnostics MUST use **ShapePort FieldPath** only.

```abnf
field-path     = segment *(connector segment)
connector      = "." / index
segment        = ident
ident          = ALPHA / "_" *( ALPHA / DIGIT / "_" )
index          = "[" nonneg-int "]"
nonneg-int     = "0" / ( %x31-39 *DIGIT )
```

Examples of **valid** paths:

- `period`
- `product_family`
- `customer.id`
- `items[0]`
- `items[0].sku`
- `address.city`

Examples of **invalid** paths (MUST be rejected by plan validation):

- `$`
- `$.period`
- `$[*]`
- `..revenue`
- `items[?(@.price>0)]`
- `['odd-key']` (non-ident keys require a future extension; not in v0.1)

### 8.2 Semantics

- Paths are relative to the **current record root** unless an operation defines another base.
- `.` selects a record field by exact name.
- `[N]` selects a 0-based list element; out-of-range yields null under read, and is an error under strict write policies.
- Missing fields yield null when read in expressions, unless an operation specifies otherwise.
- Plans MUST NOT store JSONPath strings. Any input that looks like JSONPath MUST fail validation with a clear diagnostic.

### 8.3 Non-ident field names

v0.1 schemas and plans SHOULD restrict field names to `ident` above. If a source contains non-ident keys, `inspect`/`schema` MUST still report them, but `smart`/`strict` planners MAY leave them unmapped and surface a warning until a quoted-key path extension exists.

---

## 9. Format detection

### 9.1 Pipeline

```text
explicit --input-format
        |
        v
    authoritative
        |
        +---- otherwise ----+
                           |
                           v
                    magic/signature
                           |
                           v
                    file extension
                           |
                           v
                    bounded probe
                           |
                           v
                  candidate parsers
                           |
                           v
                  confidence ranking
```

Explicit configuration MUST override inference.

### 9.2 Ambiguous YAML/JSON

When no explicit format is supplied and strict JSON parsing succeeds, ShapePort SHOULD prefer JSON over YAML.

### 9.3 Binary formats

Parquet and Arrow IPC SHOULD use format-specific signatures/probes. Protobuf MUST NOT be auto-detected (and is out of scope for v0.1).

---

## 10. Schema inference (R13)

### 10.1 Modes

| Mode | Behavior |
| --- | --- |
| `none` | Delimited fields remain strings unless explicit schema exists |
| `conservative` (default) | Infer only when evidence is strong |
| `aggressive` | Broader heuristics for timestamps/dates/numerics/booleans |

### 10.2 Conservative rules (normative)

1. Values with **leading zeroes** that are otherwise numeric MUST remain `string` (never promoted to integer/decimal/float).
2. Empty CSV/TSV field:
   - if `""` is listed in configured null spellings → `null`;
   - otherwise → empty string.
3. Mid-stream type conflict under `conservative` → **fail** the inference/transform with a diagnostic (MUST NOT silently widen to string/any).
4. `aggressive` MAY widen per documented lattice, but MUST still record evidence and MUST NOT promote leading-zero numerics unless a future explicit option enables it (default remains: do not promote).

### 10.3 Sampling options

CLI/MCP MAY accept `sampleRows`, `sampleBytes`, `sampleSeed`. Streams MUST bound inference buffers and replay the inference window into execution when needed.

---

## 11. Transformation Plan IR

### 11.1 Requirements

The IR MUST be typed, deterministic, serializable, versioned, validated before execution, independent of CLI/MCP transport, safe to inspect, and small enough to audit.

### 11.2 Canonical serialization (R2)

- Canonical on-disk / on-wire plan form: **JSON UTF-8**.
- YAML MAY be accepted by the CLI as convenience input/output.
- YAML MUST round-trip through the JSON model (data-mode). Comments/anchors are not preserved.
- Implementations MUST ship / document a plan JSON Schema envelope conceptually matching §11.5.

### 11.3 Operation set for this release (R10)

| Op | In v0.1 plan IR? | Notes |
| --- | --- | --- |
| `project` | YES | Select fields |
| `rename` | YES | Rename fields |
| `drop` | YES | Remove fields |
| `literal` | YES | Constant value |
| `cast` | YES | Type conversion with policy |
| `coalesce` | YES | First non-null |
| `object` | YES | Construct object (plan field order) |
| `map` | YES | Record field mapping (not collection map/fn) |
| `filter` | YES | Retain records |
| `sort` | YES | Requires explicit keys + nulls policy |
| `explode` | YES | Array → records; empty array → zero records |
| `join` | NO | Use `query` SQL |
| `aggregate` | NO | Use `query` SQL |
| `flatten` | NO | Deferred |
| `array` construction as top-level op | NO | Use `object`/`map`/literals as needed; list values MAY appear inside expressions as literals |

### 11.4 Expression model

Expressions MAY include:

- `field`: FieldPath
- `literal`: Value
- `cast`: `{ expr, target, policy }`
- `coalesce`: `[expr...]`
- `call`: `{ function, args }` where `function` is in the registry
- `object`: ordered `[ { name, expr } ... ]` or ordered map preserving insertion order

### 11.5 Plan envelope (normative shape)

Conceptual JSON Schema (2020-12) for the plan document:

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://shapeport.dev/schemas/transformation-plan-v1alpha1.json",
  "title": "ShapePortTransformationPlan",
  "type": "object",
  "required": ["apiVersion", "kind", "operations"],
  "additionalProperties": false,
  "properties": {
    "apiVersion": {
      "type": "string",
      "const": "shapeport.dev/v1alpha1"
    },
    "kind": {
      "type": "string",
      "const": "TransformationPlan"
    },
    "metadata": {
      "type": "object",
      "additionalProperties": true,
      "properties": {
        "name": { "type": "string" },
        "generatedBy": {
          "type": "object",
          "additionalProperties": true,
          "properties": {
            "mode": { "type": "string", "enum": ["strict", "smart", "manual"] },
            "shapeportVersion": { "type": "string" }
          }
        }
      }
    },
    "contracts": {
      "type": "object",
      "additionalProperties": false,
      "properties": {
        "input": {
          "type": "object",
          "properties": {
            "fingerprint": {
              "type": "string",
              "pattern": "^sha256:[0-9a-f]{64}$"
            }
          }
        },
        "output": {
          "type": "object",
          "properties": {
            "fingerprint": {
              "type": "string",
              "pattern": "^sha256:[0-9a-f]{64}$"
            }
          }
        }
      }
    },
    "operations": {
      "type": "array",
      "minItems": 1,
      "items": { "$ref": "#/$defs/operation" }
    },
    "validation": {
      "type": "object",
      "properties": {
        "output": { "type": "string", "enum": ["required", "optional", "none"] }
      }
    },
    "execution": {
      "type": "object",
      "additionalProperties": false,
      "properties": {
        "errorPolicy": { "type": "string", "enum": ["fail", "skip", "collect"] },
        "backend": { "type": "string", "enum": ["document"] }
      }
    }
  },
  "$defs": {
    "fieldPath": {
      "type": "string",
      "pattern": "^[A-Za-z_][A-Za-z0-9_]*(\\.[A-Za-z_][A-Za-z0-9_]*|\\[[0-9]+\\])*$"
    },
    "castPolicy": {
      "type": "string",
      "enum": ["strict", "lossy", "try"]
    },
    "expr": {
      "type": "object",
      "oneOf": [
        {
          "required": ["field"],
          "additionalProperties": false,
          "properties": { "field": { "$ref": "#/$defs/fieldPath" } }
        },
        {
          "required": ["literal"],
          "additionalProperties": false,
          "properties": { "literal": true }
        },
        {
          "required": ["cast"],
          "additionalProperties": false,
          "properties": {
            "cast": {
              "type": "object",
              "required": ["expr", "target", "policy"],
              "additionalProperties": false,
              "properties": {
                "expr": { "$ref": "#/$defs/expr" },
                "target": { "type": "string" },
                "policy": { "$ref": "#/$defs/castPolicy" }
              }
            }
          }
        },
        {
          "required": ["coalesce"],
          "additionalProperties": false,
          "properties": {
            "coalesce": {
              "type": "array",
              "items": { "$ref": "#/$defs/expr" },
              "minItems": 1
            }
          }
        },
        {
          "required": ["call"],
          "additionalProperties": false,
          "properties": {
            "call": {
              "type": "object",
              "required": ["function", "args"],
              "properties": {
                "function": { "type": "string" },
                "args": {
                  "type": "array",
                  "items": { "$ref": "#/$defs/expr" }
                }
              }
            }
          }
        },
        {
          "required": ["object"],
          "additionalProperties": false,
          "properties": {
            "object": {
              "type": "object",
              "description": "Keys are emitted in object insertion order.",
              "additionalProperties": { "$ref": "#/$defs/expr" }
            }
          }
        }
      ]
    },
    "operation": {
      "type": "object",
      "oneOf": [
        {
          "required": ["project"],
          "additionalProperties": false,
          "properties": {
            "project": {
              "type": "object",
              "required": ["fields"],
              "properties": {
                "fields": {
                  "type": "array",
                  "items": { "$ref": "#/$defs/fieldPath" },
                  "minItems": 1
                }
              }
            }
          }
        },
        {
          "required": ["rename"],
          "additionalProperties": false,
          "properties": {
            "rename": {
              "type": "object",
              "required": ["mapping"],
              "properties": {
                "mapping": {
                  "type": "object",
                  "additionalProperties": { "type": "string" }
                }
              }
            }
          }
        },
        {
          "required": ["drop"],
          "additionalProperties": false,
          "properties": {
            "drop": {
              "type": "object",
              "required": ["fields"],
              "properties": {
                "fields": {
                  "type": "array",
                  "items": { "$ref": "#/$defs/fieldPath" }
                }
              }
            }
          }
        },
        {
          "required": ["literal"],
          "additionalProperties": false,
          "properties": {
            "literal": {
              "type": "object",
              "required": ["field", "value"],
              "properties": {
                "field": { "type": "string" },
                "value": true
              }
            }
          }
        },
        {
          "required": ["cast"],
          "additionalProperties": false,
          "properties": {
            "cast": {
              "type": "object",
              "required": ["field", "target", "policy"],
              "properties": {
                "field": { "$ref": "#/$defs/fieldPath" },
                "target": { "type": "string" },
                "policy": { "$ref": "#/$defs/castPolicy" }
              }
            }
          }
        },
        {
          "required": ["coalesce"],
          "additionalProperties": false,
          "properties": {
            "coalesce": {
              "type": "object",
              "required": ["field", "values"],
              "properties": {
                "field": { "type": "string" },
                "values": {
                  "type": "array",
                  "items": { "$ref": "#/$defs/expr" }
                }
              }
            }
          }
        },
        {
          "required": ["object"],
          "additionalProperties": false,
          "properties": {
            "object": {
              "type": "object",
              "required": ["fields"],
              "properties": {
                "fields": {
                  "type": "object",
                  "additionalProperties": { "$ref": "#/$defs/expr" }
                }
              }
            }
          }
        },
        {
          "required": ["map"],
          "additionalProperties": false,
          "properties": {
            "map": {
              "type": "object",
              "required": ["fields"],
              "description": "Record field mapping; output key order is plan insertion order.",
              "properties": {
                "fields": {
                  "type": "object",
                  "additionalProperties": { "$ref": "#/$defs/expr" }
                }
              }
            }
          }
        },
        {
          "required": ["filter"],
          "additionalProperties": false,
          "properties": {
            "filter": {
              "type": "object",
              "required": ["predicate"],
              "properties": {
                "predicate": { "$ref": "#/$defs/expr" }
              }
            }
          }
        },
        {
          "required": ["sort"],
          "additionalProperties": false,
          "properties": {
            "sort": {
              "type": "object",
              "required": ["keys"],
              "properties": {
                "keys": {
                  "type": "array",
                  "minItems": 1,
                  "items": {
                    "type": "object",
                    "required": ["field"],
                    "properties": {
                      "field": { "$ref": "#/$defs/fieldPath" },
                      "order": {
                        "type": "string",
                        "enum": ["asc", "desc"],
                        "default": "asc"
                      },
                      "nulls": {
                        "type": "string",
                        "enum": ["first", "last"],
                        "default": "last"
                      }
                    }
                  }
                }
              }
            }
          }
        },
        {
          "required": ["explode"],
          "additionalProperties": false,
          "properties": {
            "explode": {
              "type": "object",
              "required": ["field"],
              "properties": {
                "field": { "$ref": "#/$defs/fieldPath" },
                "to": { "type": "string" },
                "outer": {
                  "type": "boolean",
                  "default": false,
                  "description": "If true, null array yields one record with null element; empty array still yields zero records."
                }
              }
            }
          }
        }
      ]
    }
  }
}
```

Implementations MAY refine `$defs` into tighter typed targets for `cast.target` (core type names). Unknown operation keys MUST fail validation.

### 11.6 Normative plan example (R17, R14)

Warehouse-shaped records → Flint-shaped records. Paths are FieldPath only (no `$`). Revenue remains a decimal/string unless an explicit lossy cast is requested.

```json
{
  "apiVersion": "shapeport.dev/v1alpha1",
  "kind": "TransformationPlan",
  "metadata": {
    "name": "warehouse-to-flint",
    "generatedBy": {
      "mode": "smart",
      "shapeportVersion": "0.1.0"
    }
  },
  "contracts": {
    "input": { "fingerprint": "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef" },
    "output": { "fingerprint": "sha256:fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210" }
  },
  "operations": [
    {
      "map": {
        "fields": {
          "month": { "field": "period" },
          "product": { "field": "product_family" },
          "revenue": { "field": "total_sales_usd" }
        }
      }
    }
  ],
  "validation": { "output": "required" },
  "execution": {
    "errorPolicy": "fail",
    "backend": "document"
  }
}
```

If the target schema requires JSON number `float64` for `revenue`, the plan MUST use an explicit lossy cast:

```json
{
  "map": {
    "fields": {
      "month": { "field": "period" },
      "product": { "field": "product_family" },
      "revenue": {
        "cast": {
          "expr": { "field": "total_sales_usd" },
          "target": "float64",
          "policy": "lossy"
        }
      }
    }
  }
}
```

`policy: strict` MUST NOT be used for `decimal→float64` (R4, R14).

### 11.7 Sort semantics (R5, R10)

- Without `sort`, output record order MUST equal input record order (after filters/explodes expand/contract the stream in encounter order).
- `sort` MUST include at least one key.
- Each key MUST specify `field`; `order` defaults to `asc`; `nulls` defaults to `last`.
- Sort MUST be deterministic for equal keys (stable relative to prior order).

### 11.8 Explode semantics (R10)

- `explode.field` MUST resolve to an array (or null).
- Non-array non-null → error under default `fail` policy.
- Empty array → **zero** output records for that input record.
- Null array + `outer: false` (default) → zero records.
- Null array + `outer: true` → one record with null element value.
- Relative order of exploded elements MUST follow array order; relative order across parent records MUST follow input order.

### 11.9 Plan validation

Validation SHALL include:

- envelope/version/kind;
- operation shapes;
- FieldPath syntax;
- referenced fields against input schema when available;
- expression type checking;
- function existence/signatures;
- target field completeness when output contract present;
- impossible casts;
- forbidden ops (`join`, `aggregate`, JSONPath, non-deterministic calls);
- resource policy constraints;
- `execution.backend` if present MUST be `document` in v0.1.

---

## 12. Mapping planner

### 12.1 Modes (v0.1)

| Mode | Behavior |
| --- | --- |
| `strict` | High-confidence structural matches and explicitly allowed casts only |
| `smart` | Adds deterministic normalization/heuristics (case folding style, aliases, safe coercions, path similarity, stats) |

Semantic/LLM provider mode is **out of scope** for v0.1.

### 12.2 Pipeline

```text
source schema
     |
     v
candidate discovery
     |
     v
constraint solving
     |
     v
ranked mapping(s) or ambiguous
     |
     v
plan synthesis
     |
     v
static validation
```

### 12.3 Ambiguity

If two candidates remain plausible for one required target field and neither dominates under configured thresholds, the planner MUST return `status: "ambiguous"` with candidates. MCP MUST return this as structured data. CLI MAY prompt in interactive mode.

### 12.4 Explain

`plan --explain` / MCP explain flag MUST describe, per target field:

- selected source (or unresolved);
- reasons/evidence;
- synthesized ops;
- distinction between observed evidence and inferred belief.

---

## 13. Execution architecture (R3)

### 13.1 Single backend: document VM

v0.1 MUST execute all Transformation Plans on the **document VM**.

```text
source bytes
    |
    v
format decoder  --->  stream of Value records / document Values
    |
    v
plan interpreter (document VM)
    |
    v
validation
    |
    v
format encoder
```

- CSV/TSV/JSONL/Parquet/Arrow IPC MUST decode to record `Value`s (struct-like objects) before ops run.
- Nested JSON/YAML documents execute directly as `Value` trees.
- Implementations MUST NOT silently route subsets of a plan through DataFusion in this release.
- A future `execution.backend` discriminator MAY select alternate backends; if the field is present in v0.1 it MUST be `"document"`.

### 13.2 Value model

Conceptual:

```text
Value =
    Null
  | Bool
  | Int
  | UInt
  | Float
  | Decimal
  | String
  | Binary
  | Date | Timestamp | ...
  | Array([Value...])
  | Object(ordered map name -> Value)   # insertion order matters (R4)
```

Object key order MUST be preserved as insertion/plan order through the VM.

### 13.3 Result canonicalization (R4)

| Concern | Rule |
| --- | --- |
| Object key order | Insertion/plan order |
| JSON / JSONL sinks | Emit objects with plan field order for mapped outputs |
| Decimal in JSON | Encode as **string** |
| `decimal → float64` | Requires `policy: lossy`; `strict` is an error |
| Binary in JSON | Base64 string unless format options specify otherwise |

### 13.4 Determinism (R5)

For fixed input bytes, plan, configuration, and function registry version:

- record order is input order unless `sort` is present;
- default registry MUST contain only deterministic functions;
- plans referencing non-deterministic functions MUST fail validation unless an explicit future policy enables them (not in v0.1 default).

### 13.5 Streaming and memory

Record sources SHOULD stream. Ops that require full materialization (`sort` without external spill support) MUST document memory behavior and honor configured limits (fail with exit code 10 / MCP tool error when exceeded).

---

## 14. Function registry

### 14.1 Principles

- Functions MUST be explicitly registered.
- No `eval`, shell, or dynamic source execution.
- Default registry MUST be deterministic only.

### 14.2 Initial functions

**String:** `lower`, `upper`, `trim`, `concat`, `replace`, `regex_extract`, `substring`

**Numeric:** `abs`, `round`, `floor`, `ceil`

**Temporal:** `parse_date`, `parse_timestamp`, `format_timestamp`, `date_trunc` (timezone conversion only with explicit semantics)

**General:** `length`, `is_null`, `null_if`, `coalesce`

Regex features MUST enforce size/time limits from the security policy.

---

## 15. Format adapters

### 15.1 JSON

Support single values, arrays of records, and nested documents. Large top-level arrays SHOULD stream where practical.

### 15.2 JSONL / NDJSON

Malformed-record policy: `fail` (default), `skip`, `collect`.

### 15.3 CSV / TSV (R11)

Options MUST include delimiter, header/no-header, quote, escape, null spellings, encoding policy, inference mode, sample size.

**Nested output policy for this release:** `error` only. Implementations MUST NOT offer `flatten` or `json-stringify` CSV nested policies in v0.1.

### 15.4 YAML

**Data-mode only.** Comments, anchors, aliases, tags, and formatting need not be preserved. This MUST be documented in CLI help.

### 15.5 Parquet / Arrow IPC

Use Apache Arrow Rust ecosystem crates as codecs. Preserve logical types, decimals, timestamps, nested lists/structs when decoding into `Value` and when encoding back, subject to ShapePort type coverage. Projection pushdown MAY be applied at the codec layer for efficiency, but execution semantics remain the document VM’s.

### 15.6 Protobuf (R12)

Deferred. When added in a later release: descriptor required; one message type per file; no auto-detect; framing policy explicit for streams.

---

## 16. Query engine (R9)

### 16.1 Purpose

`query` reduces/aggregates datasets before optional planning/conversion. Joins and aggregations in v0.1 happen here—not in the Plan IR.

### 16.2 Default engine

Default query engine MUST be a **bounded SQL subset** evaluated over **explicitly registered in-memory record tables**, parsed with `sqlparser`.

Apache DataFusion is a **future** optional `QueryBackend` and MUST NOT be required in the default build.

### 16.3 Source registration

- SQL MUST run only against explicitly registered sources (CLI `--source name=path` / MCP `sources` map, or a single default `input`).
- DataFusion-style filesystem table functions and arbitrary path SQL MUST NOT be available in the default engine.
- Globs MAY expand at registration time under filesystem root policy, producing registered tables—not open-ended SQL path access.

### 16.4 Bounded SQL subset (v0.1)

MUST support at least:

- `SELECT` with column refs, aliases, basic scalar expressions
- `WHERE`
- `GROUP BY` + aggregations: `COUNT`, `SUM`, `AVG`, `MIN`, `MAX`
- `ORDER BY`
- `LIMIT`
- `INNER JOIN` / `LEFT JOIN` on equality predicates over registered tables

MUST reject:

- DDL/DML
- subqueries beyond a documented minimal set (if unsupported, reject clearly)
- UDFs
- file/table functions
- network/external scanners

### 16.5 Composition

`query` results MAY feed `plan`/`transform` via composition of app services (CLI sugar allowed).

---

## 17. MCP server (R6, R7, R8)

### 17.1 Protocol and SDK requirements

| Item | Requirement |
| --- | --- |
| Spec | MCP **2026-07-28** (major revision) |
| SDK | Official Rust SDK **`rmcp` 3.x** (current published at acceptance: **3.1.2**) from https://github.com/modelcontextprotocol/rust-sdk |
| Compatibility | Remain compatible with **2025-11-25** clients via SDK dual-support / negotiation where available |
| Transports | **stdio** + **stateless Streamable HTTP** (SEP-2567) |
| Schemas | JSON Schema **2020-12** for tool `inputSchema` and `outputSchema` via **`schemars`** |
| `outputSchema` | **REQUIRED on every tool** (SEP-2106: any JSON Schema root type allowed) |
| Routing headers | `Mcp-Method` / `Mcp-Name` (SEP-2243) — handled by `rmcp` |
| Legacy HTTP+SSE | **MUST NOT** implement 2024-11-05 HTTP+SSE |
| Handler model | Fresh handler per HTTP request; shared state via `Clone` handle |
| Errors | Tool-level errors vs protocol errors per `rmcp` guidance |

Stateless Streamable HTTP means no reliance on protocol-level sessions / sticky `Mcp-Session-Id` as a required server mode for 2026-07-28. Cross-request state MUST use explicit artifact handles or equivalent tool arguments, not ambient sessions.

### 17.2 Tool names (R6)

Exact tool names (underscore, namespaced):

| Tool | Purpose |
| --- | --- |
| `shapeport_inspect` | Detect format, schema, stats |
| `shapeport_schema` | Infer/convert schema |
| `shapeport_plan` | Build/explain plan |
| `shapeport_transform` | Execute plan / plan+transform |
| `shapeport_validate` | Validate data or plan |
| `shapeport_query` | Bounded SQL over registered sources |
| `shapeport_convert` | Representation conversion |

Every tool MUST declare both `inputSchema` and `outputSchema`.

### 17.3 Common schema fragments

Shared definitions used below:

```json
{
  "SourceRef": {
    "type": "object",
    "required": ["uri"],
    "additionalProperties": false,
    "properties": {
      "uri": {
        "type": "string",
        "description": "file:// under readRoots, inline: data URI, or shapeport-artifact:// hash"
      },
      "format": {
        "type": "string",
        "enum": ["json", "jsonl", "yaml", "csv", "tsv", "parquet", "arrow-ipc"]
      },
      "inline": {
        "description": "Optional small inline payload instead of fetching uri",
        "type": ["string", "object", "array", "number", "boolean", "null"]
      }
    }
  },
  "Diagnostic": {
    "type": "object",
    "required": ["severity", "code", "message"],
    "properties": {
      "severity": { "type": "string", "enum": ["error", "warning", "info"] },
      "code": { "type": "string" },
      "message": { "type": "string" },
      "path": { "type": "string" },
      "hint": { "type": "string" }
    }
  },
  "ArtifactRef": {
    "type": "object",
    "required": ["uri", "format", "bytes", "sha256"],
    "additionalProperties": false,
    "properties": {
      "uri": {
        "type": "string",
        "pattern": "^shapeport-artifact://[0-9a-f]{64}$"
      },
      "format": { "type": "string" },
      "bytes": { "type": "integer", "minimum": 0 },
      "rows": { "type": "integer", "minimum": 0 },
      "sha256": { "type": "string", "pattern": "^[0-9a-f]{64}$" },
      "schemaFingerprint": {
        "type": "string",
        "pattern": "^sha256:[0-9a-f]{64}$"
      },
      "expiresAt": { "type": "string", "format": "date-time" },
      "localPath": {
        "type": "string",
        "description": "Present only if client capability localFilesystem is true"
      }
    }
  }
}
```

### 17.4 Tool contracts

Schemas below are normative intent for `schemars`-generated contracts. Field names use camelCase in MCP JSON.

#### 17.4.1 `shapeport_inspect`

**inputSchema** (root `object`):

```json
{
  "type": "object",
  "required": ["source"],
  "additionalProperties": false,
  "properties": {
    "source": { "$ref": "#/$defs/SourceRef" },
    "options": {
      "type": "object",
      "additionalProperties": false,
      "properties": {
        "sampleRows": { "type": "integer", "minimum": 0 },
        "inferTypes": {
          "type": "string",
          "enum": ["none", "conservative", "aggressive"],
          "default": "conservative"
        }
      }
    }
  }
}
```

**outputSchema**:

```json
{
  "type": "object",
  "required": ["format", "schema"],
  "properties": {
    "format": {
      "type": "object",
      "required": ["name", "confidence"],
      "properties": {
        "name": { "type": "string" },
        "confidence": { "type": "number" },
        "evidence": { "type": "array", "items": { "type": "string" } }
      }
    },
    "schema": { "type": "object" },
    "schemaFingerprint": {
      "type": "string",
      "pattern": "^sha256:[0-9a-f]{64}$"
    },
    "statistics": { "type": "object" },
    "sample": true,
    "diagnostics": { "type": "array", "items": { "$ref": "#/$defs/Diagnostic" } }
  }
}
```

#### 17.4.2 `shapeport_schema`

**inputSchema**:

```json
{
  "type": "object",
  "additionalProperties": false,
  "properties": {
    "source": { "$ref": "#/$defs/SourceRef" },
    "schema": { "type": "object", "description": "Explicit schema to convert" },
    "as": {
      "type": "string",
      "enum": ["shapeport", "json-schema", "arrow-schema"],
      "default": "shapeport"
    },
    "inferTypes": {
      "type": "string",
      "enum": ["none", "conservative", "aggressive"],
      "default": "conservative"
    },
    "sampleRows": { "type": "integer", "minimum": 0 }
  },
  "anyOf": [
    { "required": ["source"] },
    { "required": ["schema"] }
  ]
}
```

**outputSchema**:

```json
{
  "type": "object",
  "required": ["schema", "fingerprint"],
  "properties": {
    "schema": { "type": "object" },
    "fingerprint": {
      "type": "string",
      "pattern": "^sha256:[0-9a-f]{64}$"
    },
    "dialect": { "type": "string" },
    "evidence": { "type": "object" },
    "diagnostics": { "type": "array", "items": { "$ref": "#/$defs/Diagnostic" } }
  }
}
```

#### 17.4.3 `shapeport_plan`

**inputSchema**:

```json
{
  "type": "object",
  "required": ["targetSchema"],
  "additionalProperties": false,
  "properties": {
    "sourceSchema": { "type": "object" },
    "source": { "$ref": "#/$defs/SourceRef" },
    "targetSchema": { "type": "object" },
    "mode": { "type": "string", "enum": ["strict", "smart"], "default": "smart" },
    "explain": { "type": "boolean", "default": false }
  },
  "anyOf": [
    { "required": ["sourceSchema", "targetSchema"] },
    { "required": ["source", "targetSchema"] }
  ]
}
```

**outputSchema**:

```json
{
  "type": "object",
  "required": ["status"],
  "properties": {
    "status": {
      "type": "string",
      "enum": ["ready", "ambiguous", "error"]
    },
    "plan": { "type": "object" },
    "explanation": { "type": "array", "items": { "type": "object" } },
    "unresolved": { "type": "array", "items": { "type": "object" } },
    "diagnostics": { "type": "array", "items": { "$ref": "#/$defs/Diagnostic" } }
  }
}
```

#### 17.4.4 `shapeport_transform`

**inputSchema**:

```json
{
  "type": "object",
  "required": ["source"],
  "additionalProperties": false,
  "properties": {
    "source": { "$ref": "#/$defs/SourceRef" },
    "plan": { "type": "object" },
    "targetSchema": { "type": "object" },
    "mode": { "type": "string", "enum": ["strict", "smart"], "default": "smart" },
    "outputFormat": {
      "type": "string",
      "enum": ["json", "jsonl", "yaml", "csv", "tsv", "parquet", "arrow-ipc"]
    },
    "errorPolicy": {
      "type": "string",
      "enum": ["fail", "skip", "collect"],
      "default": "fail"
    }
  },
  "anyOf": [
    { "required": ["source", "plan"] },
    { "required": ["source", "targetSchema"] }
  ]
}
```

**outputSchema**:

```json
{
  "type": "object",
  "required": ["status"],
  "properties": {
    "status": { "type": "string", "enum": ["ok", "error"] },
    "result": true,
    "artifact": { "$ref": "#/$defs/ArtifactRef" },
    "receipt": { "type": "object" },
    "diagnostics": { "type": "array", "items": { "$ref": "#/$defs/Diagnostic" } }
  }
}
```

When inline thresholds are exceeded, `result` MUST be omitted or null and `artifact` MUST be present.

#### 17.4.5 `shapeport_validate`

**inputSchema**:

```json
{
  "type": "object",
  "additionalProperties": false,
  "properties": {
    "source": { "$ref": "#/$defs/SourceRef" },
    "schema": { "type": "object" },
    "plan": { "type": "object" }
  },
  "anyOf": [
    { "required": ["source", "schema"] },
    { "required": ["plan"] }
  ]
}
```

**outputSchema**:

```json
{
  "type": "object",
  "required": ["valid"],
  "properties": {
    "valid": { "type": "boolean" },
    "errors": {
      "type": "array",
      "items": { "$ref": "#/$defs/Diagnostic" }
    },
    "diagnostics": {
      "type": "array",
      "items": { "$ref": "#/$defs/Diagnostic" }
    }
  }
}
```

#### 17.4.6 `shapeport_query`

**inputSchema**:

```json
{
  "type": "object",
  "required": ["sql"],
  "additionalProperties": false,
  "properties": {
    "sql": { "type": "string" },
    "sources": {
      "type": "object",
      "additionalProperties": { "$ref": "#/$defs/SourceRef" },
      "description": "Explicit name -> source map; required for multi-table SQL"
    },
    "source": {
      "$ref": "#/$defs/SourceRef",
      "description": "Single source registered as input"
    },
    "outputFormat": {
      "type": "string",
      "enum": ["json", "jsonl", "yaml", "csv", "tsv", "parquet", "arrow-ipc"]
    }
  }
}
```

**outputSchema**: same shape as `shapeport_transform` outputSchema (`status`, `result`, `artifact`, `receipt`, `diagnostics`).

#### 17.4.7 `shapeport_convert`

**inputSchema**:

```json
{
  "type": "object",
  "required": ["source", "to"],
  "additionalProperties": false,
  "properties": {
    "source": { "$ref": "#/$defs/SourceRef" },
    "to": {
      "type": "string",
      "enum": ["json", "jsonl", "yaml", "csv", "tsv", "parquet", "arrow-ipc"]
    }
  }
}
```

**outputSchema**: same shape as `shapeport_transform` outputSchema.

### 17.5 Artifacts (R7)

- Artifact URIs MUST use content addressing: `shapeport-artifact://<sha256-hex>`.
- MCP results MUST NOT return ambient `file://` URIs unless the client declared capability `localFilesystem: true`.
- When `localFilesystem` is true, an optional `localPath` MAY be included, and MUST resolve only inside the server **write root**.
- Artifacts MUST enforce TTL and max bytes from configuration.
- Local mapping of artifact digests to files is an internal server concern.

Default inline thresholds (configurable):

```yaml
mcp:
  inlineResult:
    maxBytes: 1048576
    maxRows: 1000
  artifacts:
    maxBytes: 1073741824
    ttlSeconds: 3600
```

### 17.6 HTTP auth and bind (R8)

| Rule | Requirement |
| --- | --- |
| Default bind | `127.0.0.1` (loopback) |
| Loopback | MAY run without auth |
| Non-loopback bind | REQUIRES `Authorization: Bearer <token>` where token comes from env `SHAPEPORT_MCP_TOKEN` or equivalent config |
| Missing/invalid token on non-loopback | MUST deny (401) |
| `Origin` header | MUST be validated against configured allowlist; mismatch → **403** |
| Protocol | Stateless Streamable HTTP only; no legacy HTTP+SSE |

### 17.7 stdio rules

- stdout MUST contain protocol messages only.
- Logs go to stderr.
- No progress bars on stdout.

### 17.8 Tool errors vs protocol errors

- Invalid JSON-RPC / transport / auth / origin failures → protocol/HTTP errors.
- Domain failures (parse, schema, ambiguity, transform, limits) → tool results with `isError` / structured error content per `rmcp` guidance, including diagnostics in `structuredContent` when possible.
- Do not collapse domain validation failures into transport failures.

### 17.9 Handler lifecycle

For Streamable HTTP, construct a **fresh handler per request**. Share configuration, artifact store handles, and engine handles via a `Clone` state object. Do not store per-client session maps as a required path for 2026-07-28.

---

## 18. CLI

### 18.1 Synopsis

```text
shapeport <COMMAND>

Commands:
  inspect     Inspect format, schema, statistics, and capabilities
  schema      Infer or convert a schema
  plan        Build a Transformation Plan
  transform   Transform data using a plan or target schema
  validate    Validate data or plans
  query       Query one or more explicitly registered sources
  convert     Convert representation with minimal reshaping
  serve       Start the MCP server (stdio or streamable-http)
```

### 18.2 Command sketches

```bash
shapeport inspect data.parquet --output json

shapeport schema customers.csv \
  --infer-types conservative \
  --sample-rows 10000 \
  --output schema.json

shapeport plan \
  --input-schema source.schema.json \
  --output-schema target.schema.json \
  --mode smart \
  --output plan.json \
  --explain

shapeport transform customers.parquet \
  --plan customer-to-chart.json \
  --output customers.jsonl

shapeport validate output.json --schema target.schema.json

shapeport query \
  --source sales='sales/*.parquet' \
  --source customers='customers.csv' \
  --sql-file report.sql \
  --output-format jsonl

shapeport convert input.jsonl --to parquet

shapeport serve --transport stdio

shapeport serve \
  --transport streamable-http \
  --bind 127.0.0.1:8787
```

CLI MAY accept plan YAML and MUST canonicalize to the JSON model internally. Machine-readable outputs SHOULD default to JSON for scripts.

### 18.3 Exit codes

| Code | Meaning |
| --- | --- |
| 0 | Success |
| 2 | CLI usage/configuration error |
| 3 | Input format/parse error |
| 4 | Schema error |
| 5 | Planning ambiguity |
| 6 | Plan validation error |
| 7 | Transformation error |
| 8 | Target validation error |
| 9 | Security/policy denial |
| 10 | Resource limit exceeded |
| 11 | I/O error |
| 12 | Internal error |

These assignments are stable for v0.1+.

---

## 19. Validation

Layers:

```text
source parse
    |
source schema conformance (optional/required by flags)
    |
plan type validation
    |
target schema conformance
```

Error policy: `fail` (default), `skip`, `collect`. Rejected records under `collect` SHOULD be writable to a rejects sink with error metadata and without logging secrets by default.

---

## 20. Security model

### 20.1 Threat model

Attacker-controlled inputs may include data files, schemas, plans, regexes, SQL, URIs, and MCP requests. Risks include path traversal, SSRF, decompression bombs, deep recursion, catastrophic regex, memory/CPU exhaustion, SQL access to unintended files, and artifact exfiltration.

### 20.2 Filesystem policy

Server/MCP mode MUST use explicit roots:

```yaml
security:
  filesystem:
    readRoots:
      - /data
    writeRoots:
      - /tmp/shapeport
```

Paths MUST be canonicalized before checks. Symlink escapes MUST be denied.

### 20.3 URI policy

Default:

- `file:` only under configured roots
- `shapeport-artifact:` for server-managed artifacts
- `http` / `https` / object-store schemes: disabled

### 20.4 SQL policy

Only explicitly registered sources. No filesystem table functions in the default engine.

### 20.5 Resource limits

Configurable at least: max input/output bytes, max schema/nesting depth, max columns, max field-name length, max regex size/time, max execution duration, max memory, max inline MCP result, max artifact bytes/TTL.

### 20.6 Secrets and logging

Do not log raw records by default. Redact values in diagnostics unless debug/sample is explicit.

---

## 21. Observability

Use the `tracing` crate. OpenTelemetry exporter is optional.

Suggested spans:

```text
shapeport.request
  shapeport.detect
  shapeport.infer_schema
  shapeport.plan
    shapeport.match
    shapeport.solve
    shapeport.validate_plan
  shapeport.execute
    shapeport.scan
    shapeport.document
    shapeport.encode
  shapeport.validate_output
  shapeport.query
```

Avoid high-cardinality schema/path metric labels. stdio MCP MUST log only to stderr.

---

## 22. Configuration

Example:

```yaml
version: 1

runtime:
  batchRows: 8192
  tempDir: /tmp/shapeport
  maxMemoryBytes: 2147483648

inference:
  mode: conservative
  sampleRows: 10000

planner:
  mode: smart

security:
  filesystem:
    readRoots:
      - /data
    writeRoots:
      - /tmp/shapeport
  network:
    enabled: false
  limits:
    maxInputBytes: 10737418240
    maxOutputBytes: 10737418240
    maxSchemaDepth: 64
    maxNestingDepth: 128
    timeoutSeconds: 300

mcp:
  bind: 127.0.0.1:8787
  tokenEnv: SHAPEPORT_MCP_TOKEN
  allowedOrigins:
    - http://127.0.0.1:8787
  inlineResult:
    maxBytes: 1048576
    maxRows: 1000
  artifacts:
    maxBytes: 1073741824
    ttlSeconds: 3600
```

CLI flags override config. Environment variables MAY override deployment options (`SHAPEPORT_MCP_TOKEN`, bind, roots).

---

## 23. Diagnostics contract

Diagnostics MUST be structured with at least: `severity`, `code`, `message`, optional FieldPath, optional source location, optional hint. CLI renders them for humans; MCP returns structured forms.

---

## 24. Versioning

Independently versioned surfaces:

1. CLI
2. Rust library (`shapeport-core`)
3. Transformation Plan API (`apiVersion`)
4. Configuration schema
5. MCP tool schemas

Plan compatibility deserves the strongest stability. Breaking plan semantics require a new `apiVersion`.

Primary implementation: **Rust edition 2024**, license **Apache-2.0**.

---

## 25. Flint golden fixture (R14)

Fixture input:

```json
[
  {
    "period": "2026-01",
    "product_family": "Compute",
    "total_sales_usd": "12340.20"
  }
]
```

Preferred expected output (revenue as string/decimal JSON string):

```json
[
  {
    "month": "2026-01",
    "product": "Compute",
    "revenue": "12340.20"
  }
]
```

If a Flint target schema requires a JSON number, the golden plan MUST show `policy: lossy` for float conversion; tests MUST NOT imply `strict` decimal→float is valid.

---

## 26. Testing strategy

### 26.1 Required unit coverage

Type coercion, FieldPath parse/eval, functions, schema conversion, fingerprint stability, planner matching, plan validation, CSV inference edge cases (leading zeros, empty fields, mid-stream conflicts).

### 26.2 Golden tests

Persist input schema, target schema, expected plan JSON, expected explanation fragments, Flint fixture.

### 26.3 Property tests

Encode/decode round trips where formats support them; identity plan preserves values and key order; plan JSON round trip; equivalent schemas → stable fingerprints.

### 26.4 End-to-end scenarios

1. CSV → inferred schema → target JSON
2. JSONL → plan → Parquet
3. Parquet → SQL aggregate (`query`) → Flint-compatible JSON
4. Nested JSON → target JSON Schema
5. MCP stdio inspect/plan/transform
6. MCP Streamable HTTP inspect/plan/transform
7. Ambiguous mapping response
8. Rejected-record collection
9. Resource-limit enforcement
10. Non-loopback HTTP without token denied; Origin mismatch → 403

### 26.5 Fuzzing

Path parser, plan deserialization, schema normalization, CSV dialect detection, JSON/YAML adapter boundaries.

---

## 27. Acceptance criteria for v0.1

v0.1 is successful when all of the following hold:

1. `shapeport inspect` detects and describes JSON, JSONL, CSV, TSV, YAML, Parquet, Arrow IPC.
2. `shapeport schema` infers CSV/JSONL schemas under conservative rules (R13).
3. JSON Schema target contracts normalize through the Core Schema model.
4. `shapeport plan` maps common renames and safe casts in `strict`/`smart`.
5. Ambiguous mappings are reported (`exit 5` / MCP `ambiguous`).
6. Plans serialize canonically as JSON; YAML CLI paths round-trip through JSON.
7. FieldPath-only plans validate; JSONPath-like strings fail validation.
8. `shapeport transform` executes the same plan over CSV, JSONL, and Parquet for logically equivalent record data via the document VM.
9. No DataFusion dependency is required in the default build.
10. Output target validation works.
11. No arbitrary code execution exists.
12. Flint-shaped golden fixture passes with correct decimal/lossy policy.
13. MCP tools `shapeport_*` each expose `inputSchema` and `outputSchema`.
14. MCP stdio works; Streamable HTTP is stateless; legacy HTTP+SSE absent.
15. Artifacts use `shapeport-artifact://`; no ambient `file://` without `localFilesystem`.
16. HTTP security rules (R8) are enforced.
17. `query` only sees explicit sources; bounded SQL subset works for join/aggregate needs.
18. `make lint && make test` pass under workspace Clippy pedantic + complexity constraints.

---

## 28. Implementation phases (still within v0.1 contract)

Phasing is scheduling guidance; semantics above remain normative.

### Phase A — Core IR + document VM

Core schema, FieldPath, plan IR JSON, document VM ops (`project`/`rename`/`drop`/`literal`/`cast`/`coalesce`/`object`/`map`/`filter`/`sort`/`explode`), JSON/JSONL/CSV, fingerprints, conservative inference.

### Phase B — Formats + planner + CLI

Parquet/Arrow IPC/YAML codecs, strict/smart planner, explain, `inspect`/`schema`/`plan`/`transform`/`validate`/`convert`, Flint golden.

### Phase C — Query + MCP + security

Bounded SQL `query`, MCP `rmcp` 3.x stdio + Streamable HTTP, artifacts, auth/origin, roots/limits, tracing.

---

## 29. Recommended initial technical decisions

| Decision | Selection |
| --- | --- |
| Language | Rust edition 2024 |
| License | Apache-2.0 |
| Execution backend | Document VM only |
| Columnar libraries | Arrow/Parquet as codecs (optional convenience) |
| Query engine | `sqlparser` + in-memory bounded evaluator |
| DataFusion | Future optional `QueryBackend` only |
| Canonical plan | JSON UTF-8 (`shapeport.dev/v1alpha1`) |
| Path language | FieldPath (R1) |
| Target schema priority | JSON Schema 2020-12 practical subset |
| CLI | `clap` |
| Async | Tokio where needed (MCP/HTTP) |
| Telemetry | `tracing`; OTel exporter optional |
| MCP SDK | `rmcp` 3.x |
| MCP schemas | `schemars` JSON Schema 2020-12 |
| Default inference | conservative |
| Default mapping mode | smart |
| Default error policy | fail |
| Remote schema fetch | disabled |
| Arbitrary code | prohibited |

---

## 30. Risks and mitigations

| Risk | Impact | Mitigation |
| --- | --- | --- |
| IR grows into a language | High | Fixed op budget; no join/agg in plan; no user code |
| Auto-mapping mistakes | High | Ambiguity status; conservative defaults |
| Document VM too slow on huge columnar data | Medium | Codec projection; future optional query/exec backends; honest limits |
| MCP context blowups | High | Artifact threshold + content-addressed URIs |
| Remote MCP exposes files | Critical | Roots, auth, Origin, loopback default |
| Inference destroys identifiers | High | R13 conservative rules |
| Plan semantics drift | High | Versioned IR + golden tests |
| CSV nested silent loss | High | Nested policy `error` only |
| Fake architectural certainty | Medium | Qualitative tradeoffs only (R16) |

---

## 31. Final recommendation

Implement ShapePort as a **schema-driven interoperability runtime** centered on:

```text
Core Schema
    +
Mapping Planner (strict|smart)
    +
Versioned Transformation Plan (JSON)
    +
Document VM
```

Interfaces remain thin:

```text
                   shapeport-core
                   /            \
                  /              \
         shapeport-cli       shapeport-mcp
                            /            \
                         stdio    Streamable HTTP
```

Concentrate original engineering on safe, explainable, reusable source-contract → target-contract planning and deterministic execution. Treat mature parsers and columnar codecs as libraries—not as alternate silent semantic engines—until a future RFC explicitly adds a second backend.

---

## Appendix A — Locked decision checklist (implementer)

- [ ] R1 FieldPath only in plans
- [ ] R2 Plan canonical JSON; YAML convenience only
- [ ] R3 Document VM only; columnar decode-then-execute
- [ ] R4 Key order + decimal JSON string + lossy float policy
- [ ] R5 Deterministic order/sort/registry
- [ ] R6 Seven `shapeport_*` tools with input+output schemas
- [ ] R7 `shapeport-artifact://` + capability-gated local paths
- [ ] R8 Bind/auth/Origin rules
- [ ] R9 Bounded SQL; no DataFusion default; no FS table functions
- [ ] R10 Op allowlist including sort/explode; no join/agg in IR
- [ ] R11 CSV nested → error only
- [ ] R12 Protobuf deferred
- [ ] R13 Conservative inference rules
- [ ] R14 Flint fixture loss policy
- [ ] R15 Schema fingerprint algorithm
- [ ] R16 No numeric self-score matrix
- [ ] R17 Normative FieldPath plan example present
- [ ] R18 Clippy pedantic / complexity design discipline
- [ ] Three crates only; core ↛ rmcp

---

## Appendix B — Source references

1. MCP specification 2026-07-28 — https://modelcontextprotocol.io/specification/2026-07-28/
2. MCP 2026-07-28 release notes — https://blog.modelcontextprotocol.io/posts/2026-07-28/
3. Streamable HTTP transport — https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http
4. SEP-2567 (stateless Streamable HTTP / sessions) — MCP SEPs registry
5. SEP-2106 (JSON Schema 2020-12 tool schemas / outputSchema root types) — MCP SEPs registry
6. SEP-2243 (`Mcp-Method` / `Mcp-Name`) — https://modelcontextprotocol.io/seps/2243-http-standardization
7. Official Rust MCP SDK (`rmcp`) — https://github.com/modelcontextprotocol/rust-sdk
8. JSON Schema 2020-12 — https://json-schema.org/draft/2020-12
9. Apache Arrow Rust — https://arrow.apache.org/rust/arrow/
10. Apache Parquet Rust — https://arrow.apache.org/rust/parquet/
11. Microsoft Flint Chart — https://github.com/microsoft/flint-chart

Verify dependency versions at implementation time; this RFC pins intent to **`rmcp` 3.x** (3.1.2 current at acceptance) and MCP **2026-07-28** with **2025-11-25** client compatibility.
