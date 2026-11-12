use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::sync::LazyLock;

use anyhow::{Context, Result};
use fs2::FileExt;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Evento mínimo armazenado pelo adaptador de ciclo de vida.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Observation {
    pub event: String,
    pub session_id: String,
    pub project_id: String,
    pub excerpt: String,
    #[serde(default)]
    pub captured_at: String,
}

/// Converte o JSON de entrada de um hook Codex em uma observação mínima.
pub fn observation_from_codex(
    event: &str,
    payload: &Value,
    project_id: &str,
) -> Result<Observation> {
    let session_id = payload
        .get("session_id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .context("payload do Codex sem session_id")?;
    let tool = payload
        .get("tool_name")
        .or_else(|| payload.get("toolName"))
        .and_then(Value::as_str)
        .unwrap_or("unknown-tool");
    let excerpt = match event {
        "UserPromptSubmit" => payload
            .get("prompt")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        "PreToolUse" => format!("ferramenta iniciada: {tool}"),
        "PostToolUse" => format!("ferramenta concluída: {tool}"),
        "SessionStart" => format!(
            "sessão iniciada; origem: {}",
            payload
                .get("source")
                .and_then(Value::as_str)
                .unwrap_or("desconhecida")
        ),
        "SessionEnd" => format!(
            "sessão encerrada; motivo: {}",
            payload
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("desconhecido")
        ),
        "Stop" => "turno do agente encerrado".to_owned(),
        "PreCompact" => "compactação de contexto iniciada".to_owned(),
        _ => format!("evento de ciclo de vida: {event}"),
    };
    Ok(Observation {
        event: event.to_owned(),
        session_id: session_id.to_owned(),
        project_id: project_id.to_owned(),
        excerpt,
        captured_at: String::new(),
    })
}

/// Remove credenciais comuns antes da persistência local.
pub fn sanitize_excerpt(input: &str) -> String {
    static AUTHORIZATION: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)(authorization\s*[:=]\s*bearer\s+)[^\s,;}\]]+").unwrap());
    static ASSIGNMENT: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r#"(?i)(["']?[a-z0-9_.-]*(?:api[_-]?key|access[_-]?key|private[_-]?key|client[_-]?secret|secret|token|password|passwd|authorization)[a-z0-9_.-]*["']?\s*[:=]\s*)(?:"[^"]*"|'[^']*'|[^\s,;}\]]+)"#,
        )
        .unwrap()
    });
    static TOKEN: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"(?i)\b(?:sk-[a-z0-9_-]{8,}|gh[pousr]_[a-z0-9]{12,}|github_pat_[a-z0-9_]{12,}|xox[baprs]-[a-z0-9-]{12,}|akia[0-9a-z]{16}|eyj[a-z0-9_-]{10,}\.[a-z0-9_-]{10,}\.[a-z0-9_-]{10,})\b",
        )
        .unwrap()
    });
    let redacted = AUTHORIZATION.replace_all(input, "$1[REDACTED]");
    let redacted = ASSIGNMENT.replace_all(&redacted, "$1[REDACTED]");
    TOKEN.replace_all(&redacted, "[REDACTED]").into_owned()
}

/// Acrescenta um evento sanitizado ao spool JSONL.
pub fn append_observation(spool: &Path, mut observation: Observation) -> Result<()> {
    observation.excerpt = truncate_chars(&sanitize_excerpt(&observation.excerpt), 4_000);
    if observation.captured_at.is_empty() {
        observation.captured_at = format!("{}", unix_timestamp());
    }
    if let Some(parent) = spool.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("falha ao criar {}", parent.display()))?;
    }
    let mut record = serde_json::to_vec(&observation)?;
    record.push(b'\n');
    let lock_path = spool.with_extension(format!(
        "{}.lock",
        spool
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("jsonl")
    ));
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .with_context(|| format!("falha ao abrir lock {}", lock_path.display()))?;
    // Keep this OS-level lock held until the complete serialized record is
    // appended. Retain the lock file permanently: removing it can let Unix
    // writers lock different inodes under the same pathname.
    lock.lock_exclusive()
        .with_context(|| format!("falha ao bloquear spool {}", spool.display()))?;
    let mut file = OpenOptions::new().create(true).append(true).open(spool)?;
    file.write_all(&record)?;
    file.flush()?;
    Ok(())
}

/// Lê observações JSONL sem remover ou alterar o spool de origem.
pub fn read_observations(spool: &Path) -> Result<Vec<Observation>> {
    let file =
        fs::File::open(spool).with_context(|| format!("falha ao abrir {}", spool.display()))?;
    let mut observations = Vec::new();
    for (line_number, line) in BufReader::new(file).lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        observations.push(serde_json::from_str(&line).with_context(|| {
            format!(
                "JSON inválido na linha {} de {}",
                line_number + 1,
                spool.display()
            )
        })?);
    }
    Ok(observations)
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let prefix: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{prefix}…[trecho truncado]")
    } else {
        prefix
    }
}

fn unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::{Observation, append_observation, read_observations, sanitize_excerpt};

    #[test]
    fn sanitiza_segredos_inline_e_tokens_bearer() {
        let sanitized = sanitize_excerpt(
            "api_key=valor-secreto Authorization: Bearer abc123 ghp_12345678901234567890",
        );
        assert!(!sanitized.contains("valor-secreto"));
        assert!(!sanitized.contains("abc123"));
        assert!(!sanitized.contains("ghp_123"));
        assert!(sanitized.contains("[REDACTED]"));
        let more = sanitize_excerpt(
            r#"OPENAI_API_KEY=sk-1234567890 AWS_SECRET_ACCESS_KEY="aws-secret-value" {"GITHUB_TOKEN":"plain-token-value"}"#,
        );
        assert!(!more.contains("sk-123"));
        assert!(!more.contains("aws-secret-value"));
        assert!(!more.contains("plain-token-value"));
    }

    #[test]
    fn spool_guarda_evento_serializado_depois_da_sanitizacao() {
        let temp = tempfile::tempdir().unwrap();
        let spool = temp.path().join("spool/events.jsonl");
        append_observation(
            &spool,
            Observation {
                event: "prompt".to_owned(),
                session_id: "s1".to_owned(),
                project_id: "alpha".to_owned(),
                excerpt: "token=nao-gravar-este-valor".to_owned(),
                captured_at: String::new(),
            },
        )
        .unwrap();
        let stored = std::fs::read_to_string(spool).unwrap();
        assert!(stored.contains("[REDACTED]"));
        assert!(!stored.contains("nao-gravar-este-valor"));
        assert!(serde_json::from_str::<Observation>(stored.trim()).is_ok());
        let loaded = read_observations(&temp.path().join("spool/events.jsonl")).unwrap();
        assert_eq!(loaded.len(), 1);
        assert!(!loaded[0].captured_at.is_empty());
    }
}

#[cfg(test)]
mod codex_tests {
    use super::observation_from_codex;

    #[test]
    fn captura_prompt_mas_nao_grava_argumentos_de_ferramentas() {
        let prompt = serde_json::json!({
            "session_id": "s1",
            "cwd": "/repo",
            "prompt": "Corrija o erro de compilação"
        });
        let observation = observation_from_codex("UserPromptSubmit", &prompt, "acme-repo").unwrap();
        assert_eq!(observation.excerpt, "Corrija o erro de compilação");

        let tool = serde_json::json!({
            "session_id": "s1",
            "tool_name": "Bash",
            "tool_input": {"command": "echo segredo"}
        });
        let observation = observation_from_codex("PreToolUse", &tool, "acme-repo").unwrap();
        assert!(observation.excerpt.contains("Bash"));
        assert!(!observation.excerpt.contains("segredo"));
    }
}
