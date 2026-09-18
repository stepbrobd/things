use std::{
    fs,
    io::{ErrorKind, Write},
    path::Path,
};

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::dirs::{auth_file_path, create_private_dir};

#[derive(Serialize)]
struct AuthPayload {
    email: String,
    password: String,
}

/// the auth file as written, each field taken as it is, which keeps a
/// password made of digits a password
#[derive(Deserialize, Default)]
struct AuthFile {
    #[serde(default)]
    email: Option<Value>,
    #[serde(default)]
    password: Option<Value>,
}

struct AuthConfig {
    email: Option<String>,
    password: Option<String>,
}

/// a field of the auth file as text, refused in any other shape without repeating the value
fn text_field(value: Option<Value>, field: &str, path: &Path) -> Result<Option<String>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text)),
        Some(_) => Err(anyhow!(
            "The {field} in {} is not a JSON string, quote it",
            path.display()
        )),
    }
}

/// the auth file under `THINGS_EMAIL` and `THINGS_PASSWORD`, each variable standing in for the file's field when `var` yields it, as the text it holds
fn load_auth_config(path: &Path, var: impl Fn(&str) -> Option<String>) -> Result<AuthConfig> {
    let file = match fs::read_to_string(path) {
        Ok(raw) => serde_json::from_str::<AuthFile>(&raw)
            .with_context(|| format!("Failed reading auth config at {}", path.display()))?,
        Err(error) if error.kind() == ErrorKind::NotFound => AuthFile::default(),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("Failed reading auth config at {}", path.display()));
        }
    };
    let email = match var("THINGS_EMAIL") {
        Some(text) => Some(text),
        None => text_field(file.email, "email", path)?,
    };
    let password = match var("THINGS_PASSWORD") {
        Some(text) => Some(text),
        None => text_field(file.password, "password", path)?,
    };
    Ok(AuthConfig { email, password })
}

fn validate_auth(email: &str, password: &str) -> Result<(String, String)> {
    let email = email.trim().to_string();
    let password = password.to_string();

    if email.is_empty() {
        return Err(anyhow!("Missing auth email."));
    }
    if password.is_empty() {
        return Err(anyhow!("Missing auth password."));
    }

    Ok((email, password))
}

pub fn load_auth() -> Result<(String, String)> {
    let path = auth_file_path();

    let cfg = load_auth_config(&path, |name| std::env::var(name).ok())?;

    let Some(email) = cfg.email else {
        return Err(anyhow!(
            "Missing auth email. Set THINGS_EMAIL or run `things auth` to create {}.",
            path.display()
        ));
    };

    let Some(password) = cfg.password else {
        return Err(anyhow!(
            "Missing auth password. Set THINGS_PASSWORD or run `things auth` to update {}.",
            path.display()
        ));
    };

    validate_auth(&email, &password)
}

pub fn write_auth(email: &str, password: &str) -> Result<std::path::PathBuf> {
    let path = auth_file_path();
    write_auth_at(&path, email, password)?;
    Ok(path)
}

fn write_auth_at(path: &Path, email: &str, password: &str) -> Result<()> {
    let (email, password) = validate_auth(email, password)?;
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("Invalid auth file path"))?;
    create_private_dir(parent).with_context(|| format!("Failed creating {}", parent.display()))?;

    let payload = AuthPayload { email, password };
    let serialized = serde_json::to_string(&payload)?;
    let tmp_path = path.with_extension("tmp");

    // Create the staging file already private: chmod'ing after the fact leaves
    // the plaintext password world-readable in between, and create_new won't
    // follow a symlink planted at tmp_path. The rename carries the mode over.
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    // Clear a stale staging file from an interrupted run; create_new refuses it.
    let _ = fs::remove_file(&tmp_path);
    let mut file = opts
        .open(&tmp_path)
        .with_context(|| format!("Failed writing {}", tmp_path.display()))?;
    // open() filters the requested mode through the umask, which can clear
    // owner bits too, so restate it before the password goes in. The file is
    // already no wider than 0600, so this opens no window.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp_path, fs::Permissions::from_mode(0o600))
            .with_context(|| format!("Failed securing {}", tmp_path.display()))?;
    }
    file.write_all(serialized.as_bytes())
        .with_context(|| format!("Failed writing {}", tmp_path.display()))?;

    fs::rename(&tmp_path, path).with_context(|| format!("Failed finalizing {}", path.display()))?;

    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn auth_file_is_created_private() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("auth.json");
        write_auth_at(&path, "user@example.com", "hunter2").expect("write auth");

        let mode = fs::metadata(&path).expect("metadata").permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
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
            (name == "THINGS_PASSWORD").then(|| "0031415".to_string())
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
