use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use serde_json::{Map, Value, json};

/// Plano somente leitura para a integração com Codex.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetupPlan {
    pub codex_home: PathBuf,
    pub config_file: PathBuf,
    pub hooks_file: PathBuf,
    pub codex_detected: bool,
    pub actions: Vec<String>,
}

/// Detecta os caminhos do Codex e descreve ações sem alterar arquivos.
pub fn plan_codex() -> SetupPlan {
    let codex_home = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| user_home_dir().map(|home| home.join(".codex")))
        .unwrap_or_else(|| PathBuf::from(".codex"));
    plan_codex_at(codex_home)
}

/// Constrói o plano para um diretório explícito, sem consultar nem alterar o ambiente.
pub fn plan_codex_at(codex_home: PathBuf) -> SetupPlan {
    let config_file = codex_home.join("config.toml");
    let hooks_file = codex_home.join("hooks.json");
    let codex_detected = config_file.exists() || hooks_file.exists();
    let actions = if codex_detected {
        vec![
            format!("adicionar hooks gerenciados a {}", hooks_file.display()),
            "preservar eventos e handlers existentes".to_owned(),
            "revisar e confiar nos hooks no Codex antes da primeira execução".to_owned(),
        ]
    } else {
        vec![
            format!("criar {}", hooks_file.display()),
            "instalar hooks de sessão, prompt, ferramenta e encerramento".to_owned(),
            "revisar e confiar nos hooks no Codex antes da primeira execução".to_owned(),
        ]
    };
    SetupPlan {
        codex_home,
        config_file,
        hooks_file,
        codex_detected,
        actions,
    }
}

const MANAGED_MARKER: &str = "Smeagol Harness managed:";
const EVENTS: &[&str] = &[
    "SessionStart",
    "UserPromptSubmit",
    "PostToolUse",
    "Stop",
    "SessionEnd",
];

/// Mescla os hooks do Smeagol Harness mantendo outros eventos e handlers.
pub fn apply_codex_at(codex_home: PathBuf, binary: &std::path::Path) -> Result<SetupPlan> {
    let plan = plan_codex_at(codex_home);
    if let Ok(metadata) = fs::symlink_metadata(&plan.hooks_file)
        && metadata.file_type().is_symlink()
    {
        bail!(
            "{} é um link simbólico; recusando alterar um destino indireto",
            plan.hooks_file.display()
        );
    }
    let mut document = if plan.hooks_file.exists() {
        let bytes = fs::read(&plan.hooks_file)?;
        serde_json::from_slice::<Value>(&bytes).with_context(|| {
            format!(
                "{} não contém JSON válido; nenhum arquivo foi alterado",
                plan.hooks_file.display()
            )
        })?
    } else {
        json!({})
    };
    let root = document
        .as_object_mut()
        .context("a raiz de hooks.json precisa ser um objeto")?;
    let hooks_value = root.entry("hooks".to_owned()).or_insert_with(|| json!({}));
    let hooks = hooks_value
        .as_object_mut()
        .context("a propriedade 'hooks' precisa ser um objeto")?;

    for event in EVENTS {
        let groups = hooks
            .entry((*event).to_owned())
            .or_insert_with(|| json!([]));
        let groups = groups
            .as_array_mut()
            .with_context(|| format!("hooks.{event} precisa ser uma lista"))?;
        groups.retain_mut(|group| {
            let Some(handlers) = group.get_mut("hooks").and_then(Value::as_array_mut) else {
                return true;
            };
            handlers.retain(|handler| !is_managed_handler(handler));
            !handlers.is_empty()
        });
        let command = format!(
            "{} hook --event {}",
            shell_quote(&binary.display().to_string()),
            event
        );
        let mut handler = Map::new();
        handler.insert("type".to_owned(), json!("command"));
        handler.insert("command".to_owned(), json!(command));
        handler.insert(
            "statusMessage".to_owned(),
            json!(format!("{MANAGED_MARKER} {event}")),
        );
        if cfg!(windows) {
            handler.insert(
                "commandWindows".to_owned(),
                json!(format!("\"{}\" hook --event {}", binary.display(), event)),
            );
        }
        groups.push(json!({"hooks": [Value::Object(handler)]}));
    }

    fs::create_dir_all(&plan.codex_home)
        .with_context(|| format!("falha ao criar {}", plan.codex_home.display()))?;
    let bytes = serde_json::to_vec_pretty(&document)?;
    atomic_replace(&plan.hooks_file, &bytes)?;
    Ok(plan)
}

fn is_managed_handler(handler: &Value) -> bool {
    handler
        .get("statusMessage")
        .and_then(Value::as_str)
        .is_some_and(|message| message.starts_with(MANAGED_MARKER))
}

fn shell_quote(value: &str) -> String {
    #[cfg(windows)]
    {
        format!("\"{}\"", value.replace('"', "\\\""))
    }
    #[cfg(not(windows))]
    {
        format!("'{}'", value.replace('\'', "'\"'\"'"))
    }
}

/// Resolve a user home directory across Unix and Windows environments.
pub fn user_home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .or_else(
            || match (std::env::var_os("HOMEDRIVE"), std::env::var_os("HOMEPATH")) {
                (Some(drive), Some(path)) => {
                    Some(format!("{}{}", drive.to_string_lossy(), path.to_string_lossy()).into())
                }
                _ => None,
            },
        )
        .map(PathBuf::from)
}

fn managed_handler(event: &str, agent: &str, binary: &std::path::Path) -> Value {
    let name = format!("smeagol-harness:{agent}:{event}");
    let command = if agent == "antigravity" {
        antigravity_command(binary, event)
    } else {
        format!(
            "{} hook --agent {agent} --event {event}",
            shell_quote(&binary.display().to_string())
        )
    };
    json!({"name": name, "type": "command", "command": command, "timeout": 5000})
}

fn antigravity_command(binary: &std::path::Path, event: &str) -> String {
    let executable = binary.display().to_string();
    #[cfg(windows)]
    {
        // Antigravity's Windows hook runner passes the command through a shell
        // that preserves quotes around the executable as part of its name.
        // Avoid them for the normal Cargo install path (which has no spaces).
        executable.trim_matches('"').to_owned() + &format!(" hook --agent antigravity --event {event}")
    }
    #[cfg(not(windows))]
    {
        format!(
            "{} hook --agent antigravity --event {event}",
            shell_quote(&executable)
        )
    }
}

/// Merge native Gemini CLI hooks into the user's global settings.
pub fn apply_gemini_at(gemini_home: PathBuf, binary: &std::path::Path) -> Result<PathBuf> {
    let settings_file = gemini_home.join("settings.json");
    let mut document = read_json_object(&settings_file)?;
    let root = document
        .as_object_mut()
        .context("settings.json precisa conter um objeto")?;
    let hooks = root
        .entry("hooks")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .context("settings.json: 'hooks' precisa ser um objeto")?;
    for (event, agent_event) in [
        ("SessionStart", "SessionStart"),
        ("SessionEnd", "SessionEnd"),
        ("BeforeAgent", "BeforeAgent"),
        ("BeforeTool", "BeforeTool"),
        ("AfterTool", "AfterTool"),
        ("PreCompress", "PreCompress"),
    ] {
        let groups = hooks
            .entry(event.to_owned())
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .with_context(|| format!("hooks.{event} precisa ser uma lista"))?;
        for group in groups.iter_mut() {
            if let Some(handlers) = group.get_mut("hooks").and_then(Value::as_array_mut) {
                handlers.retain(|handler| {
                    !handler
                        .get("name")
                        .and_then(Value::as_str)
                        .is_some_and(|name| name.starts_with("smeagol-harness:gemini-cli:"))
                });
            }
        }
        groups.retain(|group| {
            group
                .get("hooks")
                .and_then(Value::as_array)
                .is_none_or(|hooks| !hooks.is_empty())
        });
        let handler = managed_handler(agent_event, "gemini-cli", binary);
        if matches!(event, "BeforeTool" | "AfterTool") {
            groups.push(json!({"matcher":".*", "hooks":[handler]}));
        } else {
            groups.push(json!({"hooks":[handler]}));
        }
    }
    write_json_atomic(&settings_file, &document)?;
    Ok(settings_file)
}

/// Merge native Antigravity CLI hooks into `~/.gemini/config/hooks.json`.
pub fn apply_antigravity_at(gemini_home: PathBuf, binary: &std::path::Path) -> Result<PathBuf> {
    let hooks_file = gemini_home.join("config/hooks.json");
    let mut document = read_json_object(&hooks_file)?;
    let root = document
        .as_object_mut()
        .context("hooks.json precisa conter um objeto")?;
    let key = "smeagol-harness-managed";
    if let Some(existing) = root.get(key)
        && !is_our_antigravity_group(existing)
    {
        bail!(
            "hooks.json já contém o grupo '{key}', que não pertence ao Smeagol Harness; renomeie-o antes de instalar"
        );
    }
    let mut group = serde_json::Map::new();
    for event in [
        "SessionStart",
        "PreInvocation",
        "PreToolUse",
        "PostToolUse",
        "Stop",
    ] {
        let handler = managed_handler(event, "antigravity", binary);
        if matches!(event, "PreToolUse" | "PostToolUse") {
            group.insert(
                event.to_owned(),
                json!([{"matcher":".*", "hooks":[handler]}]),
            );
        } else {
            group.insert(event.to_owned(), json!([handler]));
        }
    }
    root.insert(key.to_owned(), Value::Object(group));
    write_json_atomic(&hooks_file, &document)?;
    Ok(hooks_file)
}

fn is_our_antigravity_group(group: &Value) -> bool {
    group.as_object().is_some_and(|events| {
        events.values().all(|value| {
            value.as_array().is_some_and(|entries| {
                entries.iter().all(|entry| {
                    let nested_hooks = entry.get("hooks").and_then(Value::as_array);
                    let hooks: Vec<&Value> = nested_hooks
                        .map(|hooks| hooks.iter().collect())
                        .unwrap_or_else(|| vec![entry]);
                    hooks.iter().all(|hook| {
                        hook.get("name")
                            .and_then(Value::as_str)
                            .is_some_and(|name| name.starts_with("smeagol-harness:antigravity:"))
                    })
                })
            })
        })
    })
}

fn read_json_object(path: &std::path::Path) -> Result<Value> {
    if path.exists() {
        let bytes = fs::read(path)?;
        serde_json::from_slice::<Value>(&bytes).with_context(|| {
            format!(
                "{} não contém JSON válido; nenhum arquivo foi alterado",
                path.display()
            )
        })
    } else {
        Ok(json!({}))
    }
}

fn write_json_atomic(path: &std::path::Path, document: &Value) -> Result<()> {
    let parent = path
        .parent()
        .context("arquivo de configuração sem diretório pai")?;
    fs::create_dir_all(parent).with_context(|| format!("falha ao criar {}", parent.display()))?;
    let bytes = serde_json::to_vec_pretty(document)?;
    atomic_replace(path, &bytes)
}

fn atomic_replace(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    let temporary = path.with_extension(format!("json.{}.tmp", std::process::id()));
    fs::write(&temporary, bytes)
        .with_context(|| format!("falha ao gravar {}", temporary.display()))?;

    #[cfg(not(windows))]
    {
        fs::rename(&temporary, path)
            .with_context(|| format!("falha ao publicar {}", path.display()))?;
    }
    #[cfg(windows)]
    {
        if path.exists() {
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let backup =
                path.with_extension(format!("json.{}.{}.backup", std::process::id(), nonce));
            fs::rename(path, &backup)
                .with_context(|| format!("falha ao preservar {}", path.display()))?;
            if let Err(error) = fs::rename(&temporary, path) {
                let _ = fs::rename(&backup, path);
                let _ = fs::remove_file(&temporary);
                return Err(error).with_context(|| format!("falha ao publicar {}", path.display()));
            }
            fs::remove_file(backup)?;
        } else {
            fs::rename(&temporary, path)
                .with_context(|| format!("falha ao publicar {}", path.display()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{apply_antigravity_at, apply_codex_at, apply_gemini_at, plan_codex_at};
    use serde_json::Value;

    #[test]
    fn plano_nao_cria_arquivos_no_diretorio_detectado() {
        let temp = tempfile::tempdir().unwrap();
        let plan = plan_codex_at(temp.path().to_path_buf());
        assert!(!plan.codex_detected);
        assert!(!plan.hooks_file.exists());
        assert!(!plan.config_file.exists());
    }

    #[test]
    fn apply_preserva_hooks_de_terceiros_e_e_idempotente() {
        let temp = tempfile::tempdir().unwrap();
        let hooks = temp.path().join("hooks.json");
        std::fs::write(
            &hooks,
            r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"echo third-party"}]}]}}"#,
        )
        .unwrap();
        let binary = temp.path().join("bin with spaces/smeagol-harness");
        std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
        std::fs::write(&binary, "binary fixture").unwrap();

        apply_codex_at(temp.path().to_path_buf(), &binary).unwrap();
        let first: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&hooks).unwrap()).unwrap();
        apply_codex_at(temp.path().to_path_buf(), &binary).unwrap();
        let second: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&hooks).unwrap()).unwrap();

        assert_eq!(first, second);
        assert_eq!(second["hooks"]["Stop"].as_array().unwrap().len(), 2);
        assert_eq!(
            second["hooks"]["Stop"][0]["hooks"][0]["command"],
            "echo third-party"
        );
    }

    #[test]
    fn apply_remove_somente_o_handler_proprio_dentro_de_grupo_compartilhado() {
        let temp = tempfile::tempdir().unwrap();
        let hooks = temp.path().join("hooks.json");
        std::fs::write(
            &hooks,
            r#"{"hooks":{"Stop":[{"matcher":"Bash","hooks":[{"type":"command","command":"echo terceiro"},{"type":"command","command":"old","statusMessage":"Smeagol Harness managed: Stop"}]}]}}"#,
        )
        .unwrap();
        let binary = temp.path().join("smeagol-harness");
        std::fs::write(&binary, "fixture").unwrap();
        apply_codex_at(temp.path().to_path_buf(), &binary).unwrap();
        let document: serde_json::Value =
            serde_json::from_slice(&std::fs::read(hooks).unwrap()).unwrap();
        let stop = document["hooks"]["Stop"].as_array().unwrap();
        assert!(stop.iter().any(|group| group["matcher"] == "Bash"));
        assert!(
            stop.iter()
                .flat_map(|group| group["hooks"].as_array().into_iter().flatten())
                .any(|handler| handler["command"] == "echo terceiro")
        );
    }

    #[test]
    fn reconhece_configuracao_codex_existente_sem_muda_la() {
        let temp = tempfile::tempdir().unwrap();
        let config = temp.path().join("config.toml");
        std::fs::write(&config, "model = \"codex-mini\"\n").unwrap();
        let before = std::fs::read(&config).unwrap();

        let plan = plan_codex_at(temp.path().to_path_buf());

        assert!(plan.codex_detected);
        assert_eq!(std::fs::read(config).unwrap(), before);
        assert!(!plan.hooks_file.exists());
    }

    #[test]
    fn instala_hooks_gemini_sem_apagar_configuracao_e_e_idempotente() {
        let temp = tempfile::tempdir().unwrap();
        let settings = temp.path().join("settings.json");
        std::fs::write(
            &settings,
            r#"{"theme":"dark","hooks":{"BeforeAgent":[{"hooks":[{"name":"third-party","type":"command","command":"echo keep"}]}]}}"#,
        )
        .unwrap();
        let binary = temp.path().join("bin with spaces/smeagol-harness.exe");
        std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
        std::fs::write(&binary, "fixture").unwrap();

        apply_gemini_at(temp.path().to_path_buf(), &binary).unwrap();
        let first: Value = serde_json::from_slice(&std::fs::read(&settings).unwrap()).unwrap();
        apply_gemini_at(temp.path().to_path_buf(), &binary).unwrap();
        let second: Value = serde_json::from_slice(&std::fs::read(&settings).unwrap()).unwrap();

        assert_eq!(first, second);
        assert_eq!(second["theme"], "dark");
        let before_agent = second["hooks"]["BeforeAgent"].as_array().unwrap();
        assert!(
            before_agent
                .iter()
                .flat_map(|group| group["hooks"].as_array().into_iter().flatten())
                .any(|hook| hook["name"] == "third-party")
        );
        assert!(second["hooks"]["BeforeTool"].as_array().unwrap()[0]["matcher"] == ".*");
    }

    #[test]
    fn instala_hooks_antigravity_em_grupo_proprio_e_preserva_os_demais() {
        let temp = tempfile::tempdir().unwrap();
        let hooks = temp.path().join("config/hooks.json");
        std::fs::create_dir_all(hooks.parent().unwrap()).unwrap();
        std::fs::write(
            &hooks,
            r#"{"third-party":{"SessionStart":[{"type":"command","command":"echo keep"}]}}"#,
        )
        .unwrap();
        let binary = temp.path().join("bin/smeagol-harness.exe");
        std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
        std::fs::write(&binary, "fixture").unwrap();

        apply_antigravity_at(temp.path().to_path_buf(), &binary).unwrap();
        let first: Value = serde_json::from_slice(&std::fs::read(&hooks).unwrap()).unwrap();
        apply_antigravity_at(temp.path().to_path_buf(), &binary).unwrap();
        let second: Value = serde_json::from_slice(&std::fs::read(&hooks).unwrap()).unwrap();

        assert_eq!(first, second);
        assert_eq!(
            second["third-party"]["SessionStart"][0]["command"],
            "echo keep"
        );
        assert_eq!(
            second["smeagol-harness-managed"]["SessionStart"][0]["name"],
            "smeagol-harness:antigravity:SessionStart"
        );
        assert_eq!(
            second["smeagol-harness-managed"]["PreToolUse"][0]["matcher"],
            ".*"
        );
        let command = second["smeagol-harness-managed"]["PreInvocation"][0]["command"]
            .as_str()
            .unwrap();
        #[cfg(windows)]
        assert!(!command.starts_with('"'), "{command}");
        assert!(command.contains(" hook --agent antigravity --event PreInvocation"));
        assert!(command.contains("smeagol-harness.exe"));
    }

    #[test]
    fn antigravity_recusa_grupo_de_nome_reservado_que_nao_e_seu() {
        let temp = tempfile::tempdir().unwrap();
        let hooks = temp.path().join("config/hooks.json");
        std::fs::create_dir_all(hooks.parent().unwrap()).unwrap();
        std::fs::write(
            &hooks,
            r#"{"smeagol-harness-managed":{"SessionStart":[{"type":"command","command":"echo user"}]}}"#,
        )
        .unwrap();
        let before = std::fs::read(&hooks).unwrap();
        let binary = temp.path().join("smeagol-harness");

        assert!(apply_antigravity_at(temp.path().to_path_buf(), &binary).is_err());
        assert_eq!(std::fs::read(hooks).unwrap(), before);
    }
}
