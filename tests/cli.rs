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

/// the proxy refuses the connection
///
/// the sync fails without a request leaving the machine
/// with no journal on disk there is no cached state to show
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
        .env_remove("THINGS_LOG")
        .env_remove("THINGS_LOG_FORMAT")
        .output()
        .expect("things runs");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(
        stderr.contains("Sync failed, and no cached state is known to belong to this account"),
        "{stderr}"
    );
    assert!(output.stdout.is_empty());
}

/// a cursor that names the configured email with no journal beside it leaves no cached state to show
#[test]
fn failed_sync_with_a_cursor_and_no_journal() {
    let home = tempfile::tempdir().expect("tempdir");
    let config = home.path().join("config");
    let cache = home.path().join("state").join("things").join("append-log");
    std::fs::create_dir_all(config.join("things")).expect("config dir");
    std::fs::create_dir_all(&cache).expect("cache dir");
    std::fs::write(
        config.join("things").join("auth.json"),
        r#"{"email":"user@example.com","password":"hunter2"}"#,
    )
    .expect("auth file");
    std::fs::write(
        cache.join("cursor.json"),
        r#"{"next_start_index":0,"history_key":"k","log_offset":0,"email":"user@example.com"}"#,
    )
    .expect("cursor");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_things"))
        .args(["--json", "inbox"])
        .env("XDG_CONFIG_HOME", &config)
        .env("XDG_STATE_HOME", home.path().join("state"))
        .env("HTTPS_PROXY", "http://127.0.0.1:1")
        .env_remove("NO_PROXY")
        .env_remove("no_proxy")
        .env_remove("REQUEST_METHOD")
        .env_remove("THINGS_EMAIL")
        .env_remove("THINGS_PASSWORD")
        .env_remove("THINGS_LOG")
        .env_remove("THINGS_LOG_FORMAT")
        .output()
        .expect("things runs");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(
        stderr.contains("Sync failed, and no cached state is known to belong to this account"),
        "{stderr}"
    );
    assert!(output.stdout.is_empty());
}

/// a journal shorter than the cursor claims is emptied by the repair before the request
///
/// the empty journal that remains is no cached state to show
#[test]
fn failed_sync_with_a_journal_the_repair_empties() {
    let home = tempfile::tempdir().expect("tempdir");
    let config = home.path().join("config");
    let cache = home.path().join("state").join("things").join("append-log");
    std::fs::create_dir_all(config.join("things")).expect("config dir");
    std::fs::create_dir_all(&cache).expect("cache dir");
    std::fs::write(
        config.join("things").join("auth.json"),
        r#"{"email":"user@example.com","password":"hunter2"}"#,
    )
    .expect("auth file");
    std::fs::write(
        cache.join("things.log"),
        "{\"Ta11111111111111111111\":{\"t\":0,\"e\":\"Task7\",\"p\":{\"tt\":\"Kept\"}}}\n",
    )
    .expect("journal");
    std::fs::write(
        cache.join("cursor.json"),
        r#"{"next_start_index":2,"history_key":"k","log_offset":180,"email":"user@example.com"}"#,
    )
    .expect("cursor");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_things"))
        .args(["--json", "inbox"])
        .env("XDG_CONFIG_HOME", &config)
        .env("XDG_STATE_HOME", home.path().join("state"))
        .env("HTTPS_PROXY", "http://127.0.0.1:1")
        .env_remove("NO_PROXY")
        .env_remove("no_proxy")
        .env_remove("REQUEST_METHOD")
        .env_remove("THINGS_EMAIL")
        .env_remove("THINGS_PASSWORD")
        .env_remove("THINGS_LOG")
        .env_remove("THINGS_LOG_FORMAT")
        .output()
        .expect("things runs");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(
        stderr.contains("Sync failed, and no cached state is known to belong to this account"),
        "{stderr}"
    );
    assert!(output.stdout.is_empty());
}

/// a cache that another email's sign-in wrote is no fallback when the sync fails
#[test]
fn failed_sync_with_another_accounts_cache() {
    let home = tempfile::tempdir().expect("tempdir");
    let config = home.path().join("config");
    let journal = home.path().join("state").join("things").join("append-log");
    std::fs::create_dir_all(config.join("things")).expect("config dir");
    std::fs::create_dir_all(&journal).expect("journal dir");
    std::fs::write(
        config.join("things").join("auth.json"),
        r#"{"email":"bob@example.com","password":"hunter2"}"#,
    )
    .expect("auth file");
    std::fs::write(
        journal.join("things.log"),
        "{\"Ta11111111111111111111\":{\"t\":0,\"e\":\"Task7\",\"p\":{\"tt\":\"Alice private\",\"st\":0,\"ss\":0}}}\n",
    )
    .expect("journal");
    std::fs::write(
        journal.join("cursor.json"),
        r#"{"next_start_index":1,"history_key":"k","email":"alice@example.com"}"#,
    )
    .expect("cursor");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_things"))
        .args(["--json", "inbox"])
        .env("XDG_CONFIG_HOME", &config)
        .env("XDG_STATE_HOME", home.path().join("state"))
        .env("HTTPS_PROXY", "http://127.0.0.1:1")
        .env_remove("NO_PROXY")
        .env_remove("no_proxy")
        .env_remove("REQUEST_METHOD")
        .env_remove("THINGS_EMAIL")
        .env_remove("THINGS_PASSWORD")
        .env_remove("THINGS_LOG")
        .env_remove("THINGS_LOG_FORMAT")
        .output()
        .expect("things runs");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(
        stderr.contains("known to belong to this account"),
        "{stderr}"
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains("Alice"));
}

/// a cache that cannot be locked fails the run once, not as a failed sync
#[test]
fn unusable_cache_directory() {
    let home = tempfile::tempdir().expect("tempdir");
    let config = home.path().join("config");
    std::fs::create_dir_all(config.join("things")).expect("config dir");
    std::fs::write(
        config.join("things").join("auth.json"),
        r#"{"email":"user@example.com","password":"hunter2"}"#,
    )
    .expect("auth file");
    let state = home.path().join("state");
    std::fs::write(&state, "a file where the state directory goes").expect("state file");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_things"))
        .args(["inbox"])
        .env("XDG_CONFIG_HOME", &config)
        .env("XDG_STATE_HOME", &state)
        .env("HTTPS_PROXY", "http://127.0.0.1:1")
        .env_remove("NO_PROXY")
        .env_remove("no_proxy")
        .env_remove("REQUEST_METHOD")
        .env_remove("THINGS_EMAIL")
        .env_remove("THINGS_PASSWORD")
        .env_remove("THINGS_LOG")
        .env_remove("THINGS_LOG_FORMAT")
        .output()
        .expect("things runs");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(!stderr.contains("Sync failed"), "{stderr}");
    assert_eq!(stderr.lines().count(), 1, "{stderr}");
}

/// only a cursor that names the configured email makes its cache a fallback
#[test]
fn failed_sync_with_a_cache_nobody_vouches_for() {
    let home = tempfile::tempdir().expect("tempdir");
    let config = home.path().join("config");
    let journal = home.path().join("state").join("things").join("append-log");
    std::fs::create_dir_all(config.join("things")).expect("config dir");
    std::fs::create_dir_all(&journal).expect("journal dir");
    std::fs::write(
        config.join("things").join("auth.json"),
        r#"{"email":"bob@example.com","password":"hunter2"}"#,
    )
    .expect("auth file");
    std::fs::write(
        journal.join("things.log"),
        "{\"Ta11111111111111111111\":{\"t\":0,\"e\":\"Task7\",\"p\":{\"tt\":\"Alice private\",\"st\":0,\"ss\":0}}}\n",
    )
    .expect("journal");
    std::fs::write(
        journal.join("cursor.json"),
        r#"{"next_start_index":1,"history_key":"k"}"#,
    )
    .expect("cursor");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_things"))
        .args(["--json", "inbox"])
        .env("XDG_CONFIG_HOME", &config)
        .env("XDG_STATE_HOME", home.path().join("state"))
        .env("HTTPS_PROXY", "http://127.0.0.1:1")
        .env_remove("NO_PROXY")
        .env_remove("no_proxy")
        .env_remove("REQUEST_METHOD")
        .env_remove("THINGS_EMAIL")
        .env_remove("THINGS_PASSWORD")
        .env_remove("THINGS_LOG")
        .env_remove("THINGS_LOG_FORMAT")
        .output()
        .expect("things runs");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(!String::from_utf8_lossy(&output.stdout).contains("Alice"));
}

/// a cache that this email's sign-in wrote answers the command when the sync fails
#[test]
fn failed_sync_with_this_accounts_cache() {
    let home = tempfile::tempdir().expect("tempdir");
    let config = home.path().join("config");
    let journal = home.path().join("state").join("things").join("append-log");
    std::fs::create_dir_all(config.join("things")).expect("config dir");
    std::fs::create_dir_all(&journal).expect("journal dir");
    std::fs::write(
        config.join("things").join("auth.json"),
        r#"{"email":"bob@example.com","password":"hunter2"}"#,
    )
    .expect("auth file");
    let line = "{\"Ta11111111111111111111\":{\"t\":0,\"e\":\"Task7\",\"p\":{\"tt\":\"Bob cached\",\"st\":0,\"ss\":0}}}\n";
    std::fs::write(journal.join("things.log"), line).expect("journal");
    std::fs::write(
        journal.join("cursor.json"),
        format!(
            r#"{{"next_start_index":1,"history_key":"k","log_offset":{},"email":"bob@example.com"}}"#,
            line.len()
        ),
    )
    .expect("cursor");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_things"))
        .args(["--json", "find", "--any-status"])
        .env("XDG_CONFIG_HOME", &config)
        .env("XDG_STATE_HOME", home.path().join("state"))
        .env("HTTPS_PROXY", "http://127.0.0.1:1")
        .env_remove("NO_PROXY")
        .env_remove("no_proxy")
        .env_remove("REQUEST_METHOD")
        .env_remove("THINGS_EMAIL")
        .env_remove("THINGS_PASSWORD")
        .env_remove("THINGS_LOG")
        .env_remove("THINGS_LOG_FORMAT")
        .output()
        .expect("things runs");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(3), "{stderr}");
    // the stored key went to the items without a sign-in
    assert!(stderr.contains("the history items from 1"), "{stderr}");
    assert!(String::from_utf8_lossy(&output.stdout).contains("Bob cached"));
}

/// a cache that cannot be read after a failed sync names both failures on one line
#[test]
fn failed_sync_with_a_cache_that_does_not_read() {
    let home = tempfile::tempdir().expect("tempdir");
    let config = home.path().join("config");
    let journal = home.path().join("state").join("things").join("append-log");
    std::fs::create_dir_all(config.join("things")).expect("config dir");
    std::fs::create_dir_all(&journal).expect("journal dir");
    std::fs::write(
        config.join("things").join("auth.json"),
        r#"{"email":"bob@example.com","password":"hunter2"}"#,
    )
    .expect("auth file");
    std::fs::write(journal.join("things.log"), "{not json}\n").expect("journal");
    std::fs::write(
        journal.join("cursor.json"),
        r#"{"next_start_index":1,"history_key":"k","log_offset":11,"email":"bob@example.com"}"#,
    )
    .expect("cursor");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_things"))
        .args(["inbox"])
        .env("XDG_CONFIG_HOME", &config)
        .env("XDG_STATE_HOME", home.path().join("state"))
        .env("HTTPS_PROXY", "http://127.0.0.1:1")
        .env_remove("NO_PROXY")
        .env_remove("no_proxy")
        .env_remove("REQUEST_METHOD")
        .env_remove("THINGS_EMAIL")
        .env_remove("THINGS_PASSWORD")
        .env_remove("THINGS_LOG")
        .env_remove("THINGS_LOG_FORMAT")
        .output()
        .expect("things runs");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("Sync failed ("), "{stderr}");
    assert!(stderr.contains("the sync cache cannot be read"), "{stderr}");
    assert!(stderr.contains("Corrupt log entry"), "{stderr}");
    assert_eq!(stderr.lines().count(), 1, "{stderr}");
}

/// a reader that closed stderr leaves the exit status as it is
#[test]
fn closed_stderr() {
    let home = tempfile::tempdir().expect("tempdir");
    let (reader, writer) = std::io::pipe().expect("pipe");
    drop(reader);
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_things"))
        .args(["--no-cloud", "--load-journal", "missing.json", "today"])
        .current_dir(home.path())
        .env("XDG_CONFIG_HOME", home.path().join("config"))
        .env("XDG_STATE_HOME", home.path().join("state"))
        .env_remove("THINGS_LOG")
        .env_remove("THINGS_LOG_FORMAT")
        .stdout(std::process::Stdio::null())
        .stderr(writer)
        .status()
        .expect("things runs");
    assert_eq!(status.code(), Some(1));
}

/// a log directive that does not parse is reported without touching the exit status
#[test]
fn bad_log_directive_with_closed_stderr() {
    let home = tempfile::tempdir().expect("tempdir");
    let journal = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("cli")
        .join("today")
        .join("basic_list.in")
        .join("journal.json");
    let (reader, writer) = std::io::pipe().expect("pipe");
    drop(reader);
    let status = std::process::Command::new(env!("CARGO_BIN_EXE_things"))
        .args(["--no-cloud", "--load-journal"])
        .arg(&journal)
        .args(["--today-ts", "1774396800", "today"])
        .env("XDG_CONFIG_HOME", home.path().join("config"))
        .env("XDG_STATE_HOME", home.path().join("state"))
        .env("THINGS_LOG", "things=nope")
        .env_remove("THINGS_LOG_FORMAT")
        .stdout(std::process::Stdio::null())
        .stderr(writer)
        .status()
        .expect("things runs");
    assert_eq!(status.code(), Some(0));
}

/// a log setting that cannot apply is reported
///
/// the command runs on with the default in its place
#[cfg(unix)]
#[test]
fn unread_log_settings_are_reported() {
    use std::os::unix::ffi::OsStringExt;
    let home = tempfile::tempdir().expect("tempdir");
    let journal = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("cli")
        .join("today")
        .join("basic_list.in")
        .join("journal.json");
    let run = |name: &str, value: std::ffi::OsString| {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_things"))
            .args(["--no-cloud", "--load-journal"])
            .arg(&journal)
            .args(["--today-ts", "1774396800", "today"])
            .env("XDG_CONFIG_HOME", home.path().join("config"))
            .env("XDG_STATE_HOME", home.path().join("state"))
            .env_remove("THINGS_LOG")
            .env_remove("THINGS_LOG_FORMAT")
            .env(name, value)
            .output()
            .expect("things runs");
        assert_eq!(output.status.code(), Some(0));
        String::from_utf8_lossy(&output.stderr).into_owned()
    };
    assert_eq!(
        run("THINGS_LOG_FORMAT", "JSON".into()),
        "THINGS_LOG_FORMAT is ignored: \"JSON\" is not pretty, simplified or json\n"
    );
    assert_eq!(
        run("THINGS_LOG", std::ffi::OsString::from_vec(vec![0xff])),
        "THINGS_LOG is ignored: \"\\xFF\" is not valid UTF-8\n"
    );
    let unparsed = run("THINGS_LOG", "things=nope".into());
    assert!(
        unparsed.starts_with("THINGS_LOG is ignored: "),
        "{unparsed}"
    );
    assert_eq!(unparsed.lines().count(), 1, "{unparsed}");
}
