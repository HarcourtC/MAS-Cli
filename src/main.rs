mod banner;
mod commands;
mod queue_output;
mod runtime;

use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::{self, Write};
use std::path::PathBuf;

const DEFAULT_API_URL: &str = "http://127.0.0.1:36163";
const EXIT_OK: i32 = 0;
const EXIT_ERROR: i32 = 1;
const SOURCE_CLI: &str = "cli";
const SOURCE_BACKEND: &str = "backend";

#[derive(Debug, Parser)]
#[command(name = "auto-mas-cli")]
#[command(version)]
#[command(about = "AUTO-MAS CLI")]
struct Cli {
    #[arg(long, global = true, default_value = DEFAULT_API_URL)]
    api_url: String,

    #[arg(long, global = true)]
    app_root: Option<PathBuf>,

    #[arg(long, global = true)]
    python_exe: Option<PathBuf>,

    #[arg(long, global = true)]
    json: bool,

    #[arg(long, global = true)]
    no_auto_start: bool,

    #[arg(long, global = true)]
    keep_backend: bool,

    #[arg(long = "elevated", hide = true, global = true)]
    elevated: bool,

    #[command(subcommand)]
    command: RootCommand,
}

#[derive(Debug, Subcommand)]
enum RootCommand {
    Backend {
        #[command(subcommand)]
        command: BackendCommand,
    },
    Queue {
        #[command(subcommand)]
        command: QueueCommand,
    },
}

#[derive(Debug, Subcommand)]
enum BackendCommand {
    Status,
    Start,
    Stop,
}

#[derive(Debug, Subcommand)]
enum QueueCommand {
    List,
    Start {
        #[arg(long = "queue-id")]
        queue_id: String,
        #[arg(long, default_value = "AutoProxy")]
        mode: String,
    },
}

#[derive(Debug, Serialize)]
struct CliEnvelope {
    code: i32,
    status: String,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    category: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
struct BackendState {
    started_by_cli: bool,
    tracked_pid: Option<u32>,
    app_root: Option<String>,
    python_executable: Option<String>,
    api_url: Option<String>,
}

#[derive(Debug)]
struct RuntimeContext {
    api_url: String,
    app_root: Option<PathBuf>,
    python_executable: Option<PathBuf>,
    is_local_api: bool,
    state: BackendState,
}

#[derive(Debug)]
struct CliError {
    message: String,
    category: &'static str,
    source: &'static str,
    backend_payload: Option<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartOutcome {
    Ready,
    HandedOffToElevatedProcess,
}

fn cli_error(category: &'static str, message: impl Into<String>) -> CliError {
    CliError {
        message: message.into(),
        category,
        source: SOURCE_CLI,
        backend_payload: None,
    }
}

fn emit_json_envelope(envelope: CliEnvelope) {
    print_json(&serde_json::to_value(envelope).unwrap_or_else(|_| json!({})));
}

fn main() {
    let cli = Cli::parse();
    let exit_code = run(cli);
    std::process::exit(exit_code);
}

fn run(cli: Cli) -> i32 {
    if !cli.json {
        banner::print_mas_cli_banner();
    }

    let mut ctx = match build_context(&cli) {
        Ok(ctx) => ctx,
        Err(err) => {
            emit_error(&cli, &err);
            return EXIT_ERROR;
        }
    };

    let result = commands::execute(&cli, &mut ctx);

    match result {
        Ok(()) => EXIT_OK,
        Err(err) => {
            emit_error(&cli, &err);
            EXIT_ERROR
        }
    }
}

fn build_context(cli: &Cli) -> Result<RuntimeContext, CliError> {
    let state = runtime::load_backend_state().unwrap_or_default();
    let is_local_api = runtime::is_local_api_url(&cli.api_url);

    let app_root = if is_local_api {
        runtime::discover_app_root(cli.app_root.clone())?
    } else {
        None
    };

    let python_executable = if is_local_api {
        runtime::discover_python_executable(cli.python_exe.clone(), app_root.as_deref())?
    } else {
        None
    };

    Ok(RuntimeContext {
        api_url: cli.api_url.clone(),
        app_root,
        python_executable,
        is_local_api,
        state,
    })
}

fn emit_error(cli: &Cli, err: &CliError) {
    if cli.json {
        if let Some(payload) = &err.backend_payload {
            print_json(payload);
            return;
        }
        let response = CliEnvelope {
            code: 500,
            status: "error".to_string(),
            message: err.message.clone(),
            data: None,
            source: Some(err.source.to_string()),
            category: Some(err.category.to_string()),
        };
        emit_json_envelope(response);
    } else {
        let _ = writeln!(io::stderr(), "错误: {}", err.message);
    }
}

fn print_json(value: &Value) {
    if let Ok(s) = serde_json::to_string(value) {
        println!("{}", s);
    } else {
        println!(
            "{}",
            json!({"code":500,"status":"error","message":"JSON 序列化失败"})
        );
    }
}
