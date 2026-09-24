fn cases(case_glob: &str) {
    let things_bin = std::path::PathBuf::from(env!("CARGO_BIN_EXE_things"));

    trycmd::TestCases::new()
        .env("TRYCMD_BIN_THINGS", things_bin.display().to_string())
        .register_bin("things", &things_bin)
        .register_bin(
            "run.sh",
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests")
                .join("cli")
                .join("run.sh"),
        )
        .case(case_glob);
}

#[test]
fn anytime() {
    cases("tests/cli/anytime/**/*.trycmd");
}

#[test]
fn area() {
    cases("tests/cli/area/**/*.trycmd");
}

#[test]
fn areas() {
    cases("tests/cli/areas/**/*.trycmd");
}

#[test]
fn delete() {
    cases("tests/cli/delete/**/*.trycmd");
}

#[test]
fn edit() {
    cases("tests/cli/edit/**/*.trycmd");
}

#[test]
fn find() {
    cases("tests/cli/find/**/*.trycmd");
}

#[test]
fn inbox() {
    cases("tests/cli/inbox/**/*.trycmd");
}

#[test]
fn logbook() {
    cases("tests/cli/logbook/**/*.trycmd");
}

#[test]
fn mark() {
    cases("tests/cli/mark/**/*.trycmd");
}

#[test]
fn new() {
    cases("tests/cli/new/**/*.trycmd");
}

#[test]
fn project() {
    cases("tests/cli/project/**/*.trycmd");
}

#[test]
fn projects() {
    cases("tests/cli/projects/**/*.trycmd");
}

#[test]
fn reorder() {
    cases("tests/cli/reorder/**/*.trycmd");
}

#[test]
fn someday() {
    cases("tests/cli/someday/**/*.trycmd");
}

#[test]
fn tags() {
    cases("tests/cli/tags/**/*.trycmd");
}

#[test]
fn today() {
    cases("tests/cli/today/**/*.trycmd");
}

#[test]
fn trash() {
    cases("tests/cli/trash/**/*.trycmd");
}

#[test]
fn upcoming() {
    cases("tests/cli/upcoming/**/*.trycmd");
}

#[test]
fn show() {
    cases("tests/cli/show/**/*.trycmd");
}

/// the proxy refuses the connection, the sync fails without a request leaving the machine
#[test]
fn failed_sync() {
    let home = tempfile::tempdir().expect("tempdir");
    let config = home.path().join("config");
    std::fs::create_dir_all(config.join("things")).expect("config dir");
    std::fs::write(
        config.join("things").join("auth.json"),
        r#"{"email":"user@example.com","password":"hunter2"}"#,
    )
    .expect("auth file");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_things"))
        .args(["--json", "inbox"])
        .env("XDG_CONFIG_HOME", &config)
        .env("XDG_STATE_HOME", home.path().join("state"))
        .env("HTTPS_PROXY", "http://127.0.0.1:1")
        .env_remove("NO_PROXY")
        .env_remove("no_proxy")
        .env_remove("THINGS_EMAIL")
        .env_remove("THINGS_PASSWORD")
        .output()
        .expect("things runs");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(3), "{stderr}");
    assert!(
        stderr.contains("Sync failed, showing the cached state"),
        "{stderr}"
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "[]");
}
