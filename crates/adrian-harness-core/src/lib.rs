use std::fmt;
use std::path::Path;
use std::process::Command;

/// Escopo de uma memória persistida.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MemoryScope {
    Profile,
    Project(String),
    Session(String),
}

impl MemoryScope {
    /// Devolve o diretório relativo usado para guardar este escopo.
    pub fn storage_path(&self) -> String {
        match self {
            Self::Profile => "profile".to_owned(),
            Self::Project(project) => format!("projects/{}", safe_identifier(project)),
            Self::Session(session) => format!("sessions/{}", safe_identifier(session)),
        }
    }
}

/// Registro legível que pode ser salvo no Markdown e indexado para busca.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryRecord {
    pub id: String,
    pub scope: MemoryScope,
    pub title: String,
    pub body: String,
}

impl MemoryRecord {
    /// Valida campos que participam de caminhos ou do cabeçalho Markdown.
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

/// Erro de validação de dados de domínio.
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

/// Estima tokens sem assumir um tokenizer de provedor específico.
/// A estimativa é aproximada e serve para impor um limite conservador ao briefing.
pub fn estimate_tokens(text: &str) -> usize {
    text.split_whitespace()
        .map(|word| word.chars().count().div_ceil(4).max(1))
        .sum()
}

/// Deriva um identificador estável do remoto Git; usa o nome do diretório como fallback.
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

/// Normaliza URLs HTTPS e SSH para um identificador sem host nem sufixo `.git`.
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
    fn aceita_registro_com_escopo_valido() {
        let record = MemoryRecord {
            id: "decisao-cache".to_owned(),
            scope: MemoryScope::Project("meu-projeto".to_owned()),
            title: "Cache local".to_owned(),
            body: "Usar cache local para reduzir chamadas repetidas.".to_owned(),
        };
        assert!(record.validate().is_ok());
    }

    #[test]
    fn rejeita_componentes_que_poderiam_sair_da_arvore_de_dados() {
        let record = MemoryRecord {
            id: "../../fora".to_owned(),
            scope: MemoryScope::Project("meu-projeto".to_owned()),
            title: "Inválido".to_owned(),
            body: String::new(),
        };
        assert!(record.validate().is_err());
    }

    #[test]
    fn normaliza_remotos_git_para_identidades_estaveis() {
        assert_eq!(
            super::normalize_remote_identity("https://github.com/acme/my-repo.git"),
            "acme-my-repo"
        );
        assert_eq!(
            super::normalize_remote_identity("git@github.com:akitaonrails/ai-memory.git"),
            "akitaonrails-ai-memory"
        );
    }

    #[test]
    fn escopos_de_projeto_e_sessao_usam_diretorios_separados() {
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
