use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use smeagol_harness_core::workspace::ProjectContext;
use smeagol_harness_core::{MemoryRecord, MemoryScope};
use uuid::Uuid;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS workspaces (id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE);
CREATE TABLE IF NOT EXISTS projects (
 id TEXT PRIMARY KEY, workspace_id TEXT NOT NULL REFERENCES workspaces(id),
 name TEXT NOT NULL, identity TEXT, UNIQUE(workspace_id, name));
CREATE UNIQUE INDEX IF NOT EXISTS project_identity ON projects(workspace_id, identity) WHERE identity IS NOT NULL;
CREATE TABLE IF NOT EXISTS pages (
 workspace_id TEXT NOT NULL, project_id TEXT NOT NULL, scope TEXT NOT NULL,
 id TEXT NOT NULL, relative_path TEXT NOT NULL, PRIMARY KEY(workspace_id, project_id, scope, id));
CREATE VIRTUAL TABLE IF NOT EXISTS memory_fts USING fts5(
 workspace_id UNINDEXED, project_id UNINDEXED, scope UNINDEXED, id UNINDEXED,
 title, body, tokenize='unicode61 remove_diacritics 2');";

/// Stable storage coordinates, independent of human-readable project names.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectNamespace {
    pub workspace_id: String,
    pub project_id: String,
    pub workspace: String,
    pub project: String,
}

#[derive(Serialize, Deserialize)]
struct WorkspaceManifest {
    id: String,
    name: String,
}

#[derive(Serialize, Deserialize)]
struct ProjectManifest {
    id: String,
    workspace_id: String,
    name: String,
    identity: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct PageHeader {
    id: String,
    title: String,
    scope: String,
}

/// Markdown source and search index share stable namespace UUIDs.
/// Page and namespace mutations hold an interprocess lock; legacy data is never imported.
#[derive(Clone)]
pub struct MemoryStore {
    root: PathBuf,
    workspace: String,
    project: String,
    repository_identity: Option<String>,
}

impl MemoryStore {
    /// Select storage without reading configuration or creating files.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            workspace: "default".into(),
            project: "default".into(),
            repository_identity: None,
        }
    }

    /// Use a resolved context for project, session, and profile data.
    pub fn with_context(mut self, context: ProjectContext) -> Self {
        self.workspace = context.workspace;
        self.project = context.project;
        self.repository_identity = context.repository_identity;
        self
    }

    /// Select a workspace without changing the current project identity.
    pub fn with_workspace(mut self, workspace: impl Into<String>) -> Self {
        self.workspace = workspace.into();
        self
    }

    /// Return the configured storage root without creating it.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Create the namespace index without opening or migrating the legacy DB.
    pub fn initialize(&self) -> Result<()> {
        let _guard = self.write_lock()?;
        let mut connection = self.open_index()?;
        self.recover_empty_index(&mut connection)
    }

    /// Resolve stable UUIDs; canonical repository identity takes precedence over names.
    pub fn namespace(&self, project: &str) -> Result<ProjectNamespace> {
        validate_name(&self.workspace)?;
        validate_name(project)?;
        let _guard = self.write_lock()?;
        let mut connection = self.open_index()?;
        self.recover_empty_index(&mut connection)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let workspace_id = transaction
            .query_row(
                "SELECT id FROM workspaces WHERE name=?1",
                [&self.workspace],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .unwrap_or_else(|| Uuid::now_v7().to_string());
        transaction.execute(
            "INSERT OR IGNORE INTO workspaces(id,name) VALUES(?1,?2)",
            params![workspace_id, self.workspace],
        )?;
        let identity = if project == self.project {
            self.repository_identity.as_deref()
        } else {
            None
        };
        let by_identity =
            if let Some(identity) = identity {
                transaction.query_row(
                "SELECT id,name,identity FROM projects WHERE workspace_id=?1 AND identity=?2",
                params![workspace_id, identity], project_row,
            ).optional()?
            } else {
                None
            };
        let (project_id, project_name, stored_identity) = if let Some(existing) = by_identity {
            existing
        } else {
            let mut name = project.to_owned();
            let mut suffix = 2;
            loop {
                let existing = transaction
                    .query_row(
                        "SELECT id,name,identity FROM projects WHERE workspace_id=?1 AND name=?2",
                        params![workspace_id, name],
                        project_row,
                    )
                    .optional()?;
                match existing {
                    Some((id, name, stored))
                        if identity.is_none() || stored.as_deref() == identity =>
                    {
                        break (id, name, stored);
                    }
                    Some((id, name, None)) => {
                        transaction.execute(
                            "UPDATE projects SET identity=?1 WHERE id=?2",
                            params![identity, id],
                        )?;
                        break (id, name, identity.map(str::to_owned));
                    }
                    Some(_) => {
                        name = format!("{project}-{suffix}");
                        suffix += 1;
                    }
                    None => {
                        let id = Uuid::now_v7().to_string();
                        transaction.execute("INSERT INTO projects(id,workspace_id,name,identity) VALUES(?1,?2,?3,?4)", params![id,workspace_id,name,identity])?;
                        break (id, name, identity.map(str::to_owned));
                    }
                }
            }
        };
        let namespace = ProjectNamespace {
            workspace_id,
            project_id,
            workspace: self.workspace.clone(),
            project: project_name,
        };
        self.write_manifests(&namespace, stored_identity)?;
        transaction.commit()?;
        Ok(namespace)
    }

    /// Return an isolated spool path without lossy session-name conversion.
    pub fn session_spool_path(&self, project: &str, session: &str) -> Result<PathBuf> {
        validate_component(session)?;
        let namespace = self.namespace(project)?;
        let path = self
            .root
            .join("spool/sessions")
            .join(namespace.workspace_id)
            .join(namespace.project_id)
            .join(format!("{session}.jsonl"));
        self.ensure_directory(path.parent().context("spool sem diretório pai")?)?;
        reject_symlink(&path)?;
        Ok(path)
    }

    /// Publish an individual Markdown page, then transactionally replace its index row.
    pub fn write(&self, record: &MemoryRecord) -> Result<PathBuf> {
        record.validate()?;
        let (project, scope, category) = self.scope_coordinates(&record.scope)?;
        let namespace = self.namespace(&project)?;
        let _guard = self.write_lock()?;
        let filename = if matches!(record.scope, MemoryScope::Session(_)) {
            match &record.scope {
                MemoryScope::Session(session) => session.as_str(),
                _ => unreachable!(),
            }
        } else {
            record.id.as_str()
        };
        let relative = format!("{category}/{filename}.md");
        let page = self.project_root(&namespace).join(&relative);
        let header = PageHeader {
            id: record.id.clone(),
            title: record.title.clone(),
            scope,
        };
        reject_symlink(&page)?;
        if page.exists() {
            let (previous, _): (PageHeader, _) = read_document(&page)?;
            ensure!(
                previous.id == header.id,
                "a página de sessão já pertence a outro registro"
            );
        }
        self.atomic_write(&page, encode_document(&header, &record.body)?.as_bytes())?;
        let mut connection = self.open_index()?;
        let transaction = connection.transaction()?;
        index_page(&transaction, &namespace, &relative, &header, &record.body)?;
        transaction.commit()?;
        Ok(page)
    }

    /// Search only within the resolved workspace, project, and logical scope.
    pub fn search(
        &self,
        query: &str,
        scope: &MemoryScope,
    ) -> Result<Vec<(String, String, String, String)>> {
        let terms = query
            .split(|ch: char| !ch.is_alphanumeric())
            .filter(|term| !term.is_empty())
            .map(|term| format!("\"{term}\"*"))
            .collect::<Vec<_>>();
        ensure!(
            !terms.is_empty(),
            "a consulta precisa conter ao menos uma palavra"
        );
        let (project, scope_key, _) = self.scope_coordinates(scope)?;
        let namespace = self.namespace(&project)?;
        let connection = self.open_index()?;
        let mut statement = connection.prepare(
            "SELECT id,title,snippet(memory_fts,5,'[',']','…',18) FROM memory_fts
             WHERE memory_fts MATCH ?1 AND workspace_id=?2 AND project_id=?3 AND scope=?4
             ORDER BY bm25(memory_fts,0.0,0.0,0.0,0.0,3.0,1.0),id LIMIT 20",
        )?;
        let rows = statement.query_map(
            params![
                terms.join(" OR "),
                namespace.workspace_id,
                namespace.project_id,
                scope_key
            ],
            |row| Ok((row.get(0)?, scope.storage_path(), row.get(1)?, row.get(2)?)),
        )?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("falha ao ler resultados")
    }

    /// Replace the project handoff with content bounded to 6,000 characters.
    pub fn publish_handoff(&self, project: &str, content: &str) -> Result<PathBuf> {
        let namespace = self.namespace(project)?;
        let _guard = self.write_lock()?;
        let path = self.handoff_path(&namespace);
        let mut chars = content.chars();
        let mut bounded: String = chars.by_ref().take(6_000).collect();
        if chars.next().is_some() {
            bounded.push_str("\n\n[handoff limitado]\n");
        }
        self.atomic_write(&path, bounded.as_bytes())?;
        Ok(path)
    }

    /// Consume at most one handoff within this workspace and project namespace.
    pub fn claim_handoff(&self, project: &str) -> Result<Option<String>> {
        let namespace = self.namespace(project)?;
        let _guard = self.write_lock()?;
        let path = self.handoff_path(&namespace);
        self.ensure_directory(path.parent().context("handoff sem diretório pai")?)?;
        reject_symlink(&path)?;
        match fs::read(&path) {
            Ok(bytes) => {
                fs::remove_file(&path)?;
                Ok(Some(String::from_utf8_lossy(&bytes).into_owned()))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => {
                Err(error).with_context(|| format!("falha ao ler handoff {}", path.display()))
            }
        }
    }

    /// Rename a project's display name without changing UUIDs or moving source pages.
    pub fn rename_project(&self, old: &str, new: &str) -> Result<()> {
        validate_name(old)?;
        validate_name(new)?;
        ensure!(
            old != "_profile" && new != "_profile",
            "nome reservado para o perfil"
        );
        let _guard = self.write_lock()?;
        let mut connection = self.open_index()?;
        self.recover_empty_index(&mut connection)?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (workspace_id,project_id,identity): (String,String,Option<String>) = transaction.query_row(
            "SELECT w.id,p.id,p.identity FROM projects p JOIN workspaces w ON w.id=p.workspace_id WHERE w.name=?1 AND p.name=?2",
            params![self.workspace,old], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
        ).context("projeto não encontrado")?;
        transaction.execute(
            "UPDATE projects SET name=?1 WHERE id=?2",
            params![new, project_id],
        )?;
        self.write_manifests(
            &ProjectNamespace {
                workspace_id,
                project_id,
                workspace: self.workspace.clone(),
                project: new.into(),
            },
            identity,
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Rebuild the index exclusively from UUID manifests and Markdown source pages.
    /// Legacy flat directories are ignored; malformed manifests abort the rebuild.
    pub fn reindex(&self) -> Result<usize> {
        let _guard = self.write_lock()?;
        let mut connection = self.open_index()?;
        self.rebuild_index(&mut connection)
    }

    fn recover_empty_index(&self, connection: &mut Connection) -> Result<()> {
        let count: i64 =
            connection.query_row("SELECT count(*) FROM workspaces", [], |row| row.get(0))?;
        // Source manifests own namespace identity. Rebuild before allocating any new UUID.
        if count == 0 && !uuid_directories(&self.root.join("wiki"))?.is_empty() {
            self.rebuild_index(connection)?;
        }
        Ok(())
    }

    fn rebuild_index(&self, connection: &mut Connection) -> Result<usize> {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute_batch("DELETE FROM memory_fts; DELETE FROM pages; DELETE FROM projects; DELETE FROM workspaces;")?;
        let mut count = 0;
        for workspace_root in uuid_directories(&self.root.join("wiki"))? {
            let (workspace, _): (WorkspaceManifest, _) =
                read_document(&workspace_root.join("_meta.md"))?;
            validate_uuid_directory(&workspace_root, &workspace.id)?;
            validate_name(&workspace.name)?;
            transaction.execute(
                "INSERT INTO workspaces(id,name) VALUES(?1,?2)",
                params![workspace.id, workspace.name],
            )?;
            for project_root in uuid_directories(&workspace_root)? {
                let (project, _): (ProjectManifest, _) =
                    read_document(&project_root.join("_meta.md"))?;
                validate_uuid_directory(&project_root, &project.id)?;
                ensure!(
                    project.workspace_id == workspace.id,
                    "manifesto de projeto fora do workspace correto"
                );
                validate_name(&project.name)?;
                transaction.execute(
                    "INSERT INTO projects(id,workspace_id,name,identity) VALUES(?1,?2,?3,?4)",
                    params![project.id, workspace.id, project.name, project.identity],
                )?;
                let namespace = ProjectNamespace {
                    workspace_id: workspace.id.clone(),
                    project_id: project.id,
                    workspace: workspace.name.clone(),
                    project: project.name,
                };
                for category in ["memories", "sessions"] {
                    let directory = project_root.join(category);
                    reject_symlink(&directory)?;
                    if !directory.exists() {
                        continue;
                    }
                    for entry in fs::read_dir(&directory)? {
                        let path = entry?.path();
                        reject_symlink(&path)?;
                        if path.extension().is_none_or(|ext| ext != "md") {
                            continue;
                        }
                        let (header, body): (PageHeader, _) = read_document(&path)?;
                        ensure!(
                            match category {
                                "sessions" =>
                                    header.scope.starts_with("session:")
                                        && namespace.project != "_profile",
                                "memories" if namespace.project == "_profile" =>
                                    header.scope == "profile",
                                "memories" => header.scope == "project",
                                _ => false,
                            },
                            "escopo da página incompatível com o namespace/diretório"
                        );
                        validate_component(&header.id)?;
                        let filename = if let Some(session) = header.scope.strip_prefix("session:")
                        {
                            validate_component(session)?;
                            session
                        } else {
                            header.id.as_str()
                        };
                        ensure!(
                            path.file_stem().and_then(|n| n.to_str()) == Some(filename),
                            "id da página difere do arquivo"
                        );
                        ensure!(
                            matches!(header.scope.as_str(), "project" | "profile")
                                || header.scope.starts_with("session:"),
                            "escopo de página inválido"
                        );
                        let relative = format!("{category}/{filename}.md");
                        index_page(&transaction, &namespace, &relative, &header, &body)?;
                        count += 1;
                    }
                }
            }
        }
        transaction.commit()?;
        Ok(count)
    }

    fn scope_coordinates(&self, scope: &MemoryScope) -> Result<(String, String, &'static str)> {
        match scope {
            MemoryScope::Profile => Ok(("_profile".into(), "profile".into(), "memories")),
            MemoryScope::Project(project) => {
                validate_name(project)?;
                ensure!(project != "_profile", "nome reservado para o perfil");
                Ok((project.clone(), "project".into(), "memories"))
            }
            MemoryScope::Session(session) => {
                validate_component(session)?;
                Ok((
                    self.project.clone(),
                    format!("session:{session}"),
                    "sessions",
                ))
            }
        }
    }

    fn project_root(&self, namespace: &ProjectNamespace) -> PathBuf {
        self.root
            .join("wiki")
            .join(&namespace.workspace_id)
            .join(&namespace.project_id)
    }

    fn handoff_path(&self, namespace: &ProjectNamespace) -> PathBuf {
        self.root
            .join("handoffs")
            .join(&namespace.workspace_id)
            .join(format!("{}.md", namespace.project_id))
    }

    fn write_manifests(
        &self,
        namespace: &ProjectNamespace,
        identity: Option<String>,
    ) -> Result<()> {
        let workspace_root = self.root.join("wiki").join(&namespace.workspace_id);
        let workspace = WorkspaceManifest {
            id: namespace.workspace_id.clone(),
            name: namespace.workspace.clone(),
        };
        let project = ProjectManifest {
            id: namespace.project_id.clone(),
            workspace_id: namespace.workspace_id.clone(),
            name: namespace.project.clone(),
            identity,
        };
        self.atomic_write_if_changed(
            &workspace_root.join("_meta.md"),
            encode_document(&workspace, "Workspace de memória.")?.as_bytes(),
        )?;
        self.atomic_write_if_changed(
            &self.project_root(namespace).join("_meta.md"),
            encode_document(&project, "Projeto de memória.")?.as_bytes(),
        )
    }

    fn open_index(&self) -> Result<Connection> {
        self.ensure_directory(&self.root.join("db"))?;
        self.ensure_directory(&self.root.join("wiki"))?;
        let path = self.root.join("db/workspaces.sqlite");
        reject_symlink(&path)?;
        let connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(10))?;
        connection.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL;")?;
        connection.execute_batch(SCHEMA)?;
        Ok(connection)
    }

    fn write_lock(&self) -> Result<File> {
        self.ensure_directory(&self.root.join("db"))?;
        let path = self.root.join("db/.write.lock");
        reject_symlink(&path)?;
        let guard = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)?;
        // Retain one persistent lock inode across writers and index rebuilds.
        guard.lock()?;
        Ok(guard)
    }

    fn ensure_directory(&self, path: &Path) -> Result<()> {
        let relative = path
            .strip_prefix(&self.root)
            .context("caminho fora do armazenamento")?;
        reject_symlink(&self.root)?;
        fs::create_dir_all(&self.root)?;
        let mut current = self.root.clone();
        for component in relative.components() {
            ensure!(
                matches!(component, Component::Normal(_)),
                "componente de caminho inválido"
            );
            current.push(component.as_os_str());
            reject_symlink(&current)?;
            if !current.exists() {
                match fs::create_dir(&current) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(e) => return Err(e.into()),
                }
            }
            ensure!(current.is_dir(), "diretório de armazenamento inválido");
        }
        Ok(())
    }

    fn atomic_write_if_changed(&self, path: &Path, bytes: &[u8]) -> Result<()> {
        reject_symlink(path)?;
        if fs::read(path).ok().as_deref() == Some(bytes) {
            return Ok(());
        }
        self.atomic_write(path, bytes)
    }

    fn atomic_write(&self, path: &Path, bytes: &[u8]) -> Result<()> {
        self.ensure_directory(path.parent().context("arquivo sem diretório pai")?)?;
        reject_symlink(path)?;
        let temporary = path.with_extension(format!("{}.tmp", Uuid::now_v7()));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        let result = (|| -> Result<()> {
            file.write_all(bytes)?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temporary, path)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result.with_context(|| format!("falha ao publicar {}", path.display()))
    }
}

fn project_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<(String, String, Option<String>)> {
    Ok((row.get(0)?, row.get(1)?, row.get(2)?))
}

fn index_page(
    connection: &Connection,
    namespace: &ProjectNamespace,
    relative: &str,
    header: &PageHeader,
    body: &str,
) -> Result<()> {
    connection.execute(
        "DELETE FROM memory_fts WHERE workspace_id=?1 AND project_id=?2 AND scope=?3 AND id=?4",
        params![
            namespace.workspace_id,
            namespace.project_id,
            header.scope,
            header.id
        ],
    )?;
    connection.execute("INSERT INTO memory_fts(workspace_id,project_id,scope,id,title,body) VALUES(?1,?2,?3,?4,?5,?6)",params![namespace.workspace_id,namespace.project_id,header.scope,header.id,header.title,body])?;
    connection.execute("INSERT INTO pages(workspace_id,project_id,scope,id,relative_path) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(workspace_id,project_id,scope,id) DO UPDATE SET relative_path=excluded.relative_path",params![namespace.workspace_id,namespace.project_id,header.scope,header.id,relative])?;
    Ok(())
}

fn encode_document<T: Serialize>(metadata: &T, body: &str) -> Result<String> {
    Ok(format!(
        "---\n{}\n---\n\n{body}",
        serde_json::to_string(metadata)?
    ))
}

fn read_document<T: DeserializeOwned>(path: &Path) -> Result<(T, String)> {
    reject_symlink(path)?;
    let text =
        fs::read_to_string(path).with_context(|| format!("falha ao ler {}", path.display()))?;
    let document = text.strip_prefix("---\n").context("frontmatter ausente")?;
    let (metadata, body) = document
        .split_once("\n---\n\n")
        .context("frontmatter inválido")?;
    Ok((
        serde_json::from_str(metadata).context("metadados inválidos")?,
        body.to_owned(),
    ))
}

fn validate_name(value: &str) -> Result<()> {
    smeagol_harness_core::workspace::validate_context_name(value)
}

fn validate_component(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value.len() <= 240
            && value
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_')),
        "identificador de arquivo inválido"
    );
    Ok(())
}

fn reject_symlink(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            bail!("link simbólico não permitido: {}", path.display())
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn uuid_directories(root: &Path) -> Result<Vec<PathBuf>> {
    reject_symlink(root)?;
    let mut paths = Vec::new();
    for entry in fs::read_dir(root)? {
        let path = entry?.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if Uuid::parse_str(name).is_err() {
            continue;
        }
        reject_symlink(&path)?;
        ensure!(path.is_dir(), "namespace UUID não é um diretório");
        paths.push(path);
    }
    Ok(paths)
}

fn validate_uuid_directory(path: &Path, id: &str) -> Result<()> {
    Uuid::parse_str(id).context("UUID do manifesto inválido")?;
    ensure!(
        path.file_name().and_then(|name| name.to_str()) == Some(id),
        "UUID difere do diretório do manifesto"
    );
    Ok(())
}

#[cfg(test)]
mod tests;
