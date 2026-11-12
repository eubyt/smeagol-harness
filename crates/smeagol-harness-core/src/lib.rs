use std::fmt;
use std::path::Path;
use std::process::Command;

/// Storage boundary used to isolate persisted memories.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MemoryScope {
    Profile,
    Project(String),
    Session(String),
}

impl MemoryScope {
    /// Return a relative storage path with scope identifiers sanitized.
    pub fn storage_path(&self) -> String {
        match self {
            Self::Profile => "profile".to_owned(),
            Self::Project(project) => format!("projects/{}", safe_identifier(project)),
            Self::Session(session) => format!("sessions/{}", safe_identifier(session)),
        }
    }
}

/// Memory record whose Markdown source can be indexed for search.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryRecord {
    pub id: String,
    pub scope: MemoryScope,
    pub title: String,
    pub body: String,
}

impl MemoryRecord {
    /// Reject invalid path components and empty titles before persistence.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if !valid_component(&self.id) {
            return Err(ValidationError(
                "id deve conter apenas letras, números, '-' ou '_'",
            ));
        }
        if self.title.trim().is_empty() {
            return Err(ValidationError("título não pode ser vazio"));
        }
        if let MemoryScope::Project(id) | MemoryScope::Session(id) = &self.scope
            && !valid_component(id)
        {
            return Err(ValidationError(
                "identificador do escopo contém caracteres inválidos",
            ));
        }
        Ok(())
    }
}

/// Validation failure for a memory record or its storage scope.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ValidationError(pub &'static str);

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for ValidationError {}

fn valid_component(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
}

/// Replace unsupported characters before using an identifier in a storage path.
pub fn safe_identifier(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect()
}

/// Estimate token usage without depending on a provider-specific tokenizer.
/// This heuristic guides briefing budgets; it is not an exact token count.
pub fn estimate_tokens(text: &str) -> usize {
    text.split_whitespace()
        .map(|word| word.chars().count().div_ceil(4).max(1))
        .sum()
}

/// Prefer a Git remote identity; fall back to the directory name when unavailable.
pub fn project_id_for_cwd(cwd: &Path) -> String {
    for remote_name in ["upstream", "origin"] {
        if let Ok(output) = Command::new("git")
            .arg("-C")
            .arg(cwd)
            .args(["remote", "get-url", remote_name])
            .output()
            && output.status.success()
        {
            let remote = String::from_utf8_lossy(&output.stdout);
            let id = normalize_remote_identity(remote.trim());
            if !id.is_empty() {
                return id;
            }
        }
    }
    cwd.file_name()
        .and_then(|name| name.to_str())
        .map(slug_component)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "project".to_owned())
}

/// Normalize HTTPS and SSH remotes without retaining the host or `.git` suffix.
pub fn normalize_remote_identity(remote: &str) -> String {
    let path = if let Some((_, path)) = remote
        .strip_prefix("git@")
        .and_then(|value| value.split_once(':'))
    {
        path
    } else if let Some((_, remainder)) = remote.split_once("://") {
        remainder
            .split_once('/')
            .map(|(_, path)| path)
            .unwrap_or("")
    } else {
        remote
    };
    let path = path
        .trim_end_matches('/')
        .strip_suffix(".git")
        .unwrap_or(path.trim_end_matches('/'));
    slug_component(path)
}

fn slug_component(value: &str) -> String {
    let mut output = String::new();
    let mut previous_separator = false;
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            output.push(ch.to_ascii_lowercase());
            previous_separator = false;
        } else if !previous_separator && !output.is_empty() {
            output.push('-');
            previous_separator = true;
        }
    }
    output.trim_matches('-').to_owned()
}

#[cfg(test)]
mod tests {
    use super::{MemoryRecord, MemoryScope};

    #[test]
    fn accepts_records_with_valid_scopes() {
        let record = MemoryRecord {
            id: "decisao-cache".to_owned(),
            scope: MemoryScope::Project("meu-projeto".to_owned()),
            title: "Cache local".to_owned(),
            body: "Usar cache local para reduzir chamadas repetidas.".to_owned(),
        };
        assert!(record.validate().is_ok());
    }

    #[test]
    fn rejects_components_that_could_escape_the_data_directory() {
        let record = MemoryRecord {
            id: "../../fora".to_owned(),
            scope: MemoryScope::Project("meu-projeto".to_owned()),
            title: "Inválido".to_owned(),
            body: String::new(),
        };
        assert!(record.validate().is_err());
    }

    #[test]
    fn normalizes_git_remotes_to_stable_identities() {
        assert_eq!(
            super::normalize_remote_identity("https://github.com/acme/my-repo.git"),
            "acme-my-repo"
        );
        assert_eq!(
            super::normalize_remote_identity("git@github.com:acme/my-repo.git"),
            "acme-my-repo"
        );
    }

    #[test]
    fn project_and_session_scopes_use_separate_directories() {
        assert_eq!(
            MemoryScope::Project("alpha".to_owned()).storage_path(),
            "projects/alpha"
        );
        assert_eq!(
            MemoryScope::Session("sessao-1".to_owned()).storage_path(),
            "sessions/sessao-1"
        );
    }
}
