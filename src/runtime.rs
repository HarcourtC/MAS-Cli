use crate::{BackendState, CliError, SOURCE_CLI};
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) fn discover_app_root(
    override_path: Option<PathBuf>,
) -> Result<Option<PathBuf>, CliError> {
    if let Some(path) = override_path {
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

    for candidate_path in candidates {
        if is_valid_app_root(&candidate_path) {
            return Ok(Some(candidate_path));
        }
    }

    Ok(None)
}

pub(crate) fn discover_python_executable(
    override_path: Option<PathBuf>,
    app_root: Option<&Path>,
) -> Result<Option<PathBuf>, CliError> {
    if let Some(path) = override_path {
        return Ok(Some(path));
    }

    if let Ok(env_python) = env::var("AUTO_MAS_PYTHON") {
        return Ok(Some(PathBuf::from(env_python)));
    }

    if let Some(root) = app_root {
        let embedded_dir = root.join("environment").join("python");
        if let Some(path) = resolve_python_from_embedded_dir(&embedded_dir) {
            return Ok(Some(path));
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

pub(crate) fn is_local_api_url(api_url: &str) -> bool {
    let lowered = api_url.to_ascii_lowercase();
    lowered.contains("127.0.0.1") || lowered.contains("localhost")
}

pub(crate) fn load_backend_state() -> Option<BackendState> {
    let path = state_file_path()?;
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

pub(crate) fn save_backend_state(state: &BackendState) -> io::Result<()> {
    let path = state_file_path().ok_or_else(|| io::Error::other("state path unavailable"))?;
    let text = serde_json::to_string(state).map_err(|e| io::Error::other(e.to_string()))?;
    fs::write(path, text)
}

pub(crate) fn clear_state_if_matches(api_url: &str) {
    if let Some(path) = state_file_path() {
        if let Some(state) = load_backend_state() {
            if state.api_url.as_deref() == Some(api_url) {
                let _ = fs::remove_file(path);
            }
        }
    }
}

pub(crate) fn app_state_dir() -> Option<PathBuf> {
    let home = env::var("HOME")
        .ok()
        .or_else(|| env::var("USERPROFILE").ok())?;
    let dir = PathBuf::from(home).join(".auto-mas-cli");
    if fs::create_dir_all(&dir).is_err() {
        return None;
    }
    Some(dir)
}

fn state_file_path() -> Option<PathBuf> {
    let dir = app_state_dir()?;
    Some(dir.join("state.json"))
}

fn find_command_in_path(cmd: &str) -> Option<PathBuf> {
    let locator = if cfg!(target_os = "windows") {
        "where"
    } else {
        "which"
    };

    let output = Command::new(locator).arg(cmd).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let first_line = String::from_utf8_lossy(&output.stdout)
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(|s| s.trim().to_string())?;
    let path = PathBuf::from(first_line);
    if path.is_file() { Some(path) } else { None }
}

fn resolve_python_from_embedded_dir(embedded_dir: &Path) -> Option<PathBuf> {
    if !embedded_dir.exists() {
        return None;
    }

    let candidates = [
        embedded_dir.join("python.exe"),
        embedded_dir.join("python3.exe"),
        embedded_dir.join("bin").join("python"),
        embedded_dir.join("bin").join("python3"),
        embedded_dir.to_path_buf(),
    ];

    candidates.iter().find(|p| p.is_file()).cloned()
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
            source: SOURCE_CLI,
            backend_payload: None,
        })
    }
}

fn is_valid_app_root(path: &Path) -> bool {
    path.join("main.py").is_file()
        && path.join("app").is_dir()
        && path.join("requirements.txt").is_file()
}
