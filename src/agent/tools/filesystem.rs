//! Filesystem tools: read_file, write_file, edit_file, list_dir.

use crate::agent::tools::base::Tool;
use async_trait::async_trait;
use base64::Engine;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Detect image MIME type from magic bytes.
fn detect_image_mime(data: &[u8]) -> Option<&'static str> {
    if data.len() >= 8 && &data[..8] == b"\x89PNG\r\n\x1a\n" {
        return Some("image/png");
    }
    if data.len() >= 3 && &data[..3] == b"\xff\xd8\xff" {
        return Some("image/jpeg");
    }
    if data.len() >= 6 && (&data[..6] == b"GIF87a" || &data[..6] == b"GIF89a") {
        return Some("image/gif");
    }
    if data.len() >= 12 && &data[..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    None
}

/// Resolve a path against workspace and enforce directory restriction.
fn resolve_path(
    path: &str,
    workspace: Option<&Path>,
    allowed_dir: Option<&Path>,
) -> Result<PathBuf, String> {
    let p = Path::new(path);
    let p = if p.is_absolute() {
        p.to_path_buf()
    } else {
        workspace.map(|w| w.join(p)).unwrap_or_else(|| p.to_path_buf())
    };

    let resolved = match p.canonicalize() {
        Ok(r) => r,
        Err(_) => p.clone(), // not yet created
    };

    if let Some(allowed) = allowed_dir {
        let allowed = allowed.canonicalize().unwrap_or_else(|_| allowed.to_path_buf());
        if !resolved.starts_with(&allowed) {
            return Err(format!("Path {} is outside allowed directory", path));
        }
    }

    Ok(resolved)
}

// ---------------------------------------------------------------------------
// ReadFileTool
// ---------------------------------------------------------------------------

pub struct ReadFileTool {
    workspace: Option<PathBuf>,
    allowed_dir: Option<PathBuf>,
}

impl ReadFileTool {
    pub fn new(workspace: Option<PathBuf>, allowed_dir: Option<PathBuf>) -> Self {
        Self { workspace, allowed_dir }
    }
}

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str { "read_file" }

    fn description(&self) -> &str {
        "Read the contents of a file. Returns numbered lines. Use offset and limit to paginate."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "The file path to read"},
                "offset": {"type": "integer", "description": "Line to start from (1-indexed, default 1)", "minimum": 1},
                "limit": {"type": "integer", "description": "Max lines to read (default 2000)", "minimum": 1}
            },
            "required": ["path"]
        })
    }

    fn clone_box(&self) -> Box<dyn Tool> {
        Box::new(ReadFileTool {
            workspace: self.workspace.clone(),
            allowed_dir: self.allowed_dir.clone(),
        })
    }

    async fn execute(&self, args: &HashMap<String, Value>) -> String {
        let path = match args.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return "Error: missing 'path' argument".to_string(),
        };
        let offset = args.get("offset").and_then(|v| v.as_u64()).unwrap_or(1) as usize;
        let limit = args.get("limit").and_then(|v| v.as_u64()).map(|v| v as usize);

        let fp = match resolve_path(path, self.workspace.as_deref(), self.allowed_dir.as_deref()) {
            Ok(p) => p,
            Err(e) => return format!("Error: {}", e),
        };

        if !fp.exists() {
            return format!("Error: File not found: {}", path);
        }
        if !fp.is_file() {
            return format!("Error: Not a file: {}", path);
        }

        let raw = match std::fs::read(&fp) {
            Ok(r) => r,
            Err(e) => return format!("Error reading file: {}", e),
        };

        if raw.is_empty() {
            return format!("(Empty file: {})", path);
        }

        // Check if it's an image
        if let Some(mime) = detect_image_mime(&raw) {
            let b64 = base64::engine::general_purpose::STANDARD.encode(&raw);
            return format!(
                r#"[{{"type":"image_url","image_url":{{"url":"data:{};base64,{}"}},"_meta":{{"path":"{}"}}}}]"#,
                mime, b64, fp.display()
            );
        }

        let text = match String::from_utf8(raw) {
            Ok(t) => t,
            Err(_) => return format!("Error: Cannot read binary file {} (not UTF-8 text)", path),
        };

        let all_lines: Vec<&str> = text.lines().collect();
        let total = all_lines.len();

        let offset = offset.max(1);
        if offset > total {
            return format!("Error: offset {} is beyond end of file ({} lines)", offset, total);
        }

        let start = offset - 1;
        let default_limit = 2000;
        let end = (start + limit.unwrap_or(default_limit)).min(total);
        let numbered: Vec<String> = all_lines[start..end]
            .iter()
            .enumerate()
            .map(|(i, line)| format!("{}| {}", start + i + 1, line))
            .collect();

        let mut result = numbered.join("\n");

        const MAX_CHARS: usize = 128_000;
        if result.len() > MAX_CHARS {
            result = result[..MAX_CHARS].to_string();
        }

        if end < total {
            result += &format!("\n\n(Showing lines {}-{} of {}. Use offset={} to continue.)", offset, end, total, end + 1);
        } else {
            result += &format!("\n\n(End of file — {} lines total)", total);
        }

        result
    }
}

// ---------------------------------------------------------------------------
// WriteFileTool
// ---------------------------------------------------------------------------

pub struct WriteFileTool {
    workspace: Option<PathBuf>,
    allowed_dir: Option<PathBuf>,
}

impl WriteFileTool {
    pub fn new(workspace: Option<PathBuf>, allowed_dir: Option<PathBuf>) -> Self {
        Self { workspace, allowed_dir }
    }
}

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &str { "write_file" }

    fn description(&self) -> &str {
        "Write content to a file. Creates parent directories if needed."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "The file path to write to"},
                "content": {"type": "string", "description": "The content to write"}
            },
            "required": ["path", "content"]
        })
    }

    fn clone_box(&self) -> Box<dyn Tool> {
        Box::new(WriteFileTool {
            workspace: self.workspace.clone(),
            allowed_dir: self.allowed_dir.clone(),
        })
    }

    async fn execute(&self, args: &HashMap<String, Value>) -> String {
        let path = match args.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return "Error: missing 'path' argument".to_string(),
        };
        let content = match args.get("content").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => return "Error: missing 'content' argument".to_string(),
        };

        let fp = match resolve_path(path, self.workspace.as_deref(), self.allowed_dir.as_deref()) {
            Ok(p) => p,
            Err(e) => return format!("Error: {}", e),
        };

        if let Some(parent) = fp.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                return format!("Error creating directories: {}", e);
            }
        }

        match std::fs::write(&fp, content) {
            Ok(_) => format!("Successfully wrote {} bytes to {}", content.len(), fp.display()),
            Err(e) => format!("Error writing file: {}", e),
        }
    }
}

// ---------------------------------------------------------------------------
// EditFileTool
// ---------------------------------------------------------------------------

pub struct EditFileTool {
    workspace: Option<PathBuf>,
    allowed_dir: Option<PathBuf>,
}

impl EditFileTool {
    pub fn new(workspace: Option<PathBuf>, allowed_dir: Option<PathBuf>) -> Self {
        Self { workspace, allowed_dir }
    }

    fn find_match<'a>(content: &'a str, old_text: &str) -> Option<(&'a str, usize)> {
        // Exact match first
        let count = content.matches(old_text).count();
        if count > 0 {
            // Return the actual slice from content
            let start = content.find(old_text)?;
            return Some((&content[start..start + old_text.len()], count));
        }

        // Line-trimmed sliding window match
        let old_lines: Vec<&str> = old_text.lines().collect();
        if old_lines.is_empty() {
            return None;
        }
        let stripped_old: Vec<&str> = old_lines.iter().map(|l| l.trim()).collect();
        let content_lines: Vec<&str> = content.lines().collect();

        for i in 0..content_lines.len().saturating_sub(old_lines.len() - 1) {
            let window = &content_lines[i..i + old_lines.len()];
            let stripped_window: Vec<&str> = window.iter().map(|l| l.trim()).collect();
            if stripped_window == stripped_old {
                // Reconstruct the matched fragment
                let matched = window.join("\n");
                let count = content.matches(matched.as_str()).count();
                return Some((&content[content.find(matched.as_str())?..], count));
            }
        }
        None
    }
}

#[async_trait]
impl Tool for EditFileTool {
    fn name(&self) -> &str { "edit_file" }

    fn description(&self) -> &str {
        "Edit a file by replacing old_text with new_text. Supports minor whitespace differences."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "The file path to edit"},
                "old_text": {"type": "string", "description": "The text to find and replace"},
                "new_text": {"type": "string", "description": "The text to replace with"},
                "replace_all": {"type": "boolean", "description": "Replace all occurrences (default false)"}
            },
            "required": ["path", "old_text", "new_text"]
        })
    }

    fn clone_box(&self) -> Box<dyn Tool> {
        Box::new(EditFileTool {
            workspace: self.workspace.clone(),
            allowed_dir: self.allowed_dir.clone(),
        })
    }

    async fn execute(&self, args: &HashMap<String, Value>) -> String {
        let path = match args.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return "Error: missing 'path' argument".to_string(),
        };
        let old_text = match args.get("old_text").and_then(|v| v.as_str()) {
            Some(t) => t,
            None => return "Error: missing 'old_text' argument".to_string(),
        };
        let new_text = match args.get("new_text").and_then(|v| v.as_str()) {
            Some(t) => t,
            None => return "Error: missing 'new_text' argument".to_string(),
        };
        let replace_all = args.get("replace_all").and_then(|v| v.as_bool()).unwrap_or(false);

        let fp = match resolve_path(path, self.workspace.as_deref(), self.allowed_dir.as_deref()) {
            Ok(p) => p,
            Err(e) => return format!("Error: {}", e),
        };

        if !fp.exists() {
            return format!("Error: File not found: {}", path);
        }

        let raw = match std::fs::read(&fp) {
            Ok(r) => r,
            Err(e) => return format!("Error reading file: {}", e),
        };

        let uses_crlf = raw.windows(2).any(|w| w == b"\r\n");
        let content = match String::from_utf8(raw) {
            Ok(s) => s.replace("\r\n", "\n"),
            Err(_) => return "Error: File is not valid UTF-8".to_string(),
        };

        let old_normalized = old_text.replace("\r\n", "\n");

        // Find match
        let (matched, count) = if let Some(r) = Self::find_match(&content, &old_normalized) {
            r
        } else {
            return format!("Error: old_text not found in {}. Verify the file content.", path);
        };

        if count > 1 && !replace_all {
            return format!(
                "Warning: old_text appears {} times. Provide more context to make it unique, or set replace_all=true.",
                count
            );
        }

        let norm_new = new_text.replace("\r\n", "\n");
        let new_content = if replace_all {
            content.replace(matched, &norm_new)
        } else {
            content.replacen(matched, &norm_new, 1)
        };

        let final_content = if uses_crlf {
            new_content.replace('\n', "\r\n")
        } else {
            new_content
        };

        match std::fs::write(&fp, final_content) {
            Ok(_) => format!("Successfully edited {}", fp.display()),
            Err(e) => format!("Error writing file: {}", e),
        }
    }
}

// ---------------------------------------------------------------------------
// ListDirTool
// ---------------------------------------------------------------------------

pub struct ListDirTool {
    workspace: Option<PathBuf>,
    allowed_dir: Option<PathBuf>,
}

impl ListDirTool {
    pub fn new(workspace: Option<PathBuf>, allowed_dir: Option<PathBuf>) -> Self {
        Self { workspace, allowed_dir }
    }

    const IGNORE_DIRS: &'static [&'static str] = &[
        ".git", "node_modules", "__pycache__", ".venv", "venv",
        "dist", "build", ".tox", ".mypy_cache", ".pytest_cache",
        ".ruff_cache", ".coverage", "htmlcov", "target",
    ];
}

#[async_trait]
impl Tool for ListDirTool {
    fn name(&self) -> &str { "list_dir" }

    fn description(&self) -> &str {
        "List directory contents. Set recursive=true to explore nested structure."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "The directory path to list"},
                "recursive": {"type": "boolean", "description": "Recursively list all files (default false)"},
                "max_entries": {"type": "integer", "description": "Maximum entries to return (default 200)", "minimum": 1}
            },
            "required": ["path"]
        })
    }

    fn clone_box(&self) -> Box<dyn Tool> {
        Box::new(ListDirTool {
            workspace: self.workspace.clone(),
            allowed_dir: self.allowed_dir.clone(),
        })
    }

    async fn execute(&self, args: &HashMap<String, Value>) -> String {
        let path = match args.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return "Error: missing 'path' argument".to_string(),
        };
        let recursive = args.get("recursive").and_then(|v| v.as_bool()).unwrap_or(false);
        let max_entries = args.get("max_entries").and_then(|v| v.as_u64()).unwrap_or(200) as usize;

        let dp = match resolve_path(path, self.workspace.as_deref(), self.allowed_dir.as_deref()) {
            Ok(p) => p,
            Err(e) => return format!("Error: {}", e),
        };

        if !dp.exists() {
            return format!("Error: Directory not found: {}", path);
        }
        if !dp.is_dir() {
            return format!("Error: Not a directory: {}", path);
        }

        let mut items: Vec<String> = Vec::new();
        let mut total = 0;

        if recursive {
            fn walk(
                dir: &Path,
                base: &Path,
                items: &mut Vec<String>,
                total: &mut usize,
                max: usize,
                ignore: &[&str],
            ) {
                let mut entries: Vec<_> = std::fs::read_dir(dir)
                    .ok()
                    .into_iter()
                    .flatten()
                    .filter_map(|e| e.ok())
                    .collect();
                entries.sort_by_key(|e| e.path());
                for entry in entries {
                    let path = entry.path();
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if ignore.contains(&name) {
                        continue;
                    }
                    *total += 1;
                    if items.len() < max {
                        if let Ok(rel) = path.strip_prefix(base) {
                            if path.is_dir() {
                                items.push(format!("{}/", rel.display()));
                            } else {
                                items.push(rel.display().to_string());
                            }
                        }
                    }
                    if path.is_dir() {
                        walk(&path, base, items, total, max, ignore);
                    }
                }
            }
            walk(&dp, &dp, &mut items, &mut total, max_entries, Self::IGNORE_DIRS);
        } else {
            let mut entries: Vec<_> = std::fs::read_dir(&dp)
                .ok()
                .into_iter()
                .flatten()
                .filter_map(|e| e.ok())
                .collect();
            entries.sort_by_key(|e| e.path());
            for entry in entries {
                let path = entry.path();
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if Self::IGNORE_DIRS.contains(&name) {
                    continue;
                }
                total += 1;
                if items.len() < max_entries {
                    if path.is_dir() {
                        items.push(format!("📁 {}", name));
                    } else {
                        items.push(format!("📄 {}", name));
                    }
                }
            }
        }

        if items.is_empty() && total == 0 {
            return format!("Directory {} is empty", path);
        }

        let mut result = items.join("\n");
        if total > max_entries {
            result += &format!("\n\n(truncated, showing first {} of {} entries)", max_entries, total);
        }
        result
    }
}
