//! Configuration paths for ombudsman.

use std::path::PathBuf;

/// Get the data directory for ombudsman (~/.ombudsman).
pub fn get_data_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".ombudsman")
}

/// Get the config file path (~/.ombudsman/config.json).
pub fn get_config_path() -> PathBuf {
    get_data_dir().join("config.json")
}

/// Get the default workspace path (~/.ombudsman/workspace).
pub fn get_workspace_path() -> PathBuf {
    get_data_dir().join("workspace")
}

/// Get the sessions directory path.
pub fn get_sessions_dir(workspace: &std::path::Path) -> PathBuf {
    workspace.join("sessions")
}

/// Get the memory directory path.
pub fn get_memory_dir(workspace: &std::path::Path) -> PathBuf {
    workspace.join("memory")
}

/// Get the skills directory path.
pub fn get_skills_dir(workspace: &std::path::Path) -> PathBuf {
    workspace.join("skills")
}

/// Expand a path string, replacing ~ with the home directory.
pub fn expand_path(path: &str) -> PathBuf {
    if path.starts_with("~/") || path == "~" {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        home.join(&path[2..])
    } else {
        PathBuf::from(path)
    }
}
