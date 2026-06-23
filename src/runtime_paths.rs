use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub(crate) fn resolve_command_path(
    name: &str,
    configured: Option<&str>,
    display_name: &str,
    child_path: &str,
) -> Option<PathBuf> {
    if let Some(path) = configured.filter(|path| !path.trim().is_empty()) {
        let path = expand_home(path.trim());
        if is_executable_file(&path) {
            return Some(path);
        }

        log::error!(
            "Configured path for {} does not point to an executable file: {:?}",
            display_name,
            path
        );
        return None;
    }

    let mut searched = Vec::new();

    if let Some(path) = find_executable_in_path(name, &std::env::var_os("PATH"), &mut searched) {
        return Some(path);
    }

    if let Some(path) = find_executable_in_path(name, &Some(child_path.into()), &mut searched) {
        return Some(path);
    }

    log::error!(
        "Failed to find {}. Searched: {}.",
        display_name,
        searched
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
    None
}

pub(crate) fn build_child_path() -> String {
    let mut seen = HashSet::new();
    let mut paths = Vec::new();

    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            push_unique_path(&mut paths, &mut seen, dir);
        }
    }

    for dir in [
        "/opt/homebrew/bin",
        "/opt/homebrew/sbin",
        "/usr/local/bin",
        "/usr/local/sbin",
        "/opt/orbstack/bin",
        "/Applications/Docker.app/Contents/Resources/bin",
        "/opt/local/bin",
        "/opt/local/sbin",
        "/nix/var/nix/profiles/default/bin",
        "/run/current-system/sw/bin",
        "/usr/bin",
        "/bin",
        "/usr/sbin",
        "/sbin",
    ] {
        push_unique_path(&mut paths, &mut seen, PathBuf::from(dir));
    }

    std::env::join_paths(paths)
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

pub(crate) fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn find_executable_in_path(
    name: &str,
    path: &Option<std::ffi::OsString>,
    searched: &mut Vec<PathBuf>,
) -> Option<PathBuf> {
    let Some(path) = path else {
        return None;
    };

    for dir in std::env::split_paths(path) {
        let candidate = dir.join(name);
        if !searched.iter().any(|path| path == &candidate) {
            searched.push(candidate.clone());
        }
        if is_executable_file(&candidate) {
            return Some(candidate);
        }
    }

    None
}

fn push_unique_path(paths: &mut Vec<PathBuf>, seen: &mut HashSet<PathBuf>, path: PathBuf) {
    if seen.insert(path.clone()) {
        paths.push(path);
    }
}

fn expand_home(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }

    PathBuf::from(path)
}

fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };

    metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
}
