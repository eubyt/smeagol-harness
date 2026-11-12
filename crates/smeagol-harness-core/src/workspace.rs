//! Resolve repository-owned scope without using process-global active-project state.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

const MARKER_NAME: &str = ".smeagol-harness.toml";
const MAX_MARKER_BYTES: u64 = 65_536;

/// Human-readable scope names and an optional stable repository routing key.
/// Storage must map these names to internal IDs instead of using them as paths.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectContext {
    pub workspace: String,
    pub project: String,
    pub repository_identity: Option<String>,
    pub marker: Option<PathBuf>,
}

impl ProjectContext {
    /// Apply explicit routing overrides after marker and repository resolution.
    /// Explicit project selection resolves the existing namespace by name rather
    /// than creating a new repository identity after a project has been renamed.
    pub fn with_overrides(
        mut self,
        workspace: Option<&str>,
        project: Option<&str>,
    ) -> Result<Self> {
        if let Some(workspace) = workspace {
            validate_context_name(workspace)?;
            self.workspace = workspace.to_owned();
        }
        if let Some(project) = project {
            validate_project_name(project)?;
            self.project = project.to_owned();
            self.repository_identity = None;
        }
        Ok(self)
    }
}

/// Validate a logical scope name; Unicode is allowed but path syntax is rejected.
pub fn validate_context_name(name: &str) -> Result<()> {
    if name.trim() != name
        || name.is_empty()
        || name.chars().count() > 128
        || matches!(name, "." | "..")
        || name
            .chars()
            .any(|ch| ch.is_control() || matches!(ch, '/' | '\\'))
    {
        bail!(
            "nome de workspace/projeto inválido: use de 1 a 128 caracteres, sem separadores de caminho ou controles"
        );
    }
    Ok(())
}

fn validate_project_name(name: &str) -> Result<()> {
    validate_context_name(name)?;
    if name == "_profile" {
        bail!("o nome de projeto '_profile' é reservado ao perfil global");
    }
    Ok(())
}

/// Resolve the nearest scope marker, then Git remote identity, then directory name.
/// A malformed marker fails closed so captures cannot silently change projects.
pub fn resolve_project_context(cwd: &Path) -> Result<ProjectContext> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    resolve_at(cwd, home.as_deref())
}

fn resolve_at(cwd: &Path, home: Option<&Path>) -> Result<ProjectContext> {
    let cwd = fs::canonicalize(cwd)
        .with_context(|| format!("não foi possível localizar o diretório {}", cwd.display()))?;
    if !cwd.is_dir() {
        bail!("o contexto de projeto precisa apontar para um diretório");
    }
    let home = home.and_then(|path| fs::canonicalize(path).ok());
    let checkout = find_checkout_root(&cwd);
    let boundary = match home.as_deref() {
        Some(home) if cwd.starts_with(home) => home.to_path_buf(),
        Some(_) => checkout.clone().unwrap_or_else(|| cwd.clone()),
        None => cwd.ancestors().last().unwrap_or(&cwd).to_path_buf(),
    };
    let marker = find_scope_marker(&cwd, &boundary)?;
    let settings = marker.as_ref().map(|(_, value)| value);
    let workspace = marker_string(settings, "workspace")?
        .unwrap_or("default")
        .to_owned();
    let explicit_project = marker_string(settings, "project")?;
    let explicit_identity = marker_string(settings, "identity")?;
    let strategy = marker_string(settings, "project_strategy")?.unwrap_or("basename");
    if !matches!(strategy, "basename" | "repo-root") {
        bail!("project_strategy inválido; use 'basename' ou 'repo-root'");
    }
    let remote = repository_remote(&cwd);
    let project = match explicit_project {
        Some(project) => project.to_owned(),
        None => remote
            .as_ref()
            .map(|(_, path)| repository_path_name(path))
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| {
                let directory = if strategy == "repo-root" {
                    main_checkout_root(&cwd)
                        .or(checkout)
                        .unwrap_or_else(|| cwd.clone())
                } else {
                    cwd.clone()
                };
                directory
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("project")
                    .to_owned()
            }),
    };
    validate_context_name(&workspace)?;
    validate_project_name(&project)?;
    let repository_identity = if let Some(identity) = explicit_identity {
        validate_context_name(identity)?;
        Some(format!("marker:{identity}"))
    } else if let Some(project) = explicit_project {
        Some(format!("marker:{project}"))
    } else {
        remote.map(|(identity, _)| identity)
    };
    Ok(ProjectContext {
        workspace,
        project,
        repository_identity,
        marker: marker.map(|(path, _)| path),
    })
}

fn marker_string<'a>(settings: Option<&'a toml::Value>, key: &str) -> Result<Option<&'a str>> {
    let Some(value) = settings.and_then(|settings| settings.get(key)) else {
        return Ok(None);
    };
    let text = value
        .as_str()
        .with_context(|| format!("'{key}' precisa ser texto no marcador"))?;
    if text.trim().is_empty() {
        bail!("'{key}' não pode ser vazio no marcador");
    }
    Ok(Some(text))
}

fn find_scope_marker(cwd: &Path, boundary: &Path) -> Result<Option<(PathBuf, toml::Value)>> {
    for directory in cwd.ancestors() {
        let path = directory.join(MARKER_NAME);
        match fs::metadata(&path) {
            Ok(metadata) => {
                if !metadata.is_file() || metadata.len() > MAX_MARKER_BYTES {
                    bail!("marcador {} inválido ou maior que 64 KiB", path.display());
                }
                let text = fs::read_to_string(&path)
                    .with_context(|| format!("falha ao ler marcador {}", path.display()))?;
                let document = text
                    .parse::<toml::Value>()
                    .with_context(|| format!("TOML inválido em {}", path.display()))?;
                if ["workspace", "project", "identity", "project_strategy"]
                    .iter()
                    .any(|key| document.get(key).is_some())
                {
                    return Ok(Some((path, document)));
                }
                // Capture-only markers do not reset the enclosing scope.
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("falha ao acessar marcador {}", path.display()));
            }
        }
        if directory == boundary {
            break;
        }
    }
    Ok(None)
}

fn find_checkout_root(cwd: &Path) -> Option<PathBuf> {
    cwd.ancestors()
        .find(|path| path.join(".git").exists())
        .map(Path::to_path_buf)
}

fn git_output(cwd: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    (!value.is_empty()).then_some(value)
}

fn main_checkout_root(cwd: &Path) -> Option<PathBuf> {
    let common = git_output(
        cwd,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    let common = PathBuf::from(common);
    if common.file_name().is_some_and(|name| name == ".git") {
        common.parent().map(Path::to_path_buf)
    } else {
        git_output(cwd, &["rev-parse", "--show-toplevel"]).map(PathBuf::from)
    }
}

fn repository_remote(cwd: &Path) -> Option<(String, String)> {
    ["upstream", "origin"].into_iter().find_map(|remote| {
        let raw = git_output(cwd, &["remote", "get-url", remote])?;
        normalize_remote(&raw)
    })
}

fn normalize_remote(raw: &str) -> Option<(String, String)> {
    let raw = raw.trim();
    if raw.starts_with('/') || raw.starts_with('.') || raw.starts_with('\\') || raw.contains('\\') {
        return None;
    }
    let (host, path) = if let Some((scheme, rest)) = raw.split_once("://") {
        if !matches!(scheme, "https" | "http" | "ssh" | "git") {
            return None;
        }
        let (authority, path) = rest.split_once('/')?;
        let authority = authority.rsplit('@').next()?;
        let host = authority.split(':').next()?;
        (host, path)
    } else {
        let (authority, path) = raw.split_once(':')?;
        let host = authority.rsplit('@').next()?;
        if host.len() < 2 {
            return None;
        }
        (host, path)
    };
    let path = path.trim_matches('/').to_lowercase();
    let path = path.strip_suffix(".git").unwrap_or(&path);
    if host.is_empty()
        || path.is_empty()
        || path
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
        || raw.chars().any(char::is_whitespace)
    {
        return None;
    }
    let host = host.to_lowercase();
    let path = path.to_lowercase();
    Some((format!("{host}/{path}"), path))
}

fn repository_path_name(path: &str) -> String {
    let mut name = String::new();
    for ch in path.chars() {
        if ch.is_alphanumeric() || ch == '_' {
            name.extend(ch.to_lowercase());
        } else if !name.is_empty() && !name.ends_with('-') {
            name.push('-');
        }
    }
    name.trim_end_matches('-').to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_marker(path: &Path, contents: &str) {
        fs::write(path.join(MARKER_NAME), contents).unwrap();
    }

    fn git(cwd: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(cwd)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn nearest_scope_marker_overrides_parent_without_merging_fields() {
        let temp = tempfile::tempdir().unwrap();
        let child = temp.path().join("child");
        fs::create_dir(&child).unwrap();
        write_marker(
            temp.path(),
            "workspace = 'parent'\nproject = 'parent-project'",
        );
        write_marker(&child, "project = 'child-project'");
        let context = resolve_at(&child, Some(temp.path())).unwrap();
        assert_eq!(context.workspace, "default");
        assert_eq!(context.project, "child-project");
    }

    #[test]
    fn capture_only_markers_preserve_parent_scope_and_unicode_names() {
        let temp = tempfile::tempdir().unwrap();
        let child = temp.path().join("child");
        fs::create_dir(&child).unwrap();
        write_marker(
            temp.path(),
            "workspace = 'Pessoal'\nproject = 'Memória jurídica'",
        );
        write_marker(&child, "[capture]\nignore_paths = ['private/**']");
        let context = resolve_at(&child, Some(temp.path())).unwrap();
        assert_eq!(
            (context.workspace.as_str(), context.project.as_str()),
            ("Pessoal", "Memória jurídica")
        );
        assert_eq!(
            context.marker,
            Some(fs::canonicalize(temp.path()).unwrap().join(MARKER_NAME))
        );
    }

    #[test]
    fn malformed_markers_fail_closed_instead_of_using_defaults() {
        let temp = tempfile::tempdir().unwrap();
        for contents in [
            "workspace = [",
            "project = 5",
            "workspace = '../escape'",
            "project = ''",
            "project = '_profile'",
            "project_strategy = 'unknown'",
        ] {
            write_marker(temp.path(), contents);
            assert!(
                resolve_at(temp.path(), Some(temp.path())).is_err(),
                "{contents}"
            );
        }
    }

    #[test]
    fn outside_home_markers_stop_at_checkout_root_or_current_directory() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let checkout = temp.path().join("repo");
        let nested = checkout.join("nested");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&nested).unwrap();
        fs::create_dir(checkout.join(".git")).unwrap();
        write_marker(temp.path(), "project = 'must-not-leak'");
        let context = resolve_at(&nested, Some(&home)).unwrap();
        assert_eq!(context.project, "nested");
        fs::remove_dir(checkout.join(".git")).unwrap();
        write_marker(&checkout, "project = 'must-not-leak-either'");
        assert_eq!(resolve_at(&nested, Some(&home)).unwrap().project, "nested");
    }

    #[test]
    fn explicit_project_selection_uses_names_after_renaming() {
        let context = ProjectContext {
            workspace: "work".into(),
            project: "old-name".into(),
            repository_identity: Some("marker:old-name".into()),
            marker: None,
        };
        let selected = context
            .with_overrides(None, Some("renamed-project"))
            .unwrap();
        assert_eq!(selected.project, "renamed-project");
        assert_eq!(selected.workspace, "work");
        assert_eq!(selected.repository_identity, None);
    }

    #[test]
    fn workspace_override_preserves_repository_identity_and_reserved_profile_name_is_project_only()
    {
        let context = ProjectContext {
            workspace: "work".into(),
            project: "api".into(),
            repository_identity: Some("example.org/acme/api".into()),
            marker: None,
        };
        assert!(
            context
                .clone()
                .with_overrides(None, Some("_profile"))
                .is_err()
        );
        let selected = context.with_overrides(Some("_profile"), None).unwrap();
        assert_eq!(selected.workspace, "_profile");
        assert_eq!(
            selected.repository_identity.as_deref(),
            Some("example.org/acme/api")
        );
        let temp = tempfile::tempdir().unwrap();
        write_marker(temp.path(), "workspace = '_profile'\nproject = 'api'");
        assert_eq!(
            resolve_at(temp.path(), Some(temp.path()))
                .unwrap()
                .workspace,
            "_profile"
        );
    }

    #[test]
    fn valid_remote_wins_over_repo_root_fallback_and_invalid_upstream_is_skipped() {
        let temp = tempfile::tempdir().unwrap();
        git(temp.path(), &["init"]);
        git(temp.path(), &["remote", "add", "upstream", "/local/path"]);
        git(
            temp.path(),
            &[
                "remote",
                "add",
                "origin",
                "ssh://git@example.org:2222/Acme/API.git",
            ],
        );
        let nested = temp.path().join("subdir");
        fs::create_dir(&nested).unwrap();
        write_marker(
            temp.path(),
            "workspace = 'work'\nproject_strategy = 'repo-root'",
        );
        let context = resolve_at(&nested, Some(temp.path())).unwrap();
        assert_eq!(context.project, "acme-api");
        assert_eq!(
            context.repository_identity.as_deref(),
            Some("example.org/acme/api")
        );
    }

    #[test]
    fn explicit_identity_and_project_take_precedence_over_remote() {
        let temp = tempfile::tempdir().unwrap();
        git(temp.path(), &["init"]);
        git(
            temp.path(),
            &[
                "remote",
                "add",
                "origin",
                "https://example.org/acme/api.git",
            ],
        );
        write_marker(
            temp.path(),
            "workspace = 'work'\nproject = 'shared'\nidentity = 'stable-key'",
        );
        let context = resolve_at(temp.path(), Some(temp.path())).unwrap();
        assert_eq!(context.project, "shared");
        assert_eq!(
            context.repository_identity.as_deref(),
            Some("marker:stable-key")
        );
    }

    #[test]
    fn remotes_are_hostful_and_do_not_retain_credentials() {
        for remote in [
            "https://secret@example.org/acme/api.git",
            "git@example.org:acme/api.git",
            "ssh://git@example.org:2222/acme/api.git",
        ] {
            assert_eq!(
                normalize_remote(remote),
                Some(("example.org/acme/api".into(), "acme/api".into()))
            );
        }
        for remote in [
            "file:///tmp/repo",
            "C:/repo",
            "../repo",
            "/tmp/repo",
            "not-a-remote",
        ] {
            assert_eq!(normalize_remote(remote), None, "{remote}");
        }
    }

    #[test]
    fn sibling_checkouts_with_the_same_remote_share_context() {
        let temp = tempfile::tempdir().unwrap();
        let mut contexts = Vec::new();
        for name in ["checkout", "other-worktree"] {
            let path = temp.path().join(name);
            fs::create_dir(&path).unwrap();
            git(&path, &["init"]);
            git(
                &path,
                &[
                    "remote",
                    "add",
                    "origin",
                    "https://example.org/acme/api.git",
                ],
            );
            contexts.push(resolve_at(&path, Some(temp.path())).unwrap());
        }
        assert_eq!(contexts[0], contexts[1]);
    }

    #[test]
    fn repo_root_strategy_groups_linked_worktrees_without_a_remote() {
        let temp = tempfile::tempdir().unwrap();
        let checkout = temp.path().join("main-repo");
        let worktree = temp.path().join("feature-worktree");
        fs::create_dir(&checkout).unwrap();
        git(&checkout, &["init"]);
        git(
            &checkout,
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--allow-empty",
                "-m",
                "fixture",
            ],
        );
        git(
            &checkout,
            &["worktree", "add", "--detach", worktree.to_str().unwrap()],
        );
        write_marker(temp.path(), "project_strategy = 'repo-root'");
        let main_context = resolve_at(&checkout, Some(temp.path())).unwrap();
        let worktree_context = resolve_at(&worktree, Some(temp.path())).unwrap();
        assert_eq!(main_context.project, "main-repo");
        assert_eq!(main_context, worktree_context);
    }
}
