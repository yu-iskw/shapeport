//! `ShapePort` CLI.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use shapeport_core::config::{InferMode, RuntimeConfig};
use shapeport_core::formats::FormatId;
use shapeport_core::plan::{ErrorPolicy, parse_plan_bytes};
use shapeport_core::{
    ConvertRequest, Error, InspectRequest, PlanRequest, PlannerMode, QueryRequest, SourceSpec,
    TransformRequest, ValidateRequest, convert_data, inspect_source, plan_mapping, query_sources,
    schema_as_json, schema_from_json_value, transform_data, validate_data, write_output,
};
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(
    name = "shapeport",
    version,
    about = "Schema-driven data transformation runtime and MCP server"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
    /// YAML configuration file.
    #[arg(long, global = true)]
    config: Option<PathBuf>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    Inspect {
        input: PathBuf,
        #[arg(long, default_value = "json")]
        output: String,
        #[arg(long)]
        input_format: Option<String>,
        #[arg(long, default_value = "conservative")]
        infer_types: String,
        #[arg(long, default_value_t = 20)]
        sample_rows: usize,
    },
    Schema {
        input: Option<PathBuf>,
        #[arg(long)]
        from_schema: Option<PathBuf>,
        #[arg(long = "as", default_value = "shapeport")]
        as_dialect: String,
        #[arg(long)]
        output: Option<PathBuf>,
        #[arg(long, default_value = "conservative")]
        infer_types: String,
        #[arg(long)]
        input_format: Option<String>,
    },
    Plan {
        input: Option<PathBuf>,
        #[arg(long)]
        input_schema: Option<PathBuf>,
        #[arg(long)]
        to_schema: Option<PathBuf>,
        #[arg(long)]
        output_schema: Option<PathBuf>,
        #[arg(long, default_value = "smart")]
        mode: String,
        #[arg(long)]
        output: Option<PathBuf>,
        #[arg(long)]
        explain: bool,
        #[arg(long)]
        input_format: Option<String>,
    },
    Transform {
        input: PathBuf,
        #[arg(long)]
        plan: Option<PathBuf>,
        #[arg(long)]
        to_schema: Option<PathBuf>,
        #[arg(long, default_value = "smart")]
        mode: String,
        #[arg(long, default_value = "json")]
        output_format: String,
        #[arg(long)]
        output: Option<PathBuf>,
        #[arg(long, default_value = "fail")]
        error_policy: String,
        #[arg(long)]
        input_format: Option<String>,
    },
    Validate {
        input: Option<PathBuf>,
        #[arg(long)]
        schema: Option<PathBuf>,
        #[arg(long)]
        plan: Option<PathBuf>,
        #[arg(long)]
        input_format: Option<String>,
    },
    Query {
        #[arg(long)]
        sql: Option<String>,
        #[arg(long)]
        sql_file: Option<PathBuf>,
        #[arg(long = "source", value_parser = parse_source_kv)]
        sources: Vec<(String, String)>,
        input: Option<PathBuf>,
        #[arg(long, default_value = "jsonl")]
        output_format: String,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    Convert {
        input: PathBuf,
        #[arg(long)]
        to: String,
        #[arg(long)]
        output: Option<PathBuf>,
        #[arg(long)]
        input_format: Option<String>,
    },
    Serve {
        #[arg(long, value_enum, default_value_t = Transport::Stdio)]
        transport: Transport,
        #[arg(long, default_value = "127.0.0.1:8787")]
        bind: String,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Transport {
    Stdio,
    StreamableHttp,
}

/// Arguments for the `plan` sub-command, extracted to keep function arity ≤ 6.
struct PlanCmd {
    input: Option<PathBuf>,
    input_schema: Option<PathBuf>,
    to_schema: Option<PathBuf>,
    mode: String,
    output: Option<PathBuf>,
    explain: bool,
    input_format: Option<String>,
}

/// Arguments for the `transform` sub-command, extracted to keep function arity ≤ 6.
struct TransformCmd {
    input: PathBuf,
    plan: Option<PathBuf>,
    to_schema: Option<PathBuf>,
    mode: String,
    output_format: String,
    output: Option<PathBuf>,
    error_policy: String,
    input_format: Option<String>,
}

/// Arguments for the `schema` sub-command, extracted to keep function arity ≤ 6.
struct SchemaCmd {
    input: Option<PathBuf>,
    from_schema: Option<PathBuf>,
    as_dialect: String,
    output: Option<PathBuf>,
    infer_types: String,
    input_format: Option<String>,
}

/// Arguments for the `query` sub-command, extracted to keep function arity ≤ 6.
struct QueryCmd {
    sql: Option<String>,
    sql_file: Option<PathBuf>,
    sources: Vec<(String, String)>,
    input: Option<PathBuf>,
    output_format: String,
    output: Option<PathBuf>,
}

fn parse_source_kv(raw: &str) -> Result<(String, String), String> {
    raw.split_once('=')
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .ok_or_else(|| "expected name=path".into())
}

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{err}");
            ExitCode::from(u8::try_from(err.kind.exit_code()).unwrap_or(12))
        }
    }
}

fn run() -> shapeport_core::Result<()> {
    let cli = Cli::parse();
    let mut config = RuntimeConfig::default().with_cwd_roots(std::env::current_dir()?);
    if let Some(path) = cli.config {
        apply_yaml_config(&path, &mut config)?;
    }
    apply_env_vars(&mut config);
    dispatch(cli.command, config)
}

fn apply_env_vars(config: &mut RuntimeConfig) {
    if let Ok(token) = std::env::var("SHAPEPORT_MCP_TOKEN")
        && !token.is_empty()
    {
        config.mcp.bearer_token = Some(token);
    }
    if let Ok(origins) = std::env::var("SHAPEPORT_MCP_ORIGIN_ALLOWLIST")
        && !origins.is_empty()
    {
        let list: Vec<String> = origins
            .split(',')
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .collect();
        config.mcp.origin_allowlist = list;
    }
}

fn apply_yaml_config(path: &Path, config: &mut RuntimeConfig) -> shapeport_core::Result<()> {
    let mut loaded = RuntimeConfig::load_yaml(path)?;
    if loaded.filesystem.read_roots.is_empty() {
        loaded
            .filesystem
            .read_roots
            .clone_from(&config.filesystem.read_roots);
    }
    if loaded.filesystem.write_roots.is_empty() {
        loaded
            .filesystem
            .write_roots
            .clone_from(&config.filesystem.write_roots);
    }
    *config = loaded;
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn dispatch(command: Commands, config: RuntimeConfig) -> shapeport_core::Result<()> {
    match command {
        Commands::Inspect {
            input,
            output,
            input_format,
            infer_types,
            sample_rows,
        } => cmd_inspect(
            &config,
            &input,
            &output,
            input_format.as_deref(),
            &infer_types,
            sample_rows,
        ),
        Commands::Schema {
            input,
            from_schema,
            as_dialect,
            output,
            infer_types,
            input_format,
        } => cmd_schema(
            &config,
            SchemaCmd {
                input,
                from_schema,
                as_dialect,
                output,
                infer_types,
                input_format,
            },
        ),
        Commands::Plan {
            input,
            input_schema,
            to_schema,
            output_schema,
            mode,
            output,
            explain,
            input_format,
        } => cmd_plan(
            &config,
            PlanCmd {
                input,
                input_schema,
                to_schema: to_schema.or(output_schema),
                mode,
                output,
                explain,
                input_format,
            },
        ),
        Commands::Transform {
            input,
            plan,
            to_schema,
            mode,
            output_format,
            output,
            error_policy,
            input_format,
        } => cmd_transform(
            &config,
            &TransformCmd {
                input,
                plan,
                to_schema,
                mode,
                output_format,
                output,
                error_policy,
                input_format,
            },
        ),
        Commands::Validate {
            input,
            schema,
            plan,
            input_format,
        } => cmd_validate(
            &config,
            input,
            schema.as_deref(),
            plan.as_deref(),
            input_format.as_deref(),
        ),
        Commands::Query {
            sql,
            sql_file,
            sources,
            input,
            output_format,
            output,
        } => cmd_query(
            &config,
            QueryCmd {
                sql,
                sql_file,
                sources,
                input,
                output_format,
                output,
            },
        ),
        Commands::Convert {
            input,
            to,
            output,
            input_format,
        } => cmd_convert(
            &config,
            &input,
            &to,
            output.as_deref(),
            input_format.as_deref(),
        ),
        Commands::Serve { transport, bind } => cmd_serve(config, transport, bind),
    }
}

fn cmd_inspect(
    config: &RuntimeConfig,
    input: &Path,
    output: &str,
    input_format: Option<&str>,
    infer_types: &str,
    sample_rows: usize,
) -> shapeport_core::Result<()> {
    let result = inspect_source(
        &InspectRequest {
            source: file_source(input, input_format),
            infer: parse_infer(infer_types)?,
            sample_rows,
        },
        config,
    )?;
    let payload = serde_json::json!({
        "format": {
            "name": result.detection.format.as_str(),
            "confidence": result.detection.confidence,
            "evidence": result.detection.evidence,
        },
        "schema": result.schema,
        "schemaFingerprint": result.fingerprint,
        "statistics": {"rows": result.row_count},
        "sample": result.sample,
    });
    print_json(&payload, output == "json")
}

fn cmd_schema(config: &RuntimeConfig, cmd: SchemaCmd) -> shapeport_core::Result<()> {
    let schema = if let Some(path) = cmd.from_schema {
        let bytes = std::fs::read(&path)?;
        schema_from_json_value(&serde_json::from_slice(&bytes)?)?
    } else {
        let input = cmd
            .input
            .ok_or_else(|| Error::usage("schema_input", "schema requires input"))?;
        inspect_source(
            &InspectRequest {
                source: file_source(&input, cmd.input_format.as_deref()),
                infer: parse_infer(&cmd.infer_types)?,
                sample_rows: 10_000,
            },
            config,
        )?
        .schema
    };
    let rendered = if cmd.as_dialect == "json-schema" {
        schema_as_json(&schema)
    } else {
        serde_json::to_value(&schema)?
    };
    emit_json(cmd.output.as_deref(), &rendered, config)
}

fn cmd_plan(config: &RuntimeConfig, cmd: PlanCmd) -> shapeport_core::Result<()> {
    let target_path = cmd
        .to_schema
        .ok_or_else(|| Error::usage("plan_target", "--to-schema / --output-schema is required"))?;
    let target = load_json_schema(&target_path)?;
    let source_schema = cmd
        .input_schema
        .as_deref()
        .map(load_json_schema)
        .transpose()?;
    let input_fmt = cmd.input_format.as_deref();
    let source = cmd.input.map(|path| file_source(&path, input_fmt));
    let planned = plan_mapping(
        &PlanRequest {
            source_schema,
            target_schema: target,
            source,
            mode: parse_mode(&cmd.mode)?,
            infer: InferMode::Conservative,
        },
        config,
    )?;
    if planned.status == "ambiguous" {
        emit_ambiguous_plan(&planned)?;
        return Err(Error::ambiguity("mapping is ambiguous"));
    }
    if cmd.explain {
        println!("{}", serde_json::to_string_pretty(&planned.explanation)?);
    }
    let plan = planned
        .plan
        .ok_or_else(|| Error::internal("ready plan missing"))?;
    emit_json(cmd.output.as_deref(), &serde_json::to_value(&plan)?, config)
}

fn emit_ambiguous_plan(planned: &shapeport_core::PlanResponse) -> shapeport_core::Result<()> {
    let payload = serde_json::json!({
        "status": "ambiguous",
        "unresolved": planned.unresolved,
        "explanation": planned.explanation,
    });
    println!("{}", serde_json::to_string_pretty(&payload)?);
    Ok(())
}

fn cmd_transform(config: &RuntimeConfig, cmd: &TransformCmd) -> shapeport_core::Result<()> {
    let plan = cmd.plan.as_deref().map(load_plan).transpose()?;
    let target_schema = cmd.to_schema.as_deref().map(load_json_schema).transpose()?;
    let format = FormatId::parse(&cmd.output_format).ok_or_else(|| {
        Error::usage(
            "output_format",
            format!("unknown format {}", cmd.output_format),
        )
    })?;
    let result = transform_data(
        &TransformRequest {
            source: file_source(&cmd.input, cmd.input_format.as_deref()),
            plan,
            target_schema,
            mode: parse_mode(&cmd.mode)?,
            output_format: format,
            error_policy: parse_policy(&cmd.error_policy),
            infer: InferMode::Conservative,
        },
        config,
    )?;
    write_transform_output(cmd.output.as_deref(), &result.bytes, config)
}

fn write_transform_output(
    output: Option<&std::path::Path>,
    bytes: &[u8],
    config: &RuntimeConfig,
) -> shapeport_core::Result<()> {
    if let Some(path) = output {
        write_output(path, bytes, config)
    } else {
        std::io::Write::write_all(&mut std::io::stdout(), bytes)?;
        Ok(())
    }
}

fn cmd_validate(
    config: &RuntimeConfig,
    input: Option<PathBuf>,
    schema: Option<&Path>,
    plan: Option<&Path>,
    input_format: Option<&str>,
) -> shapeport_core::Result<()> {
    let result = validate_data(
        &ValidateRequest {
            source: input.map(|path| file_source(&path, input_format)),
            schema: schema.map(load_json_schema).transpose()?,
            plan: plan.map(load_plan).transpose()?,
            infer: InferMode::Conservative,
        },
        config,
    )?;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "valid": result.valid,
            "errors": result.errors,
        }))?
    );
    if result.valid {
        Ok(())
    } else {
        Err(Error::target("invalid", "validation failed"))
    }
}

fn cmd_query(config: &RuntimeConfig, cmd: QueryCmd) -> shapeport_core::Result<()> {
    let sql = resolve_sql(cmd.sql, cmd.sql_file)?;
    let mut map = std::collections::HashMap::new();
    for (name, path) in cmd.sources {
        map.insert(name, file_source(Path::new(&path), None));
    }
    if let Some(input) = cmd.input {
        map.insert("input".into(), file_source(&input, None));
    }
    let format = FormatId::parse(&cmd.output_format).ok_or_else(|| {
        Error::usage(
            "output_format",
            format!("unknown format {}", cmd.output_format),
        )
    })?;
    let result = query_sources(
        &QueryRequest {
            sql,
            sources: map,
            output_format: format,
            infer: InferMode::Conservative,
        },
        config,
    )?;
    write_transform_output(cmd.output.as_deref(), &result.bytes, config)
}

fn resolve_sql(sql: Option<String>, sql_file: Option<PathBuf>) -> shapeport_core::Result<String> {
    match (sql, sql_file) {
        (Some(s), _) => Ok(s),
        (_, Some(path)) => String::from_utf8(std::fs::read(path)?)
            .map_err(|err| Error::parse("sql_file", err.to_string())),
        _ => Err(Error::usage("sql", "--sql or --sql-file is required")),
    }
}

fn cmd_convert(
    config: &RuntimeConfig,
    input: &Path,
    to: &str,
    output: Option<&Path>,
    input_format: Option<&str>,
) -> shapeport_core::Result<()> {
    let to =
        FormatId::parse(to).ok_or_else(|| Error::usage("to", format!("unknown format {to}")))?;
    let result = convert_data(
        &ConvertRequest {
            source: file_source(input, input_format),
            to,
            infer: InferMode::Conservative,
        },
        config,
    )?;
    write_transform_output(output, &result.bytes, config)
}

fn cmd_serve(
    config: RuntimeConfig,
    transport: Transport,
    bind: String,
) -> shapeport_core::Result<()> {
    let runtime = tokio::runtime::Runtime::new().map_err(|err| Error::internal(err.to_string()))?;
    runtime.block_on(async move { run_serve(config, transport, bind).await })
}

async fn run_serve(
    config: RuntimeConfig,
    transport: Transport,
    bind: String,
) -> shapeport_core::Result<()> {
    match transport {
        Transport::Stdio => shapeport_mcp::serve_stdio(config)
            .await
            .map_err(Error::internal),
        Transport::StreamableHttp => {
            let addr: SocketAddr = bind
                .parse()
                .map_err(|err: std::net::AddrParseError| Error::usage("bind", err.to_string()))?;
            shapeport_mcp::serve_http(addr, config)
                .await
                .map_err(Error::internal)
        }
    }
}

fn file_source(path: &Path, format: Option<&str>) -> SourceSpec {
    let uri = if path.as_os_str() == "-" {
        "-".into()
    } else {
        path.to_string_lossy().into_owned()
    };
    SourceSpec {
        uri: Some(uri),
        format: format.and_then(FormatId::parse),
        inline: None,
        bytes: None,
    }
}

fn load_json_schema(path: &Path) -> shapeport_core::Result<shapeport_core::Schema> {
    let bytes = std::fs::read(path)?;
    let value: serde_json::Value = if is_yaml_ext(path) {
        serde_norway::from_slice(&bytes)
            .map_err(|err| Error::parse("yaml_schema", err.to_string()))?
    } else {
        serde_json::from_slice(&bytes)?
    };
    schema_from_json_value(&value)
}

fn load_plan(path: &Path) -> shapeport_core::Result<shapeport_core::TransformationPlan> {
    let bytes = std::fs::read(path)?;
    parse_plan_bytes(&bytes, is_yaml_ext(path))
}

fn is_yaml_ext(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext == "yaml" || ext == "yml")
}

fn parse_infer(raw: &str) -> shapeport_core::Result<InferMode> {
    InferMode::parse(raw).ok_or_else(|| Error::usage("infer_types", format!("unknown mode {raw}")))
}

fn parse_mode(raw: &str) -> shapeport_core::Result<PlannerMode> {
    PlannerMode::parse(raw).ok_or_else(|| Error::usage("mode", format!("unknown mode {raw}")))
}

fn parse_policy(raw: &str) -> ErrorPolicy {
    match raw {
        "skip" => ErrorPolicy::Skip,
        "collect" => ErrorPolicy::Collect,
        _ => ErrorPolicy::Fail,
    }
}

fn emit_json(
    output: Option<&std::path::Path>,
    value: &serde_json::Value,
    config: &RuntimeConfig,
) -> shapeport_core::Result<()> {
    let bytes = serde_json::to_vec_pretty(value)?;
    if let Some(path) = output {
        write_output(path, &bytes, config)
    } else {
        println!("{}", String::from_utf8_lossy(&bytes));
        Ok(())
    }
}

fn print_json(value: &serde_json::Value, _json: bool) -> shapeport_core::Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}
