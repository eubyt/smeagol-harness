use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, params};
use smeagol_harness_core::{MemoryRecord, MemoryScope};

/// Armazenamento local: Markdown é a fonte e SQLite é um índice derivado.
pub struct MemoryStore {
    root: PathBuf,
}

impl MemoryStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Cria as pastas de dados e prepara a tabela de busca textual.
    pub fn initialize(&self) -> Result<()> {
        for dir in ["wiki/profile", "wiki/projects", "wiki/sessions", "db"] {
            fs::create_dir_all(self.root.join(dir)).with_context(|| {
                format!("não foi possível criar {}", self.root.join(dir).display())
            })?;
        }
        let connection = self.open_index()?;
        connection.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS memory_fts
             USING fts5(id UNINDEXED, scope UNINDEXED, title, body, tokenize='unicode61 remove_diacritics 2');",
        )?;
        Ok(())
    }

    /// Salva primeiro a página Markdown e depois atualiza o índice reconstruível.
    pub fn write(&self, record: &MemoryRecord) -> Result<PathBuf> {
        record.validate()?;
        self.initialize()?;
        let relative = format!("{}.md", record.scope.storage_path());
        let page = self.root.join("wiki").join(relative);
        let parent = page.parent().context("página sem diretório pai")?;
        fs::create_dir_all(parent)?;
        let markdown = format!(
            "---\nid: {}\ntitle: {:?}\n---\n\n{}\n",
            record.id, record.title, record.body
        );
        let temporary = page.with_extension("md.tmp");
        fs::write(&temporary, markdown)
            .with_context(|| format!("falha ao gravar {}", temporary.display()))?;
        fs::rename(&temporary, &page)
            .with_context(|| format!("falha ao publicar {}", page.display()))?;

        let scope = record.scope.storage_path();
        let connection = self.open_index()?;
        connection.execute(
            "DELETE FROM memory_fts WHERE id = ?1 AND scope = ?2",
            params![record.id, scope],
        )?;
        connection.execute(
            "INSERT INTO memory_fts (id, scope, title, body) VALUES (?1, ?2, ?3, ?4)",
            params![record.id, scope, record.title, record.body],
        )?;
        Ok(page)
    }

    /// Busca texto no índice; o Markdown continua sendo a fonte de verdade.
    pub fn search(
        &self,
        query: &str,
        scope: &MemoryScope,
    ) -> Result<Vec<(String, String, String, String)>> {
        if query.trim().is_empty() {
            bail!("a consulta não pode ser vazia");
        }
        let connection = self.open_index()?;
        let terms = query
            .split(|ch: char| !ch.is_alphanumeric())
            .filter(|term| !term.is_empty())
            .map(|term| format!("\"{}\"*", term.replace('"', "")))
            .collect::<Vec<_>>();
        if terms.is_empty() {
            bail!("a consulta precisa conter ao menos uma palavra");
        }
        // OR tolera consultas em linguagem natural e o prefixo encontra flexões simples.
        let match_query = terms.join(" OR ");
        let mut statement = connection.prepare(
            "SELECT id, scope, title, snippet(memory_fts, 3, '[', ']', '…', 18)
             FROM memory_fts WHERE memory_fts MATCH ?1 AND scope = ?2
             ORDER BY bm25(memory_fts, 0.0, 0.0, 3.0, 1.0), id LIMIT 20",
        )?;
        let scope_path = scope.storage_path();
        let rows = statement.query_map(params![match_query, scope_path], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("falha ao ler resultados")
    }

    /// Publica o handoff mais recente do projeto por substituição atômica.
    pub fn publish_handoff(&self, project_id: &str, content: &str) -> Result<PathBuf> {
        let directory = self.root.join("handoffs");
        fs::create_dir_all(&directory)?;
        let name = safe_filename(project_id);
        let destination = directory.join(format!("{name}.md"));
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let temporary = directory.join(format!(".{name}.{}.{}.tmp", std::process::id(), nonce));
        let content = truncate_chars(content, 6_000);
        fs::write(&temporary, content)?;
        fs::rename(&temporary, &destination)
            .with_context(|| format!("falha ao publicar handoff {}", destination.display()))?;
        Ok(destination)
    }

    /// Retira atomicamente um handoff para que uma só sessão o receba.
    pub fn claim_handoff(&self, project_id: &str) -> Result<Option<String>> {
        let directory = self.root.join("handoffs");
        let name = safe_filename(project_id);
        let source = directory.join(format!("{name}.md"));
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let claimed = directory.join(format!(".{name}.{}.{}.claimed", std::process::id(), nonce));
        match fs::rename(&source, &claimed) {
            Ok(()) => {
                let content = fs::read_to_string(&claimed)
                    .with_context(|| format!("falha ao ler handoff {}", claimed.display()))?;
                fs::remove_file(&claimed)?;
                Ok(Some(content))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => {
                Err(error).with_context(|| format!("falha ao reivindicar {}", source.display()))
            }
        }
    }

    fn open_index(&self) -> Result<Connection> {
        let path = self.root.join("db/memory.sqlite");
        let connection = Connection::open(&path)
            .with_context(|| format!("não foi possível abrir o índice {}", path.display()))?;
        Ok(connection)
    }
}

/// Resolve o diretório de dados seguindo XDG e depois o diretório do usuário.
pub fn default_data_dir() -> PathBuf {
    if let Some(path) =
        std::env::var_os("SMEAGOL_HARNESS_HOME").or_else(|| std::env::var_os("ADRIAN_HARNESS_HOME"))
    {
        return PathBuf::from(path);
    }
    if let Some(path) = std::env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(path).join("smeagol-harness");
    }
    #[cfg(windows)]
    {
        if let Some(path) = std::env::var_os("LOCALAPPDATA") {
            return PathBuf::from(path).join("SmeagolHarness");
        }
        if let Some(path) = std::env::var_os("USERPROFILE") {
            return PathBuf::from(path).join("AppData/Local/SmeagolHarness");
        }
    }
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".local/share/smeagol-harness")
}

fn safe_filename(value: &str) -> String {
    let mut output = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            output.push(ch.to_ascii_lowercase());
        } else if !output.ends_with('-') {
            output.push('-');
        }
    }
    output.trim_matches('-').to_owned()
}

fn truncate_chars(value: &str, limit: usize) -> String {
    let mut chars = value.chars();
    let prefix: String = chars.by_ref().take(limit).collect();
    if chars.next().is_some() {
        format!("{prefix}\n\n[handoff limitado]\n")
    } else {
        prefix
    }
}

#[cfg(test)]
mod tests {
    use super::MemoryStore;
    use smeagol_harness_core::{MemoryRecord, MemoryScope};

    #[test]
    fn grava_markdown_e_indexa_conteudo_para_busca() {
        let temp = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(temp.path());
        let record = MemoryRecord {
            id: "cache".to_owned(),
            scope: MemoryScope::Project("alpha".to_owned()),
            title: "Decisão de cache".to_owned(),
            body: "Usar cache local para reduzir chamadas repetidas.".to_owned(),
        };

        let page = store.write(&record).unwrap();
        let results = store
            .search("chamadas", &MemoryScope::Project("alpha".to_owned()))
            .unwrap();

        assert!(page.exists());
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "cache");
        assert_eq!(results[0].2, "Decisão de cache");
    }

    #[test]
    fn busca_nao_vaza_resultados_de_outro_escopo() {
        let temp = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(temp.path());
        for (id, project) in [("nota-a", "alpha"), ("nota-b", "beta")] {
            store
                .write(&MemoryRecord {
                    id: id.to_owned(),
                    scope: MemoryScope::Project(project.to_owned()),
                    title: "Banco de dados".to_owned(),
                    body: "SQLite mantém o índice local.".to_owned(),
                })
                .unwrap();
        }

        let results = store
            .search("SQLite", &MemoryScope::Project("alpha".to_owned()))
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "nota-a");
    }

    #[test]
    fn busca_aceita_linguagem_natural_acentos_e_pontuacao() {
        let temp = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(temp.path());
        store
            .write(&MemoryRecord {
                id: "decisao".to_owned(),
                scope: MemoryScope::Project("alpha".to_owned()),
                title: "Decisão de cache".to_owned(),
                body: "Usar cache local para reduzir chamadas repetidas.".to_owned(),
            })
            .unwrap();

        let results = store
            .search("cache, decisão!", &MemoryScope::Project("alpha".to_owned()))
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "decisao");
    }

    #[test]
    fn handoff_e_entregue_uma_vez_e_fica_limitado() {
        let temp = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(temp.path());
        store
            .publish_handoff("alpha", "retomar implementação")
            .unwrap();
        assert_eq!(
            store.claim_handoff("alpha").unwrap().as_deref(),
            Some("retomar implementação")
        );
        assert_eq!(store.claim_handoff("alpha").unwrap(), None);
    }

    #[test]
    fn recusa_consulta_vazia() {
        let temp = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(temp.path());
        store.initialize().unwrap();
        assert!(store.search("  ", &MemoryScope::Profile).is_err());
    }
}
