use crate::{
    Cli, CliEnvelope, CliError, RuntimeContext, SOURCE_BACKEND, StartOutcome, cli_error,
    emit_json_envelope, print_json, queue_output, runtime,
};
use reqwest::blocking::Client;
use serde_json::{Value, json};
#[cfg(target_os = "windows")]
use std::env;
use std::fs;
use std::fs::OpenOptions;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

pub(crate) fn execute(
    cli: &Cli,
    ctx: &mut RuntimeContext,
    command: &crate::RootCommand,
) -> Result<(), CliError> {
    match command {
        crate::RootCommand::Backend { command } => match command {
            crate::BackendCommand::Status => cmd_backend_status(cli, ctx),
            crate::BackendCommand::Start => cmd_backend_start(cli, ctx).map(|_| ()),
            crate::BackendCommand::Stop => cmd_backend_stop(cli, ctx),
        },
        crate::RootCommand::Queue { command } => match command {
            crate::QueueCommand::List => cmd_queue_list(cli, ctx),
            crate::QueueCommand::Start { queue_id, mode } => {
                cmd_queue_start(cli, ctx, queue_id, mode)
            }
        },
    }
}

fn cmd_backend_status(cli: &Cli, ctx: &RuntimeContext) -> Result<(), CliError> {
    let is_ready = probe_backend(&ctx.api_url).is_ok();

    let status_payload = status_data(ctx, is_ready);
    if cli.json {
        let status_message = if is_ready {
            "后端运行中"
        } else {
            "后端未运行"
        };
        let response = CliEnvelope {
            code: 200,
            status: "success".to_string(),
            message: status_message.to_string(),
            data: Some(status_payload),
            source: None,
            category: None,
        };
        emit_json_envelope(response);
    } else if is_ready {
        println!("backend: running");
        println!("apiUrl: {}", ctx.api_url);
        if let Some(app_root) = &ctx.app_root {
            println!("appRoot: {}", app_root.display());
        }
        if let Some(python_executable) = &ctx.python_executable {
            println!("python: {}", python_executable.display());
        }
    } else {
        println!("backend: stopped");
        println!("message: 后端未运行");
        println!("apiUrl: {}", ctx.api_url);
    }

    Ok(())
}

fn cmd_backend_start(cli: &Cli, ctx: &mut RuntimeContext) -> Result<StartOutcome, CliError> {
    if !ctx.is_local_api {
        return Err(cli_error(
            "invalid_runtime_configuration",
            "远端 apiUrl 不支持 backend start",
        ));
    }

    if should_attempt_windows_elevation(cli) && !is_windows_admin() {
        relaunch_self_as_admin()?;
        emit_elevation_handoff(cli);
        return Ok(StartOutcome::HandedOffToElevatedProcess);
    }

    if probe_backend(&ctx.api_url).is_ok() {
        emit_backend_ready(cli, ctx, ctx.state.tracked_pid);
        return Ok(StartOutcome::Ready);
    }

    let app_root = ctx.app_root.clone().ok_or_else(|| {
        cli_error(
            "invalid_runtime_configuration",
            "无法解析应用根目录，请通过 --app-root 或 AUTO_MAS_ROOT 指定",
        )
    })?;

    let python_executable = ctx.python_executable.clone().ok_or_else(|| {
        cli_error(
            "invalid_runtime_configuration",
            "无法解析 Python 解释器，请通过 --python-exe 或 AUTO_MAS_PYTHON 指定",
        )
    })?;

    let mut child_process = spawn_backend_daemon(&python_executable, &app_root)?;

    let process_id = Some(child_process.id());

    // Detach by dropping stdio handles.
    let _ = child_process.stdin.take();
    let _ = child_process.stdout.take();
    let _ = child_process.stderr.take();

    wait_until_ready(&ctx.api_url, 30, Duration::from_millis(500))?;

    ctx.state = crate::BackendState {
        started_by_cli: true,
        tracked_pid: process_id,
        app_root: Some(app_root.display().to_string()),
        python_executable: Some(python_executable.display().to_string()),
        api_url: Some(ctx.api_url.clone()),
    };
    let _ = runtime::save_backend_state(&ctx.state);

    emit_backend_ready(cli, ctx, process_id);
    Ok(StartOutcome::Ready)
}

fn cmd_backend_stop(cli: &Cli, ctx: &mut RuntimeContext) -> Result<(), CliError> {
    if probe_backend(&ctx.api_url).is_err() {
        return Err(cli_error("backend_unreachable", "后端未运行，无法执行命令"));
    }

    let close_response = api_post(&ctx.api_url, "/api/core/close", &json!({}))?;
    if cli.json {
        let target = if ctx.is_local_api { "local" } else { "remote" };
        let response = CliEnvelope {
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
        emit_json_envelope(response);
    } else {
        let _ = close_response;
        println!("后端关闭请求已发送");
        println!(
            "target: {}",
            if ctx.is_local_api { "local" } else { "remote" }
        );
        println!("apiUrl: {}", ctx.api_url);
    }

    runtime::clear_state_if_matches(&ctx.api_url);
    Ok(())
}

fn cmd_queue_list(cli: &Cli, ctx: &mut RuntimeContext) -> Result<(), CliError> {
    let started_for_request = ensure_backend_for_queue_command(cli, ctx)?;

    let queue_response = api_post(&ctx.api_url, "/api/queue/get", &json!({ "queueId": null }))?;

    if cli.json {
        print_json(&queue_response);
    } else {
        queue_output::print_queue_table(&queue_response);
    }

    if started_for_request && !cli.keep_backend {
        let _ = api_post(&ctx.api_url, "/api/core/close", &json!({}));
        runtime::clear_state_if_matches(&ctx.api_url);
    }

    Ok(())
}

fn cmd_queue_start(
    cli: &Cli,
    ctx: &mut RuntimeContext,
    queue_id: &str,
    mode: &str,
) -> Result<(), CliError> {
    let started_for_request = ensure_backend_for_queue_command(cli, ctx)?;

    let start_response = api_post(
        &ctx.api_url,
        "/api/dispatch/start",
        &json!({
            "mode": mode,
            "taskId": queue_id
        }),
    )?;

    if cli.json {
        print_json(&start_response);
    } else {
        println!("queue start accepted");
        println!("queueId: {}", queue_id);
        println!("mode: {}", mode);
    }

    if started_for_request && !cli.keep_backend {
        let _ = api_post(&ctx.api_url, "/api/core/close", &json!({}));
        runtime::clear_state_if_matches(&ctx.api_url);
    }

    Ok(())
}

fn ensure_backend_for_queue_command(cli: &Cli, ctx: &mut RuntimeContext) -> Result<bool, CliError> {
    if probe_backend(&ctx.api_url).is_ok() {
        return Ok(false);
    }

    if cli.no_auto_start {
        return Err(cli_error("backend_unreachable", "后端未运行，无法执行命令"));
    }

    if !ctx.is_local_api {
        return Err(cli_error(
            "backend_unreachable",
            "远端后端不可达，且无法自动启动",
        ));
    }

    let start_outcome = cmd_backend_start(cli, ctx)?;
    if start_outcome == StartOutcome::HandedOffToElevatedProcess {
        wait_until_ready(&ctx.api_url, 60, Duration::from_millis(500))?;
    }

    Ok(true)
}

fn api_post(base_url: &str, path: &str, body: &Value) -> Result<Value, CliError> {
    let url = format!("{}{}", base_url.trim_end_matches('/'), path);
    let http_client = Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| {
            cli_error(
                "invalid_runtime_configuration",
                format!("创建 HTTP 客户端失败: {e}"),
            )
        })?;

    let http_response = http_client
        .post(url)
        .json(body)
        .send()
        .map_err(|_| cli_error("backend_unreachable", "后端未运行，无法执行命令"))?;

    let http_status = http_response.status();
    let response_text = http_response.text().unwrap_or_default();
    let response_json: Value = serde_json::from_str(&response_text).unwrap_or_else(|_| {
        json!({
            "code": http_status.as_u16(),
            "status": if http_status.is_success() { "success" } else { "error" },
            "message": response_text,
        })
    });

    if !http_status.is_success() {
        return Err(error_from_backend_or_default(
            response_json,
            "backend_business_error",
        ));
    }

    if is_backend_error_payload(&response_json) {
        return Err(error_from_backend_or_default(
            response_json,
            "backend_business_error",
        ));
    }

    Ok(response_json)
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

    Err(cli_error(
        "backend_startup_failed",
        "后端启动超时，探活未通过",
    ))
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

fn emit_backend_ready(cli: &Cli, ctx: &RuntimeContext, tracked_pid: Option<u32>) {
    if cli.json {
        let response = CliEnvelope {
            code: 200,
            status: "success".to_string(),
            message: "后端已就绪".to_string(),
            data: Some(json!({
                "ready": true,
                "startedByCli": true,
                "trackedPid": tracked_pid,
                "appRoot": ctx.app_root.as_ref().map(|p| p.display().to_string()),
                "pythonExecutable": ctx.python_executable.as_ref().map(|p| p.display().to_string()),
                "apiUrl": ctx.api_url,
            })),
            source: None,
            category: None,
        };
        emit_json_envelope(response);
    } else {
        println!("backend: running");
        println!("apiUrl: {}", ctx.api_url);
        if let Some(app_root) = &ctx.app_root {
            println!("appRoot: {}", app_root.display());
        }
        if let Some(python_executable) = &ctx.python_executable {
            println!("python: {}", python_executable.display());
        }
        if let Some(pid) = tracked_pid {
            println!("pid: {}", pid);
        }
    }
}

fn emit_elevation_handoff(cli: &Cli) {
    if cli.json {
        let response = CliEnvelope {
            code: 200,
            status: "success".to_string(),
            message: "已请求管理员权限，命令将在提权后继续执行".to_string(),
            data: Some(json!({
                "elevated": true,
                "restarted": true
            })),
            source: None,
            category: None,
        };
        emit_json_envelope(response);
    } else {
        println!("已请求管理员权限，命令将在提权后继续执行");
    }
}

fn error_from_backend_or_default(
    backend_payload: Value,
    fallback_category: &'static str,
) -> CliError {
    let message = backend_payload
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("后端返回错误")
        .to_string();

    CliError {
        message,
        category: fallback_category,
        source: SOURCE_BACKEND,
        backend_payload: Some(backend_payload),
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

fn should_attempt_windows_elevation(cli: &Cli) -> bool {
    cfg!(target_os = "windows") && !cli.elevated
}

fn is_windows_admin() -> bool {
    #[cfg(target_os = "windows")]
    {
        let script = "[bool]([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)";
        let output = Command::new("powershell")
            .arg("-NoProfile")
            .arg("-NonInteractive")
            .arg("-Command")
            .arg(script)
            .output();

        match output {
            Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout)
                .trim()
                .eq_ignore_ascii_case("true"),
            _ => false,
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        true
    }
}

fn relaunch_self_as_admin() -> Result<(), CliError> {
    #[cfg(target_os = "windows")]
    {
        let current_executable = env::current_exe().map_err(|e| {
            cli_error(
                "invalid_runtime_configuration",
                format!("读取当前可执行文件路径失败: {e}"),
            )
        })?;

        let current_working_dir = env::current_dir().map_err(|e| {
            cli_error(
                "invalid_runtime_configuration",
                format!("读取当前工作目录失败: {e}"),
            )
        })?;

        let mut elevated_args: Vec<String> = env::args()
            .skip(1)
            .filter(|arg| arg != "--elevated")
            .collect();
        elevated_args.push("--elevated".to_string());

        let argument_list = elevated_args
            .iter()
            .map(|arg| format!("'{}'", arg.replace('\'', "''")))
            .collect::<Vec<_>>()
            .join(",");

        let powershell_script = format!(
            "Start-Process -FilePath '{}' -ArgumentList @({}) -WorkingDirectory '{}' -Verb RunAs",
            current_executable.display().to_string().replace('\'', "''"),
            argument_list,
            current_working_dir
                .display()
                .to_string()
                .replace('\'', "''"),
        );

        let elevation_status = Command::new("powershell")
            .arg("-NoProfile")
            .arg("-NonInteractive")
            .arg("-Command")
            .arg(powershell_script)
            .status()
            .map_err(|e| {
                cli_error(
                    "invalid_runtime_configuration",
                    format!("请求管理员权限失败: {e}"),
                )
            })?;

        if !elevation_status.success() {
            return Err(cli_error(
                "backend_startup_failed",
                "用户取消了管理员权限请求或系统拒绝提权",
            ));
        }
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        Ok(())
    }
}

fn spawn_backend_daemon(python_executable: &Path, app_root: &Path) -> Result<Child, CliError> {
    let log_dir = runtime::app_state_dir().unwrap_or_else(|| app_root.to_path_buf());
    let _ = fs::create_dir_all(&log_dir);

    let stdout_log = log_dir.join("backend.stdout.log");
    let stderr_log = log_dir.join("backend.stderr.log");

    let stdout_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&stdout_log)
        .map_err(|e| {
            cli_error(
                "invalid_runtime_configuration",
                format!("打开后端 stdout 日志失败: {e} ({})", stdout_log.display()),
            )
        })?;
    let stderr_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&stderr_log)
        .map_err(|e| {
            cli_error(
                "invalid_runtime_configuration",
                format!("打开后端 stderr 日志失败: {e} ({})", stderr_log.display()),
            )
        })?;

    let mut command = Command::new(python_executable);
    command
        .arg("main.py")
        .current_dir(app_root)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout_file))
        .stderr(Stdio::from(stderr_file));

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }

    command.spawn().map_err(|e| {
        cli_error(
            "backend_startup_failed",
            format!(
                "启动后端进程失败: {e} (python={})",
                python_executable.display()
            ),
        )
    })
}
