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
    command: Option<RootCommand>,
}

#[derive(Debug, Clone, Subcommand)]
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

#[derive(Debug, Clone, Subcommand)]
enum BackendCommand {
    Status,
    Start,
    Stop,
}

#[derive(Debug, Clone, Subcommand)]
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

    let result = if let Some(command) = &cli.command {
        commands::execute(&cli, &mut ctx, command)
    } else if cli.json {
        Err(cli_error(
            "invalid_arguments",
            "缺少子命令；--json 模式下请明确传入命令",
        ))
    } else {
        run_repl(&cli, &mut ctx)
    };

    match result {
        Ok(()) => EXIT_OK,
        Err(err) => {
            emit_error(&cli, &err);
            EXIT_ERROR
        }
    }
}

fn run_repl(cli: &Cli, ctx: &mut RuntimeContext) -> Result<(), CliError> {
    let stdin = io::stdin();
    let mut line = String::new();

    loop {
        print!("mas> ");
        let _ = io::stdout().flush();

        line.clear();
        let read = stdin.read_line(&mut line).map_err(|e| {
            cli_error(
                "invalid_runtime_configuration",
                format!("读取输入失败: {e}"),
            )
        })?;
        if read == 0 {
            println!();
            break;
        }

        let input = line.trim();
        if input.is_empty() {
            continue;
        }

        match parse_repl_command(input) {
            Ok(None) => continue,
            Ok(Some(command)) => {
                if let Err(err) = commands::execute(cli, ctx, &command) {
                    emit_error(cli, &err);
                }
            }
            Err(message) => eprintln!("错误: {}", message),
        }
    }

    Ok(())
}

fn parse_repl_command(input: &str) -> Result<Option<RootCommand>, String> {
    let args: Vec<&str> = input.split_whitespace().collect();
    if args.is_empty() {
        return Ok(None);
    }

    match args[0] {
        "exit" | "quit" => {
            std::process::exit(0);
        }
        "help" | "/help" => {
            print_repl_help();
            Ok(None)
        }
        "backend" => parse_repl_backend(&args),
        "queue" => parse_repl_queue(&args),
        _ => Err("未知命令，输入 help 查看可用命令".to_string()),
    }
}

fn parse_repl_backend(args: &[&str]) -> Result<Option<RootCommand>, String> {
    if args.len() < 2 {
        return Err("用法: backend <status|start|stop>".to_string());
    }
    let command = match args[1] {
        "status" => BackendCommand::Status,
        "start" => BackendCommand::Start,
        "stop" => BackendCommand::Stop,
        _ => return Err("backend 子命令仅支持: status/start/stop".to_string()),
    };
    Ok(Some(RootCommand::Backend { command }))
}

fn parse_repl_queue(args: &[&str]) -> Result<Option<RootCommand>, String> {
    if args.len() < 2 {
        return Err("用法: queue <list|start ...>".to_string());
    }
    match args[1] {
        "list" => Ok(Some(RootCommand::Queue {
            command: QueueCommand::List,
        })),
        "start" => {
            let mut queue_id: Option<String> = None;
            let mut mode = "AutoProxy".to_string();
            let mut i = 2usize;
            while i < args.len() {
                match args[i] {
                    "--queue-id" => {
                        let value = args
                            .get(i + 1)
                            .ok_or_else(|| "queue start 缺少 --queue-id 的值".to_string())?;
                        queue_id = Some((*value).to_string());
                        i += 2;
                    }
                    "--mode" => {
                        let value = args
                            .get(i + 1)
                            .ok_or_else(|| "queue start 缺少 --mode 的值".to_string())?;
                        mode = (*value).to_string();
                        i += 2;
                    }
                    _ => {
                        if queue_id.is_none() {
                            queue_id = Some(args[i].to_string());
                            i += 1;
                        } else {
                            return Err(format!("无法识别参数: {}", args[i]));
                        }
                    }
                }
            }
            let queue_id = queue_id
                .ok_or_else(|| "用法: queue start --queue-id <id> [--mode <mode>]".to_string())?;
            Ok(Some(RootCommand::Queue {
                command: QueueCommand::Start { queue_id, mode },
            }))
        }
        _ => Err("queue 子命令仅支持: list/start".to_string()),
    }
}

fn print_repl_help() {
    println!("Tips for getting started:");
    println!("1. backend status|start|stop");
    println!("2. queue list");
    println!("3. queue start --queue-id <id> [--mode <mode>]");
    println!("4. help / exit");
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
