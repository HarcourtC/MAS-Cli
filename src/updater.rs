use crate::{CliError, cli_error};
use reqwest::blocking::Client;
use reqwest::header::{ACCEPT, HeaderMap, HeaderValue, USER_AGENT};
use serde::Deserialize;
use serde_json::{Value, json};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;
#[cfg(target_os = "windows")]
use std::process::Command;

const RELEASE_API: &str = "https://api.github.com/repos/HarcourtC/MAS-Cli/releases/latest";

#[derive(Debug)]
pub(crate) struct UpdateCheck {
    pub(crate) current_version: String,
    pub(crate) latest_version: String,
    pub(crate) has_update: bool,
    pub(crate) release_url: Option<String>,
}

impl UpdateCheck {
    pub(crate) fn to_json(&self) -> Value {
        json!({
            "currentVersion": self.current_version,
            "latestVersion": self.latest_version,
            "hasUpdate": self.has_update,
            "releaseUrl": self.release_url,
        })
    }
}

#[derive(Debug)]
pub(crate) struct UpdateApplyOutcome {
    pub(crate) message: String,
    pub(crate) updated: bool,
    pub(crate) latest_version: Option<String>,
    pub(crate) next_step: Option<String>,
}

impl UpdateApplyOutcome {
    pub(crate) fn to_json(&self) -> Value {
        json!({
            "updated": self.updated,
            "latestVersion": self.latest_version,
            "nextStep": self.next_step,
        })
    }
}

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    html_url: Option<String>,
    assets: Vec<GitHubAsset>,
}

#[derive(Debug, Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
}

pub(crate) fn print_startup_update_hint() {
    match check_update() {
        Ok(check) if check.has_update => {
            println!(
                "更新提示: 当前 {}，最新 {}",
                check.current_version, check.latest_version
            );
            println!("执行 `mas update apply` 进行更新");
            println!();
        }
        Ok(_) => {}
        Err(_) => {}
    }
}

pub(crate) fn check_update() -> Result<UpdateCheck, CliError> {
    let release = fetch_latest_release()?;
    Ok(build_update_check(&release))
}

pub(crate) fn apply_update() -> Result<UpdateApplyOutcome, CliError> {
    let release = fetch_latest_release()?;
    let check = build_update_check(&release);
    if !check.has_update {
        return Ok(UpdateApplyOutcome {
            message: format!("当前已是最新版本: {}", check.current_version),
            updated: false,
            latest_version: Some(check.latest_version),
            next_step: None,
        });
    }

    let asset_name = expected_asset_name()?;
    let asset = release
        .assets
        .iter()
        .find(|a| a.name.eq_ignore_ascii_case(&asset_name))
        .ok_or_else(|| {
            cli_error(
                "invalid_runtime_configuration",
                format!("未找到当前平台更新包: {}", asset_name),
            )
        })?;

    let update_dir = update_work_dir()?;
    fs::create_dir_all(&update_dir).map_err(|e| {
        cli_error(
            "invalid_runtime_configuration",
            format!("创建更新目录失败: {e}"),
        )
    })?;

    let downloaded_path = update_dir.join(format!("{}.download", asset.name));
    download_asset(&asset.browser_download_url, &downloaded_path)?;
    let current_exe = env::current_exe().map_err(|e| {
        cli_error(
            "invalid_runtime_configuration",
            format!("读取当前可执行文件路径失败: {e}"),
        )
    })?;

    #[cfg(target_os = "windows")]
    {
        schedule_windows_replace(&current_exe, &downloaded_path)?;
        Ok(UpdateApplyOutcome {
            message: "更新包已下载，将在当前进程退出后替换可执行文件".to_string(),
            updated: true,
            latest_version: Some(normalize_version(&release.tag_name)),
            next_step: Some("退出后重新运行: mas".to_string()),
        })
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = current_exe;
        let _ = downloaded_path;
        Err(cli_error(
            "invalid_runtime_configuration",
            "当前仅支持 Windows 自动更新",
        ))
    }
}

fn fetch_latest_release() -> Result<GitHubRelease, CliError> {
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, HeaderValue::from_static("mas-cli-updater"));
    headers.insert(
        ACCEPT,
        HeaderValue::from_static("application/vnd.github+json"),
    );
    let client = Client::builder()
        .default_headers(headers)
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| {
            cli_error(
                "invalid_runtime_configuration",
                format!("创建更新 HTTP 客户端失败: {e}"),
            )
        })?;

    let response = client.get(RELEASE_API).send().map_err(|e| {
        cli_error("backend_unreachable", format!("拉取 Release 失败: {e}"))
    })?;
    let status = response.status();
    if !status.is_success() {
        return Err(cli_error(
            "backend_business_error",
            format!("GitHub Release 接口返回异常状态: {}", status),
        ));
    }

    response.json::<GitHubRelease>().map_err(|e| {
        cli_error(
            "backend_business_error",
            format!("解析 Release JSON 失败: {e}"),
        )
    })
}

fn download_asset(url: &str, target_path: &Path) -> Result<(), CliError> {
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, HeaderValue::from_static("mas-cli-updater"));
    let client = Client::builder()
        .default_headers(headers)
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| {
            cli_error(
                "invalid_runtime_configuration",
                format!("创建下载 HTTP 客户端失败: {e}"),
            )
        })?;

    let response = client
        .get(url)
        .send()
        .map_err(|e| cli_error("backend_unreachable", format!("下载更新包失败: {e}")))?;

    let status = response.status();
    if !status.is_success() {
        return Err(cli_error(
            "backend_business_error",
            format!("下载更新包失败，HTTP 状态: {}", status),
        ));
    }

    let bytes = response.bytes().map_err(|e| {
        cli_error(
            "backend_business_error",
            format!("读取下载内容失败: {e}"),
        )
    })?;

    fs::write(target_path, bytes.as_ref()).map_err(|e| {
        cli_error(
            "invalid_runtime_configuration",
            format!("写入更新包失败: {e}"),
        )
    })
}

fn normalize_version(raw: &str) -> String {
    raw.trim_start_matches('v').to_string()
}

fn build_update_check(release: &GitHubRelease) -> UpdateCheck {
    let current_version = env!("CARGO_PKG_VERSION").to_string();
    let latest_version = normalize_version(&release.tag_name);
    let has_update = is_newer_version(&latest_version, &current_version);
    UpdateCheck {
        current_version,
        latest_version,
        has_update,
        release_url: release.html_url.clone(),
    }
}

fn is_newer_version(candidate: &str, current: &str) -> bool {
    let a = parse_version(candidate);
    let b = parse_version(current);
    a > b
}

fn parse_version(version: &str) -> (u64, u64, u64) {
    let base = version.split('-').next().unwrap_or(version);
    let mut nums = base.split('.').map(|p| p.parse::<u64>().unwrap_or(0));
    (
        nums.next().unwrap_or(0),
        nums.next().unwrap_or(0),
        nums.next().unwrap_or(0),
    )
}

fn expected_asset_name() -> Result<String, CliError> {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        Ok("mas-x86_64-pc-windows-msvc.exe".to_string())
    }
    #[cfg(all(target_os = "windows", target_arch = "x86"))]
    {
        Ok("mas-i686-pc-windows-msvc.exe".to_string())
    }
    #[cfg(not(target_os = "windows"))]
    {
        Err(cli_error(
            "invalid_runtime_configuration",
            "当前仅支持 Windows 更新包",
        ))
    }
    #[cfg(all(target_os = "windows", not(any(target_arch = "x86_64", target_arch = "x86"))))]
    {
        Err(cli_error(
            "invalid_runtime_configuration",
            "当前架构暂不支持自动更新",
        ))
    }
}

fn update_work_dir() -> Result<PathBuf, CliError> {
    crate::runtime::app_state_dir()
        .map(|p| p.join("updates"))
        .ok_or_else(|| cli_error("invalid_runtime_configuration", "无法解析本地状态目录"))
}

#[cfg(target_os = "windows")]
fn schedule_windows_replace(current_exe: &Path, downloaded_path: &Path) -> Result<(), CliError> {
    let script_path = downloaded_path.with_extension("replace.cmd");
    let script_content = format!(
        "@echo off\r\n\
setlocal\r\n\
set \"TARGET={}\"\r\n\
set \"SOURCE={}\"\r\n\
:retry\r\n\
timeout /t 1 /nobreak >nul\r\n\
move /Y \"%SOURCE%\" \"%TARGET%\" >nul 2>nul\r\n\
if errorlevel 1 goto retry\r\n\
del \"%~f0\" >nul 2>nul\r\n",
        current_exe.display(),
        downloaded_path.display(),
    );

    fs::write(&script_path, script_content).map_err(|e| {
        cli_error(
            "invalid_runtime_configuration",
            format!("写入更新替换脚本失败: {e}"),
        )
    })?;

    let status = Command::new("cmd")
        .arg("/C")
        .arg("start")
        .arg("")
        .arg("/MIN")
        .arg(script_path.as_os_str())
        .status()
        .map_err(|e| {
            cli_error(
                "invalid_runtime_configuration",
                format!("启动更新替换脚本失败: {e}"),
            )
        })?;

    if !status.success() {
        return Err(cli_error(
            "backend_startup_failed",
            "更新替换脚本启动失败",
        ));
    }
    Ok(())
}
