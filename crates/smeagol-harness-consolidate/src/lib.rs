use anyhow::{Result, bail};
use smeagol_harness_core::{MemoryRecord, MemoryScope};
use smeagol_harness_hooks::Observation;

/// Build a deterministic Markdown page from observations in the selected session.
pub fn compile_session(
    observations: &[Observation],
    project_id: &str,
    session_id: &str,
) -> Result<MemoryRecord> {
    let matching: Vec<&Observation> = observations
        .iter()
        .filter(|item| item.project_id == project_id && item.session_id == session_id)
        .collect();
    if matching.is_empty() {
        bail!("não há observações para a sessão e o projeto informados");
    }

    let scope_id = session_scope_id(project_id, session_id);
    let mut body = format!("Projeto: {project_id}\nSessão: {session_id}\n\n## Eventos\n");
    for observation in matching {
        body.push_str(&format!(
            "- [{}] {}: {}\n",
            observation.captured_at, observation.event, observation.excerpt
        ));
    }
    Ok(MemoryRecord {
        id: scope_id.clone(),
        scope: MemoryScope::Session(scope_id),
        title: format!("Sessão {session_id} — {project_id}"),
        body,
    })
}

/// Promote only statements that explicitly declare a durable rule.
/// Promotion is deliberately conservative: regular requests and tool results
/// must not be interpreted as durable decisions.
pub fn promote_durable_memories(
    observations: &[Observation],
    project_id: &str,
) -> Vec<MemoryRecord> {
    const SIGNALS: &[&str] = &[
        "lembre-se",
        "remember that",
        "a partir de agora",
        "sempre ",
        "nunca ",
        "prefiro ",
        "decidimos ",
        "decisão:",
        "padrão:",
        "convenção:",
        "use sempre",
        "não use",
    ];

    let mut records = Vec::new();
    for observation in observations.iter().filter(|item| {
        item.project_id == project_id
            && matches!(item.event.as_str(), "UserPromptSubmit" | "prompt")
    }) {
        for sentence in observation
            .excerpt
            .split(['\n', ';', '.', '!', '?'])
            .map(str::trim)
        {
            let normalized = sentence.to_lowercase();
            if sentence.chars().count() < 24
                || sentence.chars().count() > 240
                || !SIGNALS.iter().any(|signal| normalized.contains(signal))
            {
                continue;
            }

            let title = sentence.chars().take(72).collect::<String>();
            let id = format!("auto-{:016x}", stable_fingerprint(&normalized));
            records.push(MemoryRecord {
                id,
                scope: MemoryScope::Project(project_id.to_owned()),
                title,
                body: format!(
                    "Regra durável declarada explicitamente pelo usuário.\n\n{}",
                    sentence
                ),
            });
        }
    }
    records.sort_by(|left, right| left.id.cmp(&right.id));
    records.dedup_by(|left, right| left.id == right.id);
    records
}

fn stable_fingerprint(value: &str) -> u64 {
    // FNV-1a provides reproducible identifiers, not cryptographic protection.
    value.bytes().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    })
}

/// Build a stable path component for a project and session pair.
pub fn session_scope_id(project_id: &str, session_id: &str) -> String {
    let combined = format!("p-{project_id}-s-{session_id}");
    combined
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

#[cfg(test)]
mod tests {
    use super::{compile_session, promote_durable_memories, session_scope_id};
    use smeagol_harness_hooks::Observation;

    #[test]
    fn consolidates_only_events_from_the_selected_session_and_project() {
        let observations = vec![
            Observation {
                event: "prompt".to_owned(),
                session_id: "s1".to_owned(),
                project_id: "alpha".to_owned(),
                excerpt: "Implementar busca local".to_owned(),
                captured_at: "100".to_owned(),
            },
            Observation {
                event: "prompt".to_owned(),
                session_id: "s2".to_owned(),
                project_id: "alpha".to_owned(),
                excerpt: "Outro trabalho".to_owned(),
                captured_at: "101".to_owned(),
            },
        ];
        let page = compile_session(&observations, "alpha", "s1").unwrap();
        assert!(page.body.contains("Implementar busca local"));
        assert!(!page.body.contains("Outro trabalho"));
        assert_eq!(page.id, session_scope_id("alpha", "s1"));
    }

    #[test]
    fn returns_an_error_when_the_session_has_no_observations() {
        assert!(compile_session(&[], "alpha", "s1").is_err());
    }

    #[test]
    fn promotes_explicit_durable_rules_and_deduplicates_them() {
        let prompt = Observation {
            event: "UserPromptSubmit".to_owned(),
            session_id: "s1".to_owned(),
            project_id: "alpha".to_owned(),
            excerpt: "Sempre mantenha os comentários do código em português; Sempre mantenha os comentários do código em português".to_owned(),
            captured_at: "100".to_owned(),
        };
        let records = promote_durable_memories(&[prompt], "alpha");
        assert_eq!(records.len(), 1);
        assert!(
            records[0]
                .body
                .contains("comentários do código em português")
        );
    }

    #[test]
    fn does_not_promote_regular_requests_or_content_from_other_projects() {
        let prompt = Observation {
            event: "UserPromptSubmit".to_owned(),
            session_id: "s1".to_owned(),
            project_id: "beta".to_owned(),
            excerpt: "Investigue este erro de compilação e explique a causa provável.".to_owned(),
            captured_at: "100".to_owned(),
        };
        assert!(promote_durable_memories(&[prompt], "alpha").is_empty());
    }
}
