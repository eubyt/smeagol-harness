use smeagol_harness_setup::apply_devin_at;

#[test]
fn installs_global_devin_hooks_idempotently_and_preserves_other_settings() {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("devin/config.json");
    std::fs::create_dir_all(config.parent().unwrap()).unwrap();
    std::fs::write(
        &config,
        r#"{"model":"keep-me","hooks":{"UserPromptSubmit":[{"hooks":[{"name":"third-party","type":"command","command":"echo keep"}]}]}}"#,
    )
    .unwrap();
    let binary = std::path::Path::new("/opt/tools/smeagol harness");

    apply_devin_at(config.clone(), binary).unwrap();
    let first: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&config).unwrap()).unwrap();
    apply_devin_at(config.clone(), binary).unwrap();
    let second: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&config).unwrap()).unwrap();

    assert_eq!(first, second);
    assert_eq!(second["model"], "keep-me");
    assert_eq!(
        second["hooks"]["UserPromptSubmit"][0]["hooks"][0]["name"],
        "third-party"
    );
    assert_eq!(
        second["hooks"]["SessionStart"][0]["hooks"][0]["name"],
        "smeagol-harness:devin:SessionStart"
    );
}
