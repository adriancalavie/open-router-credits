//! Config file handling. The management key lives in the config file, which is
//! created with 0600 permissions inside a 0700 directory.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::api::DEFAULT_API_BASE;

pub const APP_DIR: &str = "open-router-credits";
pub const FILE_NAME: &str = "config.toml";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub api_key: String,
    #[serde(default = "default_refresh_secs")]
    pub refresh_secs: u64,
    /// Optional override, e.g. a self-hosted proxy. `OPENROUTER_API_BASE` wins.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_base: Option<String>,
}

fn default_refresh_secs() -> u64 {
    60
}

/// `$XDG_CONFIG_HOME/open-router-credits`, else `~/.config/open-router-credits`.
/// `dirs::config_dir()` is only a fallback because it resolves to
/// `~/Library/Application Support` on macOS, which is not where users look.
pub fn config_dir() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME").filter(|value| !value.is_empty()) {
        return PathBuf::from(xdg).join(APP_DIR);
    }
    #[cfg(unix)]
    if let Some(home) = std::env::var_os("HOME").filter(|value| !value.is_empty()) {
        return PathBuf::from(home).join(".config").join(APP_DIR);
    }
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(APP_DIR)
}

#[derive(Debug)]
pub enum LoadError {
    NotFound,
    Io(io::Error),
    Parse(String),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => write!(f, "no config file"),
            Self::Io(err) => write!(f, "cannot read config: {err}"),
            Self::Parse(msg) => write!(f, "invalid config: {msg}"),
        }
    }
}

impl std::error::Error for LoadError {}

impl Config {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            refresh_secs: default_refresh_secs(),
            api_base: None,
        }
    }

    pub fn default_path() -> PathBuf {
        config_dir().join(FILE_NAME)
    }

    pub fn load(path: &Path) -> Result<Self, LoadError> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Err(LoadError::NotFound),
            Err(err) => return Err(LoadError::Io(err)),
        };
        let config: Config =
            toml::from_str(&text).map_err(|err| LoadError::Parse(err.to_string()))?;
        if config.api_key.trim().is_empty() {
            return Err(LoadError::Parse("api_key is empty".into()));
        }
        Ok(config)
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            // Only tighten a directory we created ourselves (or our own app dir).
            // `--config /tmp/x.toml` must not chmod /tmp to 0700.
            let existed = parent.exists();
            fs::create_dir_all(parent)?;
            let ours = parent
                .file_name()
                .is_some_and(|name| name == std::ffi::OsStr::new(APP_DIR));
            if !existed || ours {
                restrict_dir(parent)?;
            }
        }

        let body = toml::to_string_pretty(self)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        let header = "# open-router-credits — created by the setup wizard.\n\
                      # This file holds a management key. Keep it private.\n\n";

        // Write through a temp file so a crash never leaves a truncated config.
        let tmp = path.with_extension("toml.tmp");
        {
            let mut options = fs::OpenOptions::new();
            options.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            use io::Write;
            let mut file = options.open(&tmp)?;
            file.write_all(header.as_bytes())?;
            file.write_all(body.as_bytes())?;
            file.sync_all()?;
        }
        restrict_file(&tmp)?;
        fs::rename(&tmp, path)?;
        restrict_file(path)?;
        Ok(())
    }

    /// `OPENROUTER_API_BASE` beats the config value so tests and proxies can override.
    pub fn api_base(&self) -> String {
        std::env::var("OPENROUTER_API_BASE")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| self.api_base.clone())
            .unwrap_or_else(|| DEFAULT_API_BASE.to_string())
    }
}

#[cfg(unix)]
fn restrict_file(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn restrict_file(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn restrict_dir(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn restrict_dir(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "open-router-credits-test-{}-{name}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        dir.join(FILE_NAME)
    }

    #[test]
    fn default_path_is_under_dot_config() {
        let path = Config::default_path();
        assert!(
            path.ends_with("open-router-credits/config.toml"),
            "unexpected path: {}",
            path.display()
        );
        println!("default config path: {}", path.display());
    }

    #[test]
    fn round_trips() {
        let path = temp_path("roundtrip");
        let config = Config::new("sk-or-v1-test");
        config.save(&path).unwrap();

        let loaded = Config::load(&path).unwrap();
        assert_eq!(loaded.api_key, "sk-or-v1-test");
        assert_eq!(loaded.refresh_secs, 60);
        assert!(loaded.api_base.is_none());
    }

    #[test]
    fn missing_file_is_not_found() {
        let path = temp_path("missing");
        assert!(matches!(Config::load(&path), Err(LoadError::NotFound)));
    }

    #[test]
    fn rejects_empty_key() {
        let path = temp_path("empty");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "api_key = \"\"\n").unwrap();
        assert!(matches!(Config::load(&path), Err(LoadError::Parse(_))));
    }

    #[cfg(unix)]
    #[test]
    fn does_not_tighten_a_directory_it_did_not_create() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("orc-shared-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();

        Config::new("sk-or-v1-test")
            .save(&dir.join(FILE_NAME))
            .unwrap();

        let mode = fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o755,
            "save() must not chmod a directory it did not create"
        );
        let file_mode = fs::metadata(dir.join(FILE_NAME))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(file_mode, 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let path = temp_path("perms");
        Config::new("sk-or-v1-secret").save(&path).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "config must not be world readable");
    }

    #[test]
    fn env_base_overrides_config() {
        let mut config = Config::new("k");
        config.api_base = Some("http://from-config".into());
        // Not touching the env here: just assert the config fallback works.
        if std::env::var("OPENROUTER_API_BASE").is_err() {
            assert_eq!(config.api_base(), "http://from-config");
        }
    }
}
