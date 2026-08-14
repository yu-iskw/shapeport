# ShapePort

Schema-driven data transformation runtime and MCP server for AI agents.

ShapePort inspects structured data, infers or loads schemas, builds deterministic Transformation Plans, and executes those plans — all without a query engine or DataFusion dependency.

## Crate Layout

| Crate | Type | Purpose |
|---|---|---|
| `shapeport-core` | library | Schema model, Transformation Plan IR, planner, document VM, format adapters (JSON, JSONL, YAML, CSV, TSV, Parquet, Arrow IPC), query engine, application services |
| `shapeport-mcp` | library | MCP 2026-07-28 server built on `rmcp` 3.x — stdio and stateless Streamable HTTP transports |
| `shapeport-cli` | binary (`shapeport`) | CLI wrapping core app services and the MCP server |

## CLI Commands

```bash
# Inspect format, schema, and statistics
shapeport inspect data.json

# Infer or convert a schema
shapeport schema data.csv
shapeport schema --from-schema target.schema.json --as json-schema

# Plan a mapping from source to target schema
shapeport plan --to-schema target.schema.json data.json

# Transform data using a plan or target schema
shapeport transform --to-schema target.schema.json data.json
shapeport transform --plan mapping.plan.json --output-format jsonl data.csv

# Validate data against a schema or plan
shapeport validate --schema target.schema.json data.json

# Run SQL over named sources
shapeport query --sql "SELECT * FROM sales" --source sales=data.json

# Convert format
shapeport convert --to jsonl data.json

# Serve MCP over stdio (default)
shapeport serve

# Serve MCP over Streamable HTTP
shapeport serve --transport streamable-http --bind 127.0.0.1:8787
```

### Global Flags

| Flag | Description |
|---|---|
| `--config <file.yaml>` | YAML runtime configuration |

## Flint Example

Given `fixtures/flint/input.json`:

```json
[
  {
    "period": "2026-01",
    "product_family": "Compute",
    "total_sales_usd": "12340.20"
  }
]
```

And `fixtures/flint/target.schema.json`:

```json
{
  "root": {
    "kind": "record",
    "fields": [
      { "name": "month",   "type": { "kind": "string" }, "nullable": false },
      { "name": "product", "type": { "kind": "string" }, "nullable": false },
      { "name": "revenue", "type": { "kind": "string" }, "nullable": false }
    ]
  }
}
```

Run:

```bash
shapeport transform --to-schema fixtures/flint/target.schema.json fixtures/flint/input.json
```

Output matches `fixtures/flint/expected.json`:

```json
[{"month":"2026-01","product":"Compute","revenue":"12340.20"}]
```

## MCP Protocol

ShapePort implements **MCP 2026-07-28** via **`rmcp` 3.x**.

Available tools:

- `shapeport_inspect` — format detection, schema inference, statistics, sample rows
- `shapeport_schema` — infer or convert a schema (ShapePort native or JSON Schema)
- `shapeport_plan` — build a deterministic Transformation Plan
- `shapeport_transform` — execute a plan or inline mapping
- `shapeport_validate` — validate data or a plan
- `shapeport_query` — bounded SQL over explicitly registered sources
- `shapeport_convert` — format conversion

Large results are returned as `shapeport-artifact://` URIs instead of inline JSON.

## Security

### Loopback enforcement

`serve_http` on a non-loopback address requires `SHAPEPORT_MCP_TOKEN` to be set.
Binding to `127.0.0.1` does not require a token (suitable for local agent use).

### Bearer token

Set `SHAPEPORT_MCP_TOKEN` environment variable.
All requests to a non-loopback HTTP server must carry `Authorization: Bearer <token>`.

### Origin allowlist

```bash
export SHAPEPORT_MCP_ORIGIN_ALLOWLIST="https://cursor.sh,https://my-app.example"
shapeport serve --transport streamable-http
```

Or in a config file:

```yaml
mcp:
  allowedOrigins:
    - https://cursor.sh
    - https://my-app.example
```

Requests with an `Origin` header not in the allowlist receive **403 Forbidden**.

## Configuration

```yaml
# shapeport.yaml
mcp:
  bearerToken: "my-secret"
  allowedOrigins:
    - "https://trusted.example"
```

Pass with `shapeport --config shapeport.yaml serve`.

## Development

```bash
make setup              # fetch Cargo dependencies; install cargo tools
make format             # rustfmt + Trunk repo formatters
make lint               # Trunk, cargo check, Clippy, cargo-shear, cargo-deny
make check-features     # cargo hack --each-feature across the workspace
make test               # cargo test --workspace --all-features
make coverage           # cargo-llvm-cov report
make analyze-complexity # Debtmap cyclomatic complexity analysis
make deep-analysis      # Miri + cargo-udeps (requires nightly toolchain)
make build              # release binaries and libraries
make clean              # remove build artifacts
```

See [docs/quality.md](docs/quality.md) for the full quality-assurance architecture, complexity thresholds, lint suppression policy, and tool upgrade process.

## Architecture

See `docs/adr/` for architecture decision records. Key decisions:

- **No DataFusion** in the default build — the document VM is a pure-Rust tree walker
- **MCP 2026-07-28 via rmcp 3.x** — stateless Streamable HTTP and stdio
- **FieldPath-only plan IR** — no JSONPath or JMESPath dependencies

## License

Apache-2.0
