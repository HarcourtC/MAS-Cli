use crate::{CliError, cli_error};
use serde_json::{Value, json};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const INSTALL_DIR_NAME: &str = "AUTO-MAS\\bin";
const COMMAND_FILE_NAME: &str = "mas.exe";

#[derive(Debug)]
pub(crate) struct RegisterResult {
    pub(crate) installed_executable: PathBuf,
    pub(crate) path_updated: bool,
}

impl RegisterResult {
    pub(crate) fn to_json(&self) -> Value {
        json!({
            "installedExecutable": self.installed_executable,
            "pathUpdated": self.path_updated,
        })
    }
}

#[derive(Debug)]
pub(crate) struct UnregisterResult {
    pub(crate) executable_removed: bool,
    pub(crate) path_updated: bool,
}

impl UnregisterResult {
    pub(crate) fn to_json(&self) -> Value {
        json!({
            "executableRemoved": self.executable_removed,
            "pathUpdated": self.path_updated,
        })
    }
}

pub(crate) fn register_command_alias() -> Result<RegisterResult, CliError> {
    ensure_windows()?;
    let install_dir = user_install_dir()?;
    fs::create_dir_all(&install_dir).map_err(|e| {
        cli_error(
            "invalid_runtime_configuration",
            format!("创建安装目录失败: {e}"),
        )
    })?;

    let target_executable = install_dir.join(COMMAND_FILE_NAME);
    let source_executable = env::current_exe().map_err(|e| {
        cli_error(
            "invalid_runtime_configuration",
            format!("读取当前可执行文件失败: {e}"),
        )
    })?;
    if source_executable != target_executable {
        fs::copy(&source_executable, &target_executable).map_err(|e| {
            cli_error(
                "invalid_runtime_configuration",
                format!("复制可执行文件失败: {e}"),
            )
        })?;
    }

    let path_updated = add_user_path(&install_dir)?;
    Ok(RegisterResult {
        installed_executable: target_executable,
        path_updated,
    })
}

pub(crate) fn unregister_command_alias() -> Result<UnregisterResult, CliError> {
    ensure_windows()?;
    let install_dir = user_install_dir()?;
    let target_executable = install_dir.join(COMMAND_FILE_NAME);
    let executable_removed = if target_executable.exists() {
        fs::remove_file(&target_executable).is_ok()
    } else {
        false
    };

    let path_updated = remove_user_path(&install_dir)?;
    Ok(UnregisterResult {
        executable_removed,
        path_updated,
    })
}

fn ensure_windows() -> Result<(), CliError> {
    #[cfg(not(target_os = "windows"))]
    {
        return Err(cli_error(
            "invalid_runtime_configuration",
            "当前仅支持 Windows 系统命令注册",
        ));
    }
    #[cfg(target_os = "windows")]
    {
        Ok(())
    }
}

fn user_install_dir() -> Result<PathBuf, CliError> {
    let local_app_data = env::var("LOCALAPPDATA").map_err(|_| {
        cli_error(
            "invalid_runtime_configuration",
            "未检测到 LOCALAPPDATA，无法安装 mas 命令",
        )
    })?;
    Ok(PathBuf::from(local_app_data).join(INSTALL_DIR_NAME))
}

fn add_user_path(path: &Path) -> Result<bool, CliError> {
    let existing = read_user_path()?;
    if path_contains(&existing, path) {
        return Ok(false);
    }

    let updated = if existing.trim().is_empty() {
        path.display().to_string()
    } else {
        format!("{};{}", existing, path.display())
    };
    write_user_path(&updated)?;
    Ok(true)
}

fn remove_user_path(path: &Path) -> Result<bool, CliError> {
    let existing = read_user_path()?;
    let mut removed = false;
    let normalized_target = normalize_path(path);
    let kept: Vec<String> = existing
        .split(';')
        .filter_map(|segment| {
            let trimmed = segment.trim();
            if trimmed.is_empty() {
                return None;
            }
            if normalize_path_str(trimmed) == normalized_target {
                removed = true;
                return None;
            }
            Some(trimmed.to_string())
        })
        .collect();

    if removed {
        write_user_path(&kept.join(";"))?;
    }
    Ok(removed)
}

fn path_contains(existing: &str, target: &Path) -> bool {
    let normalized_target = normalize_path(target);
    existing
        .split(';')
        .any(|segment| normalize_path_str(segment) == normalized_target)
}

fn normalize_path(path: &Path) -> String {
    normalize_path_str(&path.display().to_string())
}

fn normalize_path_str(path: &str) -> String {
    path.trim()
        .trim_matches('"')
        .trim_end_matches('\\')
        .to_ascii_lowercase()
}

fn read_user_path() -> Result<String, CliError> {
    run_powershell(
        "[Environment]::GetEnvironmentVariable('Path', 'User')",
        "读取用户 PATH 失败",
    )
}

fn write_user_path(value: &str) -> Result<(), CliError> {
    let escaped = value.replace('\'', "''");
    let script = format!(
        "[Environment]::SetEnvironmentVariable('Path', '{}', 'User')",
        escaped
    );
    let _ = run_powershell(&script, "写入用户 PATH 失败")?;
    Ok(())
}

fn run_powershell(script: &str, context: &str) -> Result<String, CliError> {
    let output = Command::new("powershell")
        .arg("-NoProfile")
        .arg("-NonInteractive")
        .arg("-Command")
        .arg(script)
        .output()
        .map_err(|e| {
            cli_error(
                "invalid_runtime_configuration",
                format!("{context}: {e}"),
            )
        })?;

    if !output.status.success() {
        return Err(cli_error(
            "invalid_runtime_configuration",
            format!(
                "{context}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
