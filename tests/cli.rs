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
fn schedule() {
    cases("tests/cli/schedule/**/*.trycmd");
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
fn upcoming() {
    cases("tests/cli/upcoming/**/*.trycmd");
}
