use std::{
    ffi::OsString,
    fs,
    io::{ErrorKind, Write},
    path::Path,
};

use anyhow::{Context, Result, anyhow};
use serde::Serialize;
use serde_json::Value;

use crate::dirs::{auth_file_path, create_private_dir};

#[derive(Serialize)]
struct AuthPayload {
    email: String,
    password: String,
}

struct AuthConfig {
    email: Option<String>,
    password: Option<String>,
}

/// a field of the auth file as text
///
/// refused in any other shape without repeating the value
fn text_field(value: Option<Value>, field: &str, path: &Path) -> Result<Option<String>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text)),
        Some(_) => Err(anyhow!(
            "The {field} in the auth file at {} is not a JSON string, quote it",
            path.display()
        )),
    }
}

/// the auth file under `THINGS_EMAIL` and `THINGS_PASSWORD`
///
/// each variable stands in for the file's field when `var` yields it, as the text it holds
/// an empty variable counts as unset
/// every other variable the program reads counts that way too
/// a variable that is not UTF-8 is refused rather than taken as unset
fn load_auth_config(path: &Path, var: impl Fn(&str) -> Option<OsString>) -> Result<AuthConfig> {
    // each field is read as the JSON value it holds
    // a password made of digits stays a password
    let mut fields = match fs::read_to_string(path) {
        Ok(raw) => match serde_json::from_str::<Value>(&raw)
            .with_context(|| format!("Failed reading auth file at {}", path.display()))?
        {
            Value::Object(fields) => fields,
            // any other shape is refused without repeating it
            // a bare string there could be the password
            _ => {
                return Err(anyhow!(
                    "The auth file at {} is not a JSON object",
                    path.display()
                ));
            }
        },
        Err(error) if error.kind() == ErrorKind::NotFound => serde_json::Map::new(),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("Failed reading auth file at {}", path.display()));
        }
    };
    let text_var = |name: &str| {
        var(name)
            .filter(|value| !value.is_empty())
            .map(|value| {
                value
                    .into_string()
                    .map_err(|_| anyhow!("{name} is not valid UTF-8."))
            })
            .transpose()
    };
    let email = match text_var("THINGS_EMAIL")? {
        Some(text) => Some(text),
        None => text_field(fields.remove("email"), "email", path)?,
    };
    let password = match text_var("THINGS_PASSWORD")? {
        Some(text) => Some(text),
        None => text_field(fields.remove("password"), "password", path)?,
    };
    Ok(AuthConfig { email, password })
}

fn validate_auth(email: &str, password: &str) -> Result<(String, String)> {
    let email = email.trim().to_string();
    let password = password.to_string();

    if email.is_empty() {
        return Err(anyhow!("The Things Cloud email is empty."));
    }
    if password.is_empty() {
        return Err(anyhow!("The Things Cloud password is empty."));
    }

    Ok((email, password))
}

pub fn load_auth() -> Result<(String, String)> {
    let path = auth_file_path()?;

    let cfg = load_auth_config(&path, |name| std::env::var_os(name))?;

    let Some(email) = cfg.email else {
        return Err(anyhow!(
            "Missing Things Cloud email. Set THINGS_EMAIL or run `things auth` to save it in the auth file at {}.",
            path.display()
        ));
    };

    let Some(password) = cfg.password else {
        return Err(anyhow!(
            "Missing Things Cloud password. Set THINGS_PASSWORD or run `things auth` to save it in the auth file at {}.",
            path.display()
        ));
    };

    validate_auth(&email, &password)
}

/// the credentials go to the auth file once `sign_in` accepts them
///
/// a refused pair leaves the file as it was
pub fn write_verified_auth(
    email: &str,
    password: &str,
    sign_in: impl FnOnce(&str, &str) -> Result<()>,
) -> Result<std::path::PathBuf> {
    let path = auth_file_path()?;
    write_verified_auth_at(&path, email, password, sign_in)?;
    Ok(path)
}

fn write_verified_auth_at(
    path: &Path,
    email: &str,
    password: &str,
    sign_in: impl FnOnce(&str, &str) -> Result<()>,
) -> Result<()> {
    let (email, password) = validate_auth(email, password)?;
    sign_in(&email, &password).context("Signing in to Things Cloud failed, nothing was saved")?;
    write_auth_at(path, &email, &password)
}

fn write_auth_at(path: &Path, email: &str, password: &str) -> Result<()> {
    let (email, password) = validate_auth(email, password)?;
    let saving = || format!("Failed to save the auth file at {}", path.display());
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("Invalid auth file path"))?;
    create_private_dir(parent).with_context(saving)?;

    let payload = AuthPayload { email, password };
    let serialized = serde_json::to_string(&payload)?;
    let tmp_path = path.with_extension("tmp");

    // the staging file is private from the start
    // a chmod afterwards leaves the password readable in between
    // create_new follows no link planted at the staging path
    // the rename keeps the mode
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    // a staging file an interrupted run left behind goes first
    // create_new refuses it
    let _ = fs::remove_file(&tmp_path);
    let mut file = opts.open(&tmp_path).with_context(saving)?;
    // the staging file holds the password from here on
    // a step that fails takes it away again
    let staged = (|| {
        // open filters the mode through the umask
        // the umask can clear owner bits too
        // the mode is restated before the password goes in
        // the file is no wider than 0600 meanwhile
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&tmp_path, fs::Permissions::from_mode(0o600))?;
        }
        file.write_all(serialized.as_bytes())?;
        fs::rename(&tmp_path, path)
    })();
    if let Err(error) = staged {
        let _ = fs::remove_file(&tmp_path);
        return Err(error).with_context(saving);
    }

    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn a_failed_save_names_the_auth_file_and_leaves_no_staging_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("auth.json");
        // a directory where the file goes fails the rename
        fs::create_dir(&path).expect("directory");
        let error = write_auth_at(&path, "user@example.com", "hunter2").expect_err("no file");
        assert!(
            format!("{error:#}").contains("Failed to save the auth file at"),
            "{error:#}"
        );
        assert!(!path.with_extension("tmp").exists());
    }

    #[test]
    fn auth_file_is_created_private() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("auth.json");
        write_auth_at(&path, "user@example.com", "hunter2").expect("write auth");

        let mode = fs::metadata(&path).expect("metadata").permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn a_refused_sign_in_saves_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("auth.json");
        write_auth_at(&path, "user@example.com", "hunter2").expect("seed");
        let Err(error) = write_verified_auth_at(&path, "other@example.com", "wrong", |_, _| {
            Err(anyhow!("HTTP 401 for the account"))
        }) else {
            panic!("a refused sign-in saves nothing");
        };
        assert!(format!("{error:#}").contains("HTTP 401"), "{error:#}");
        let config = load_auth_config(&path, |_| None).expect("config");
        assert_eq!(config.email.as_deref(), Some("user@example.com"));
        assert_eq!(config.password.as_deref(), Some("hunter2"));
    }

    #[test]
    fn the_sign_in_tries_the_credentials_that_are_saved() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("auth.json");
        let mut tried = None;
        write_verified_auth_at(
            &path,
            " user@example.com ",
            "pass word ",
            |email, password| {
                tried = Some((email.to_string(), password.to_string()));
                Ok(())
            },
        )
        .expect("saved");
        assert_eq!(
            tried,
            Some(("user@example.com".to_string(), "pass word ".to_string()))
        );
        let config = load_auth_config(&path, |_| None).expect("config");
        assert_eq!(config.email.as_deref(), Some("user@example.com"));
        assert_eq!(config.password.as_deref(), Some("pass word "));
    }

    #[test]
    fn an_empty_field_is_refused_before_the_sign_in() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("auth.json");
        let result = write_verified_auth_at(&path, "user@example.com", "", |_, _| {
            panic!("no sign-in without a password")
        });
        assert!(result.is_err());
        assert!(!path.exists());
    }

    #[test]
    fn a_password_made_of_digits_is_read_as_text() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("auth.json");
        fs::write(
            &path,
            r#"{"email":"user@example.com","password":"27182818"}"#,
        )
        .expect("seed");
        let config = load_auth_config(&path, |_| None).expect("config");
        assert_eq!(config.password.as_deref(), Some("27182818"));

        let config = load_auth_config(&path, |name| {
            (name == "THINGS_PASSWORD").then(|| "0031415".into())
        })
        .expect("config");
        assert_eq!(config.email.as_deref(), Some("user@example.com"));
        assert_eq!(config.password.as_deref(), Some("0031415"));
    }

    #[test]
    fn a_field_that_is_not_a_string_is_refused_without_being_repeated() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("auth.json");
        fs::write(&path, r#"{"email":"user@example.com","password":31415926}"#).expect("seed");
        let Err(error) = load_auth_config(&path, |_| None) else {
            panic!("a number is not a password");
        };
        let text = format!("{error:#}");
        assert!(text.contains("password"), "{text}");
        assert!(!text.contains("31415926"), "{text}");
    }

    #[test]
    fn an_empty_variable_counts_as_unset() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("auth.json");
        fs::write(
            &path,
            r#"{"email":"user@example.com","password":"hunter2"}"#,
        )
        .expect("seed");
        let config = load_auth_config(&path, |_| Some(OsString::new())).expect("config");
        assert_eq!(config.email.as_deref(), Some("user@example.com"));
        assert_eq!(config.password.as_deref(), Some("hunter2"));
    }

    #[test]
    fn a_variable_that_is_not_utf8_is_refused_rather_than_taken_as_unset() {
        use std::os::unix::ffi::OsStringExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("auth.json");
        fs::write(
            &path,
            r#"{"email":"user@example.com","password":"hunter2"}"#,
        )
        .expect("seed");
        let Err(error) = load_auth_config(&path, |name| {
            (name == "THINGS_PASSWORD").then(|| OsString::from_vec(vec![0xff]))
        }) else {
            panic!("the file's password stood in");
        };
        assert!(
            format!("{error:#}").contains("THINGS_PASSWORD"),
            "{error:#}"
        );
    }

    #[test]
    fn a_file_that_is_not_an_object_is_refused_without_being_repeated() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("auth.json");
        for raw in [
            "31415926",
            r#""hunter2""#,
            r#"["user@example.com","hunter2"]"#,
        ] {
            fs::write(&path, raw).expect("seed");
            let Err(error) = load_auth_config(&path, |_| None) else {
                panic!("{raw} is no auth file");
            };
            let text = format!("{error:#}");
            assert!(
                !text.contains("31415926") && !text.contains("hunter2"),
                "{text}"
            );
        }
    }

    #[test]
    fn a_missing_file_leaves_both_fields_to_the_environment() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("auth.json");
        let config = load_auth_config(&path, |_| None).expect("config");
        assert!(config.email.is_none() && config.password.is_none());
    }

    #[test]
    fn password_is_not_written_through_a_planted_symlink() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("auth.json");
        let victim = dir.path().join("victim");
        fs::write(&victim, "original").expect("seed victim");
        std::os::unix::fs::symlink(&victim, path.with_extension("tmp")).expect("plant symlink");

        write_auth_at(&path, "user@example.com", "hunter2").expect("write auth");

        assert_eq!(
            fs::read_to_string(&victim).expect("victim"),
            "original",
            "the password must not be written through a symlink at the staging path"
        );
    }
}
