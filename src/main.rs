use clap::{Parser, Subcommand};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

const DEFAULT_API_URL: &str = "http://127.0.0.1:36163";
const EXIT_OK: i32 = 0;
const EXIT_ERROR: i32 = 1;

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

fn main() {
    let cli = Cli::parse();
    let exit_code = run(cli);
    std::process::exit(exit_code);
}

fn run(cli: Cli) -> i32 {
    let mut ctx = match build_context(&cli) {
        Ok(ctx) => ctx,
        Err(err) => {
            emit_error(&cli, &err);
            return EXIT_ERROR;
        }
    };

    let result = match &cli.command {
        RootCommand::Backend { command } => match command {
            BackendCommand::Status => cmd_backend_status(&cli, &ctx),
            BackendCommand::Start => cmd_backend_start(&cli, &mut ctx),
            BackendCommand::Stop => cmd_backend_stop(&cli, &mut ctx),
        },
        RootCommand::Queue { command } => match command {
            QueueCommand::List => cmd_queue_list(&cli, &mut ctx),
        },
    };

    match result {
        Ok(()) => EXIT_OK,
        Err(err) => {
            emit_error(&cli, &err);
            EXIT_ERROR
        }
    }
}

fn build_context(cli: &Cli) -> Result<RuntimeContext, CliError> {
    let state = load_backend_state().unwrap_or_default();
    let is_local_api = is_local_api_url(&cli.api_url);

    let app_root = if is_local_api {
        discover_app_root(cli.app_root.clone())?
    } else {
        None
    };

    let python_executable = if is_local_api {
        discover_python_executable(cli.python_exe.clone(), app_root.as_deref())?
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

fn cmd_backend_status(cli: &Cli, ctx: &RuntimeContext) -> Result<(), CliError> {
    let live = probe_backend(&ctx.api_url).ok();
    let ready = live.is_some();

    let data = status_data(ctx, ready);
    if cli.json {
        let message = if ready {
            "后端运行中"
        } else {
            "后端未运行"
        };
        let out = CliEnvelope {
            code: 200,
            status: "success".to_string(),
            message: message.to_string(),
            data: Some(data),
            source: None,
            category: None,
        };
        print_json(&serde_json::to_value(out).unwrap_or_else(|_| json!({})));
    } else if ready {
        println!("backend: running");
        println!("apiUrl: {}", ctx.api_url);
        if let Some(app_root) = &ctx.app_root {
            println!("appRoot: {}", app_root.display());
        }
        if let Some(python) = &ctx.python_executable {
            println!("python: {}", python.display());
        }
    } else {
        println!("backend: stopped");
        println!("message: 后端未运行");
        println!("apiUrl: {}", ctx.api_url);
    }

    Ok(())
}

fn cmd_backend_start(cli: &Cli, ctx: &mut RuntimeContext) -> Result<(), CliError> {
    if !ctx.is_local_api {
        return Err(CliError {
            message: "远端 apiUrl 不支持 backend start".to_string(),
            category: "invalid_runtime_configuration",
            source: "cli",
            backend_payload: None,
        });
    }

    if probe_backend(&ctx.api_url).is_ok() {
        emit_backend_ready(cli, ctx, ctx.state.tracked_pid);
        return Ok(());
    }

    let app_root = ctx.app_root.clone().ok_or_else(|| CliError {
        message: "无法解析应用根目录，请通过 --app-root 或 AUTO_MAS_ROOT 指定".to_string(),
        category: "invalid_runtime_configuration",
        source: "cli",
        backend_payload: None,
    })?;

    let python = ctx.python_executable.clone().ok_or_else(|| CliError {
        message: "无法解析 Python 解释器，请通过 --python-exe 或 AUTO_MAS_PYTHON 指定".to_string(),
        category: "invalid_runtime_configuration",
        source: "cli",
        backend_payload: None,
    })?;

    let mut child = Command::new(&python)
        .arg("main.py")
        .current_dir(&app_root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| CliError {
            message: format!("启动后端进程失败: {e}"),
            category: "backend_startup_failed",
            source: "cli",
            backend_payload: None,
        })?;

    let pid = Some(child.id());

    // Detach by forgetting child handle.
    let _ = child.stdin.take();
    let _ = child.stdout.take();
    let _ = child.stderr.take();

    wait_until_ready(&ctx.api_url, 30, Duration::from_millis(500))?;

    ctx.state = BackendState {
        started_by_cli: true,
        tracked_pid: pid,
        app_root: Some(app_root.display().to_string()),
        python_executable: Some(python.display().to_string()),
        api_url: Some(ctx.api_url.clone()),
    };
    let _ = save_backend_state(&ctx.state);

    emit_backend_ready(cli, ctx, pid);
    Ok(())
}

fn cmd_backend_stop(cli: &Cli, ctx: &mut RuntimeContext) -> Result<(), CliError> {
    if probe_backend(&ctx.api_url).is_err() {
        return Err(CliError {
            message: "后端未运行，无法执行命令".to_string(),
            category: "backend_unreachable",
            source: "cli",
            backend_payload: None,
        });
    }

    let body = api_post(&ctx.api_url, "/api/core/close", &json!({}))?;
    if cli.json {
        let target = if ctx.is_local_api { "local" } else { "remote" };
        let out = CliEnvelope {
            code: 200,
            status: "success".to_string(),
            message: "后端关闭请求已发送".to_string(),
            data: Some(json!({
                "requestAccepted": true,
                "target": target,
            })),
            source: None,
            category: None,
        };
        print_json(&serde_json::to_value(out).unwrap_or_else(|_| json!({})));
    } else {
        let _ = body;
        println!("后端关闭请求已发送");
        println!(
            "target: {}",
            if ctx.is_local_api { "local" } else { "remote" }
        );
        println!("apiUrl: {}", ctx.api_url);
    }

    clear_state_if_matches(&ctx.api_url);
    Ok(())
}

fn cmd_queue_list(cli: &Cli, ctx: &mut RuntimeContext) -> Result<(), CliError> {
    let mut started_temporarily = false;

    if probe_backend(&ctx.api_url).is_err() {
        if cli.no_auto_start {
            return Err(CliError {
                message: "后端未运行，无法执行命令".to_string(),
                category: "backend_unreachable",
                source: "cli",
                backend_payload: None,
            });
        }

        if !ctx.is_local_api {
            return Err(CliError {
                message: "远端后端不可达，且无法自动启动".to_string(),
                category: "backend_unreachable",
                source: "cli",
                backend_payload: None,
            });
        }

        cmd_backend_start(cli, ctx)?;
        started_temporarily = true;
    }

    let body = api_post(&ctx.api_url, "/api/queue/get", &json!({ "queueId": null }))?;

    if cli.json {
        print_json(&body);
    } else {
        print_queue_table(&body);
    }

    if started_temporarily && !cli.keep_backend {
        let _ = api_post(&ctx.api_url, "/api/core/close", &json!({}));
        clear_state_if_matches(&ctx.api_url);
    }

    Ok(())
}

fn api_post(base: &str, path: &str, body: &Value) -> Result<Value, CliError> {
    let url = format!("{}{}", base.trim_end_matches('/'), path);
    let client = Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| CliError {
            message: format!("创建 HTTP 客户端失败: {e}"),
            category: "invalid_runtime_configuration",
            source: "cli",
            backend_payload: None,
        })?;

    let resp = client.post(url).json(body).send().map_err(|_| CliError {
        message: "后端未运行，无法执行命令".to_string(),
        category: "backend_unreachable",
        source: "cli",
        backend_payload: None,
    })?;

    let status = resp.status();
    let text = resp.text().unwrap_or_default();
    let parsed: Value = serde_json::from_str(&text).unwrap_or_else(|_| {
        json!({
            "code": status.as_u16(),
            "status": if status.is_success() { "success" } else { "error" },
            "message": text,
        })
    });

    if !status.is_success() {
        return Err(error_from_backend_or_default(
            parsed,
            "backend_business_error",
        ));
    }

    if is_backend_error_payload(&parsed) {
        return Err(error_from_backend_or_default(
            parsed,
            "backend_business_error",
        ));
    }

    Ok(parsed)
}

fn probe_backend(api_url: &str) -> Result<Value, CliError> {
    api_post(api_url, "/api/info/version", &json!({}))
}

fn wait_until_ready(api_url: &str, retries: usize, delay: Duration) -> Result<(), CliError> {
    for _ in 0..retries {
        if probe_backend(api_url).is_ok() {
            return Ok(());
        }
        std::thread::sleep(delay);
    }

    Err(CliError {
        message: "后端启动超时，探活未通过".to_string(),
        category: "backend_startup_failed",
        source: "cli",
        backend_payload: None,
    })
}

fn status_data(ctx: &RuntimeContext, ready: bool) -> Value {
    let local = ctx.is_local_api;
    json!({
        "ready": ready,
        "startedByCli": if local { Some(ctx.state.started_by_cli) } else { None::<bool> },
        "trackedPid": if local { ctx.state.tracked_pid } else { None::<u32> },
        "appRoot": if local { ctx.app_root.as_ref().map(|p| p.display().to_string()) } else { None::<String> },
        "pythonExecutable": if local { ctx.python_executable.as_ref().map(|p| p.display().to_string()) } else { None::<String> },
        "apiUrl": ctx.api_url,
    })
}

fn emit_backend_ready(cli: &Cli, ctx: &RuntimeContext, pid: Option<u32>) {
    if cli.json {
        let out = CliEnvelope {
            code: 200,
            status: "success".to_string(),
            message: "后端已就绪".to_string(),
            data: Some(json!({
                "ready": true,
                "startedByCli": true,
                "trackedPid": pid,
                "appRoot": ctx.app_root.as_ref().map(|p| p.display().to_string()),
                "pythonExecutable": ctx.python_executable.as_ref().map(|p| p.display().to_string()),
                "apiUrl": ctx.api_url,
            })),
            source: None,
            category: None,
        };
        print_json(&serde_json::to_value(out).unwrap_or_else(|_| json!({})));
    } else {
        println!("backend: running");
        println!("apiUrl: {}", ctx.api_url);
        if let Some(app_root) = &ctx.app_root {
            println!("appRoot: {}", app_root.display());
        }
        if let Some(py) = &ctx.python_executable {
            println!("python: {}", py.display());
        }
        if let Some(p) = pid {
            println!("pid: {}", p);
        }
    }
}

fn print_queue_table(value: &Value) {
    let mut printed = false;

    if let Some(data) = value.get("data") {
        if let Some(arr) = data.get("list").and_then(|v| v.as_array()) {
            println!("queueId\tname");
            for item in arr {
                let qid = item.get("queueId").and_then(Value::as_str).unwrap_or("-");
                let name = item
                    .get("queueName")
                    .or_else(|| item.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or("-");
                println!("{}\t{}", qid, name);
            }
            printed = true;
        }
    }

    if !printed {
        if let Some(arr) = value.as_array() {
            println!("queueId\tname");
            for item in arr {
                let qid = item.get("queueId").and_then(Value::as_str).unwrap_or("-");
                let name = item
                    .get("queueName")
                    .or_else(|| item.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or("-");
                println!("{}\t{}", qid, name);
            }
        } else {
            println!("queueId\tname");
        }
    }
}

fn error_from_backend_or_default(value: Value, fallback_category: &'static str) -> CliError {
    let message = value
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("后端返回错误")
        .to_string();

    CliError {
        message,
        category: fallback_category,
        source: "backend",
        backend_payload: Some(value),
    }
}

fn is_backend_error_payload(value: &Value) -> bool {
    let status_is_error = value
        .get("status")
        .and_then(Value::as_str)
        .map(|s| s.eq_ignore_ascii_case("error"))
        .unwrap_or(false);

    let code = value.get("code").and_then(Value::as_i64).unwrap_or(200);
    status_is_error || code >= 400
}

fn emit_error(cli: &Cli, err: &CliError) {
    if cli.json {
        if let Some(payload) = &err.backend_payload {
            print_json(payload);
            return;
        }
        let out = CliEnvelope {
            code: 500,
            status: "error".to_string(),
            message: err.message.clone(),
            data: None,
            source: Some(err.source.to_string()),
            category: Some(err.category.to_string()),
        };
        print_json(&serde_json::to_value(out).unwrap_or_else(|_| json!({})));
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

fn discover_app_root(flag: Option<PathBuf>) -> Result<Option<PathBuf>, CliError> {
    if let Some(path) = flag {
        return validate_app_root(path).map(Some);
    }

    if let Ok(env_root) = env::var("AUTO_MAS_ROOT") {
        return validate_app_root(PathBuf::from(env_root)).map(Some);
    }

    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Ok(cwd) = env::current_dir() {
        candidates.push(cwd);
    }

    if let Ok(current_exe) = env::current_exe() {
        if let Some(parent) = current_exe.parent() {
            candidates.push(parent.to_path_buf());
            if let Some(pp) = parent.parent() {
                candidates.push(pp.to_path_buf());
            }
        }
    }

    for candidate in candidates {
        if is_valid_app_root(&candidate) {
            return Ok(Some(candidate));
        }
    }

    Ok(None)
}

fn discover_python_executable(
    flag: Option<PathBuf>,
    app_root: Option<&Path>,
) -> Result<Option<PathBuf>, CliError> {
    if let Some(path) = flag {
        return Ok(Some(path));
    }

    if let Ok(env_python) = env::var("AUTO_MAS_PYTHON") {
        return Ok(Some(PathBuf::from(env_python)));
    }

    if let Some(root) = app_root {
        let embedded = root.join("environment").join("python");
        if embedded.exists() {
            return Ok(Some(embedded));
        }
    }

    if let Some(path) = find_command_in_path("python3") {
        return Ok(Some(path));
    }

    if let Some(path) = find_command_in_path("python") {
        return Ok(Some(path));
    }

    Ok(None)
}

fn find_command_in_path(cmd: &str) -> Option<PathBuf> {
    let output = Command::new("which").arg(cmd).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if path.is_empty() {
        return None;
    }
    Some(PathBuf::from(path))
}

fn validate_app_root(path: PathBuf) -> Result<PathBuf, CliError> {
    if is_valid_app_root(&path) {
        Ok(path)
    } else {
        Err(CliError {
            message: format!(
                "应用根目录不合法: {}，要求包含 main.py、app/、requirements.txt",
                path.display()
            ),
            category: "invalid_runtime_configuration",
            source: "cli",
            backend_payload: None,
        })
    }
}

fn is_valid_app_root(path: &Path) -> bool {
    path.join("main.py").is_file()
        && path.join("app").is_dir()
        && path.join("requirements.txt").is_file()
}

fn is_local_api_url(api_url: &str) -> bool {
    let lowered = api_url.to_ascii_lowercase();
    lowered.contains("127.0.0.1") || lowered.contains("localhost")
}

fn state_file_path() -> Option<PathBuf> {
    let home = env::var("HOME").ok()?;
    let dir = PathBuf::from(home).join(".auto-mas-cli");
    if fs::create_dir_all(&dir).is_err() {
        return None;
    }
    Some(dir.join("state.json"))
}

fn load_backend_state() -> Option<BackendState> {
    let path = state_file_path()?;
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn save_backend_state(state: &BackendState) -> io::Result<()> {
    let path = state_file_path().ok_or_else(|| io::Error::other("state path unavailable"))?;
    let text = serde_json::to_string(state).map_err(|e| io::Error::other(e.to_string()))?;
    fs::write(path, text)
}

fn clear_state_if_matches(api_url: &str) {
    if let Some(path) = state_file_path() {
        if let Some(state) = load_backend_state() {
            if state.api_url.as_deref() == Some(api_url) {
                let _ = fs::remove_file(path);
            }
        }
    }
}
