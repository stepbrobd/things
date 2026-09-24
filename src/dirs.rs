use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Result, anyhow};

const APP_NAME: &str = "things";

/// the XDG base directory named by `var`, on every platform
///
/// `default` under the home directory stands in when the variable is unset, empty or relative
/// the XDG spec has an implementation ignore a relative path
fn xdg_home(var: &str, default: &[&str]) -> Result<PathBuf> {
    xdg_home_from(std::env::var_os(var), dirs::home_dir(), var, default)
}

fn xdg_home_from(
    value: Option<OsString>,
    home: Option<PathBuf>,
    var: &str,
    default: &[&str],
) -> Result<PathBuf> {
    match value {
        Some(custom) if Path::new(&custom).is_absolute() => Ok(PathBuf::from(custom)),
        _ => {
            // without a home the files would land in whatever directory the command runs in
            // a relative home would put them there too
            let mut path = home
                .filter(|home| home.is_absolute())
                .ok_or_else(|| anyhow!("No absolute home directory, set {var}."))?;
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

/// create `dir` and narrow it to its owner
///
/// the config directory holds the Things Cloud password and the state directory a journal with every title and note, which the default 0755 leaves readable to every local user
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

    #[test]
    fn a_relative_xdg_directory_counts_as_unset() {
        let home = PathBuf::from("/home/ada");
        for value in [None, Some(OsString::new()), Some("relstate".into())] {
            assert_eq!(
                xdg_home_from(
                    value,
                    Some(home.clone()),
                    "XDG_STATE_HOME",
                    &[".local", "state"]
                )
                .expect("dir"),
                home.join(".local").join("state")
            );
        }
        assert_eq!(
            xdg_home_from(
                Some("/srv/state".into()),
                None,
                "XDG_STATE_HOME",
                &[".local", "state"]
            )
            .expect("dir"),
            PathBuf::from("/srv/state")
        );
    }

    #[test]
    fn a_relative_home_counts_as_none() {
        for home in [None, Some(PathBuf::from("rel"))] {
            assert!(xdg_home_from(None, home, "XDG_STATE_HOME", &[".local", "state"]).is_err());
        }
    }

    #[test]
    fn an_absolute_xdg_directory_need_not_be_utf8() {
        use std::os::unix::ffi::OsStringExt;
        let value = OsString::from_vec(b"/srv/st\xffte".to_vec());
        assert_eq!(
            xdg_home_from(
                Some(value.clone()),
                None,
                "XDG_STATE_HOME",
                &[".local", "state"]
            )
            .expect("dir"),
            PathBuf::from(value)
        );
    }

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
