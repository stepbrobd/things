use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Result, anyhow};

const APP_NAME: &str = "things";

/// The XDG base directory named by `var`, or `default` under the home
/// directory when the variable is unset or empty, on every platform.
fn xdg_home(var: &str, default: &[&str]) -> Result<PathBuf> {
    match std::env::var(var) {
        Ok(custom) if !custom.is_empty() => Ok(PathBuf::from(custom)),
        _ => {
            // without a home the files would land in whatever directory the command runs in
            let mut path =
                dirs::home_dir().ok_or_else(|| anyhow!("No home directory, set {var}."))?;
            path.extend(default);
            Ok(path)
        }
    }
}

pub fn append_log_dir() -> Result<PathBuf> {
    Ok(xdg_home("XDG_STATE_HOME", &[".local", "state"])?
        .join(APP_NAME)
        .join("append-log"))
}

pub fn auth_file_path() -> Result<PathBuf> {
    Ok(xdg_home("XDG_CONFIG_HOME", &[".config"])?
        .join(APP_NAME)
        .join("auth.json"))
}

/// Create `dir` and narrow it to owner-only. The config directory holds the
/// Things Cloud password and the state directory an append log carrying every
/// task title and note, so the default 0755 leaves that readable by any other
/// local user.
pub fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn mode_of(path: &Path) -> u32 {
        fs::metadata(path).expect("metadata").permissions().mode() & 0o777
    }

    #[test]
    fn creates_the_directory_private() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path().join("things").join("append-log");
        create_private_dir(&dir).expect("create");
        assert_eq!(mode_of(&dir), 0o700);
    }

    #[test]
    fn narrows_an_existing_world_readable_directory() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path().join("things");
        fs::create_dir_all(&dir).expect("seed");
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).expect("widen");
        create_private_dir(&dir).expect("create");
        assert_eq!(mode_of(&dir), 0o700);
    }
}
