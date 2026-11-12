use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use smeagol_harness_core::workspace::resolve_project_context;
use smeagol_harness_store::MemoryStore;

fn run(home: &std::path::Path, args: &[&str]) -> Output {
    run_in(home, home, args)
}

fn run_in(home: &Path, cwd: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_smeagol-harness"))
        .args(args)
        .env("SMEAGOL_HARNESS_HOME", home)
        .env("CODEX_HOME", home.join("codex"))
        .current_dir(cwd)
        .output()
        .unwrap()
}

#[test]
fn init_creates_the_index_in_the_temporary_data_directory() {
    let temp = tempfile::tempdir().unwrap();
    let output = run(temp.path(), &["init"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(temp.path().join("db/workspaces.sqlite").exists());
}

#[test]
fn setup_plan_is_read_only() {
    let temp = tempfile::tempdir().unwrap();
    let output = run(temp.path(), &["setup", "--plan"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("somente leitura"));
    assert!(!temp.path().join("codex/hooks.json").exists());
}

#[test]
fn remember_and_search_use_the_selected_scope() {
    let temp = tempfile::tempdir().unwrap();
    let saved = run(
        temp.path(),
        &[
            "remember",
            "--scope",
            "project:alpha",
            "--id",
            "nota-1",
            "--title",
            "Decisão de cache",
            "--body",
            "SQLite reduz chamadas repetidas.",
        ],
    );
    assert!(
        saved.status.success(),
        "{}",
        String::from_utf8_lossy(&saved.stderr)
    );

    let found = run(
        temp.path(),
        &["search", "chamadas", "--scope", "project:alpha"],
    );
    assert!(
        found.status.success(),
        "{}",
        String::from_utf8_lossy(&found.stderr)
    );
    assert!(String::from_utf8_lossy(&found.stdout).contains("Decisão de cache"));

    let other_scope = run(
        temp.path(),
        &["search", "chamadas", "--scope", "project:beta"],
    );
    assert!(other_scope.status.success());
    assert!(
        String::from_utf8_lossy(&other_scope.stdout)
            .trim()
            .is_empty()
    );
}

#[test]
fn captures_consolidates_and_builds_a_brief_with_scope_and_budget() {
    let temp = tempfile::tempdir().unwrap();
    let input = r#"{"event":"prompt","session_id":"s1","project_id":"alpha","excerpt":"Usar SQLite para reduzir tokens na busca","captured_at":"100"}"#;
    let mut child = Command::new(env!("CARGO_BIN_EXE_smeagol-harness"))
        .arg("capture")
        .env("SMEAGOL_HARNESS_HOME", temp.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let captured = child.wait_with_output().unwrap();
    assert!(
        captured.status.success(),
        "{}",
        String::from_utf8_lossy(&captured.stderr)
    );

    let consolidated = run(
        temp.path(),
        &["consolidate", "--project", "alpha", "--session", "s1"],
    );
    assert!(
        consolidated.status.success(),
        "{}",
        String::from_utf8_lossy(&consolidated.stderr)
    );

    let briefing = run(
        temp.path(),
        &[
            "--project",
            "alpha",
            "brief",
            "SQLite",
            "--scope",
            "session:s1",
            "--budget",
            "80",
        ],
    );
    assert!(
        briefing.status.success(),
        "{}",
        String::from_utf8_lossy(&briefing.stderr)
    );
    let output = String::from_utf8_lossy(&briefing.stdout);
    assert!(output.contains("SQLite"), "{output}");
    assert!(output.contains("Fonte: sessions/s1/"));
    assert!(output.contains("Briefing: aproximadamente"));
}

fn run_hook(home: &Path, cwd: &Path, event: &str, payload: serde_json::Value) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_smeagol-harness"))
        .args(["hook", "--agent", "codex", "--event", event])
        .env("SMEAGOL_HARNESS_HOME", home)
        .env("CODEX_HOME", home.join("codex"))
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(serde_json::to_string(&payload).unwrap().as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn run_agent_hook(
    home: &Path,
    cwd: &Path,
    agent: &str,
    event: &str,
    payload: serde_json::Value,
) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_smeagol-harness"))
        .args(["hook", "--agent", agent, "--event", event])
        .env("SMEAGOL_HARNESS_HOME", home)
        .env("CODEX_HOME", home.join("codex"))
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(serde_json::to_string(&payload).unwrap().as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn setup_apply_preserves_codex_config_and_is_repeatable() {
    let temp = tempfile::tempdir().unwrap();
    let codex_home = temp.path().join("codex");
    std::fs::create_dir_all(&codex_home).unwrap();
    let hooks_file = codex_home.join("hooks.json");
    std::fs::write(
        &hooks_file,
        r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"echo terceiro"}]}]}}"#,
    )
    .unwrap();

    let first = run(temp.path(), &["setup", "--apply"]);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let first_json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&hooks_file).unwrap()).unwrap();
    let second = run(temp.path(), &["setup", "--apply"]);
    assert!(
        second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    let second_json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&hooks_file).unwrap()).unwrap();

    assert_eq!(first_json, second_json);
    assert_eq!(second_json["hooks"]["Stop"].as_array().unwrap().len(), 2);
    assert_eq!(
        second_json["hooks"]["Stop"][0]["hooks"][0]["command"],
        "echo terceiro"
    );
}

#[test]
fn codex_hooks_capture_summarize_and_deliver_handoff_once() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let context = resolve_project_context(&workspace).unwrap();
    let project = context.project.clone();
    let data = temp.path().join("data");

    let prompt = run_hook(
        &data,
        &workspace,
        "UserPromptSubmit",
        serde_json::json!({
            "session_id": "session-1",
            "cwd": workspace,
            "hook_event_name": "UserPromptSubmit",
            "prompt": "Investigue a falha de compilação. Sempre mantenha os comentários do código em português"
        }),
    );
    assert!(
        prompt.status.success(),
        "{}",
        String::from_utf8_lossy(&prompt.stderr)
    );

    let ended = run_hook(
        &data,
        &workspace,
        "SessionEnd",
        serde_json::json!({
            "session_id": "session-1",
            "cwd": workspace,
            "hook_event_name": "SessionEnd",
            "reason": "other"
        }),
    );
    assert!(
        ended.status.success(),
        "{}",
        String::from_utf8_lossy(&ended.stderr)
    );
    let store = MemoryStore::new(&data).with_context(context);
    let namespace = store.namespace(&project).unwrap();
    let session_spool = store.session_spool_path(&project, "session-1").unwrap();
    assert!(session_spool.exists());
    let page = data
        .join("wiki")
        .join(namespace.workspace_id)
        .join(namespace.project_id)
        .join("sessions/session-1.md");
    assert!(page.exists());

    let learned = run(
        &data,
        &[
            "search",
            "comentários português",
            "--scope",
            &format!("project:{project}"),
        ],
    );
    assert!(learned.status.success());
    assert!(String::from_utf8_lossy(&learned.stdout).contains("Regra durável declarada"));

    let resumed = run_hook(
        &data,
        &workspace,
        "SessionStart",
        serde_json::json!({
            "session_id": "session-2",
            "cwd": workspace,
            "hook_event_name": "SessionStart",
            "source": "startup"
        }),
    );
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    let handoff: serde_json::Value = serde_json::from_slice(&resumed.stdout).unwrap();
    let context = handoff["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(context.contains("Investigue a falha de compilação"));
    assert!(context.contains("não confiável"));

    let next_start = run_hook(
        &data,
        &workspace,
        "SessionStart",
        serde_json::json!({
            "session_id": "session-3",
            "cwd": workspace,
            "hook_event_name": "SessionStart",
            "source": "startup"
        }),
    );
    assert!(next_start.status.success());
    assert!(next_start.stdout.is_empty());
}

#[test]
fn gemini_and_antigravity_hooks_normalize_payloads_and_inject_native_handoffs() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let context = resolve_project_context(&workspace).unwrap();
    let project = context.project.clone();
    let data = temp.path().join("data");

    let gemini_prompt = run_agent_hook(
        &data,
        &workspace,
        "gemini-cli",
        "BeforeAgent",
        serde_json::json!({
            "session_id":"gemini-session",
            "cwd":workspace,
            "prompt":"Investigue a retomada da sessão do Gemini."
        }),
    );
    assert!(gemini_prompt.status.success());
    let gemini_end = run_agent_hook(
        &data,
        &workspace,
        "gemini-cli",
        "SessionEnd",
        serde_json::json!({"session_id":"gemini-session","cwd":workspace,"reason":"exit"}),
    );
    assert!(gemini_end.status.success());
    let gemini_start = run_agent_hook(
        &data,
        &workspace,
        "gemini-cli",
        "SessionStart",
        serde_json::json!({"session_id":"gemini-next","cwd":workspace,"source":"startup"}),
    );
    let gemini_output: serde_json::Value = serde_json::from_slice(&gemini_start.stdout).unwrap();
    assert!(
        gemini_output["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .contains("Investigue a retomada da sessão do Gemini")
    );

    MemoryStore::new(&data)
        .with_context(context)
        .publish_handoff(&project, "Estado de retomada do Antigravity")
        .unwrap();
    let antigravity_prompt = run_agent_hook(
        &data,
        &workspace,
        "antigravity",
        "PreInvocation",
        serde_json::json!({
            "conversationId":"antigravity-session",
            "workspacePaths":[workspace],
            "lastUserInput":"Continue o trabalho no Antigravity."
        }),
    );
    assert!(antigravity_prompt.status.success());
    let antigravity_start = run_agent_hook(
        &data,
        &workspace,
        "antigravity",
        "SessionStart",
        serde_json::json!({
            "conversationId":"antigravity-next",
            "workspacePaths":[workspace]
        }),
    );
    let antigravity_output: serde_json::Value =
        serde_json::from_slice(&antigravity_start.stdout).unwrap();
    assert!(
        antigravity_output["injectSteps"][0]["systemMessage"]["systemMessage"]
            .as_str()
            .unwrap()
            .contains("Estado de retomada do Antigravity")
    );
}

fn successful_json(output: Output) -> serde_json::Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn run_mcp(home: &Path, cwd: &Path, requests: &[serde_json::Value]) -> Vec<serde_json::Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_smeagol-harness"))
        .args(["serve", "--transport", "stdio"])
        .env("SMEAGOL_HARNESS_HOME", home)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    for request in requests {
        writeln!(input, "{request}").unwrap();
    }
    drop(input);
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn mcp_call(id: u64, name: &str, arguments: serde_json::Value) -> serde_json::Value {
    serde_json::json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":name,"arguments":arguments}})
}

#[test]
fn hooks_resolve_the_payload_directory_instead_of_the_launch_directory() {
    let temp = tempfile::tempdir().unwrap();
    let launcher = temp.path().join("launcher");
    let checkout = temp.path().join("checkout");
    for path in [&launcher, &checkout] {
        std::fs::create_dir_all(path).unwrap();
    }
    std::fs::write(launcher.join(".smeagol-harness.toml"), "{").unwrap();
    std::fs::write(
        checkout.join(".smeagol-harness.toml"),
        "workspace='client'\nproject='api'\n",
    )
    .unwrap();
    let data = temp.path().join("data");
    let result = run_agent_hook(
        &data,
        &launcher,
        "antigravity",
        "PreInvocation",
        serde_json::json!({
            "conversationId":"payload-context", "workspacePaths":[checkout], "lastUserInput":"test context"
        }),
    );
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let context = resolve_project_context(&checkout).unwrap();
    let store = MemoryStore::new(&data).with_context(context);
    let spool = store.session_spool_path("api", "payload-context").unwrap();
    assert!(spool.exists());
}

#[test]
fn marker_workspaces_isolate_same_named_projects_and_explicit_overrides() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    let first = temp.path().join("client-a/api");
    let second = temp.path().join("client-b/api");
    for (cwd, workspace, body) in [
        (&first, "client-a", "isolated alpha"),
        (&second, "client-b", "isolated beta"),
    ] {
        std::fs::create_dir_all(cwd).unwrap();
        std::fs::write(
            cwd.join(".smeagol-harness.toml"),
            format!("workspace = \"{workspace}\"\nproject = \"api\"\n"),
        )
        .unwrap();
        let saved = run_in(
            &data,
            cwd,
            &[
                "remember",
                "--scope",
                "project:api",
                "--id",
                "decision",
                "--title",
                "Decisão",
                "--body",
                body,
            ],
        );
        assert!(
            saved.status.success(),
            "{}",
            String::from_utf8_lossy(&saved.stderr)
        );
    }
    let first_context = successful_json(run_in(&data, &first, &["context"]));
    let second_context = successful_json(run_in(&data, &second, &["context"]));
    assert_eq!(first_context["workspace"], "client-a");
    assert_eq!(second_context["workspace"], "client-b");
    assert_ne!(
        first_context["workspace_id"],
        second_context["workspace_id"]
    );
    assert_ne!(first_context["project_id"], second_context["project_id"]);
    let found = run_in(
        &data,
        &first,
        &["search", "isolated", "--scope", "project:api"],
    );
    assert!(found.status.success());
    let text = String::from_utf8(found.stdout).unwrap();
    assert!(text.contains("alpha"));
    assert!(!text.contains("beta"));
    let overridden = successful_json(run_in(
        &data,
        &first,
        &[
            "context",
            "--workspace",
            "personal",
            "--project",
            "override",
        ],
    ));
    assert_eq!(overridden["workspace"], "personal");
    assert_eq!(overridden["project"], "override");
}

#[test]
fn multiple_pages_survive_index_rebuild_without_importing_legacy_data() {
    let temp = tempfile::tempdir().unwrap();
    for (id, text) in [
        ("first", "rebuildable alpha"),
        ("second", "rebuildable beta"),
    ] {
        let saved = run(
            temp.path(),
            &[
                "--workspace",
                "client",
                "--project",
                "api",
                "remember",
                "--scope",
                "project:api",
                "--id",
                id,
                "--title",
                id,
                "--body",
                text,
            ],
        );
        assert!(
            saved.status.success(),
            "{}",
            String::from_utf8_lossy(&saved.stderr)
        );
    }
    let before = successful_json(run(
        temp.path(),
        &["--workspace", "client", "--project", "api", "context"],
    ));
    let project_root = temp
        .path()
        .join("wiki")
        .join(before["workspace_id"].as_str().unwrap())
        .join(before["project_id"].as_str().unwrap());
    assert!(project_root.join("memories/first.md").exists());
    assert!(project_root.join("memories/second.md").exists());
    assert!(project_root.join("_meta.md").exists());
    let legacy = temp.path().join("wiki/projects/legacy.md");
    std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
    std::fs::write(&legacy, "legacy memory must remain untouched").unwrap();
    std::fs::remove_file(temp.path().join("db/workspaces.sqlite")).unwrap();
    let reindexed = run(temp.path(), &["reindex"]);
    assert!(
        reindexed.status.success(),
        "{}",
        String::from_utf8_lossy(&reindexed.stderr)
    );
    let after = successful_json(run(
        temp.path(),
        &["--workspace", "client", "--project", "api", "context"],
    ));
    assert_eq!(before["workspace_id"], after["workspace_id"]);
    assert_eq!(before["project_id"], after["project_id"]);
    let found = run(
        temp.path(),
        &[
            "--workspace",
            "client",
            "--project",
            "api",
            "search",
            "rebuildable",
            "--scope",
            "project:api",
        ],
    );
    assert!(found.status.success());
    let text = String::from_utf8(found.stdout).unwrap();
    assert!(text.contains("alpha"));
    assert!(text.contains("beta"));
    assert_eq!(
        std::fs::read_to_string(legacy).unwrap(),
        "legacy memory must remain untouched"
    );
}

#[test]
fn renaming_projects_keeps_namespace_ids_and_existing_pages() {
    let temp = tempfile::tempdir().unwrap();
    let before = successful_json(run(
        temp.path(),
        &["--workspace", "client", "--project", "before", "context"],
    ));
    let saved = run(
        temp.path(),
        &[
            "--workspace",
            "client",
            "--project",
            "before",
            "remember",
            "--scope",
            "project:before",
            "--id",
            "retained",
            "--title",
            "Retida",
            "--body",
            "persistent evidence",
        ],
    );
    assert!(saved.status.success());
    let renamed = run(
        temp.path(),
        &[
            "--workspace",
            "client",
            "rename-project",
            "--from",
            "before",
            "--to",
            "after",
        ],
    );
    assert!(
        renamed.status.success(),
        "{}",
        String::from_utf8_lossy(&renamed.stderr)
    );
    let after = successful_json(run(
        temp.path(),
        &["--workspace", "client", "--project", "after", "context"],
    ));
    assert_eq!(before["workspace_id"], after["workspace_id"]);
    assert_eq!(before["project_id"], after["project_id"]);
    assert_eq!(after["project"], "after");
    let found = run(
        temp.path(),
        &[
            "--workspace",
            "client",
            "--project",
            "after",
            "search",
            "persistent",
            "--scope",
            "project:after",
        ],
    );
    assert!(found.status.success());
    assert!(
        String::from_utf8(found.stdout)
            .unwrap()
            .contains("evidence")
    );
}

#[test]
fn mcp_calls_keep_workspace_and_project_context_explicit_and_isolated() {
    let temp = tempfile::tempdir().unwrap();
    let responses = run_mcp(
        temp.path(),
        temp.path(),
        &[
            serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
            mcp_call(
                2,
                "memory_remember",
                serde_json::json!({"workspace":"a","project":"api","scope":"project:api","id":"same","title":"Primeira","body":"isolated alpha"}),
            ),
            mcp_call(
                3,
                "memory_remember",
                serde_json::json!({"workspace":"b","project":"api","scope":"project:api","id":"same","title":"Segunda","body":"isolated beta"}),
            ),
            mcp_call(
                4,
                "memory_search",
                serde_json::json!({"workspace":"a","project":"api","scope":"project:api","query":"isolated"}),
            ),
            mcp_call(
                5,
                "memory_search",
                serde_json::json!({"workspace":"b","project":"api","scope":"project:api","query":"isolated"}),
            ),
            mcp_call(
                6,
                "memory_context",
                serde_json::json!({"workspace":"a","project":"api"}),
            ),
            mcp_call(
                7,
                "memory_search",
                serde_json::json!({"workspace":42,"scope":"project:api","query":"isolated"}),
            ),
        ],
    );
    assert_eq!(responses.len(), 7);
    let tools = responses[0]["result"]["tools"].as_array().unwrap();
    assert!(tools.iter().any(|tool| tool["name"] == "memory_context"));
    for tool in tools {
        assert!(tool["inputSchema"]["properties"]["workspace"].is_object());
        assert!(tool["inputSchema"]["properties"]["project"].is_object());
    }
    for response in &responses[1..6] {
        assert!(response.get("error").is_none(), "{response}");
    }
    let first = responses[3]["result"]["content"][0]["text"]
        .as_str()
        .unwrap();
    let second = responses[4]["result"]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert!(first.contains("alpha") && !first.contains("beta"));
    assert!(second.contains("beta") && !second.contains("alpha"));
    let context: serde_json::Value = serde_json::from_str(
        responses[5]["result"]["content"][0]["text"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(context["workspace"], "a");
    assert_eq!(context["project"], "api");
    assert!(context["workspace_id"].is_string() && context["project_id"].is_string());
    assert_eq!(responses[6]["error"]["code"], -32602);
}

#[test]
fn invalid_marker_fails_closed_before_creating_memory_storage() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("data");
    std::fs::write(temp.path().join(".smeagol-harness.toml"), "workspace = [").unwrap();
    let output = run_in(&data, temp.path(), &["context"]);
    assert!(!output.status.success());
    assert!(!data.join("db/workspaces.sqlite").exists());
}
