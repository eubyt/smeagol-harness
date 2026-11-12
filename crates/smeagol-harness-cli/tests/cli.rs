use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use smeagol_harness_consolidate::session_scope_id;
use smeagol_harness_core::project_id_for_cwd;
use smeagol_harness_store::MemoryStore;

fn run(home: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_smeagol-harness"))
        .args(args)
        .env("SMEAGOL_HARNESS_HOME", home)
        .env("CODEX_HOME", home.join("codex"))
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
    assert!(temp.path().join("db/memory.sqlite").exists());
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
            "brief",
            "SQLite",
            "--scope",
            "session:p-alpha-s-s1",
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
    assert!(output.contains("Fonte: sessions/p-alpha-s-s1/"));
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
    let project = project_id_for_cwd(&workspace);
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
    let session_spool = data
        .join("spool/sessions")
        .join(project.clone())
        .join("session-1.jsonl");
    assert!(session_spool.exists());
    let page = data
        .join("wiki/sessions")
        .join(format!("{}.md", session_scope_id(&project, "session-1")));
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
    let project = project_id_for_cwd(&workspace);
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
