use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub default_provider: Option<String>,

    #[serde(default)]
    pub remotes: HashMap<String, RemoteConfig>,

    #[serde(default)]
    pub providers: ProvidersConfig,

    #[serde(default)]
    pub hosts: HashMap<String, String>,

    /// Read from the global config only; see [`Config::load`].
    #[serde(default)]
    pub writes: WritesConfig,
}

/// Tools that change a remote: `pr_merge`, `git_push`, `git_pull`,
/// `git_fetch`. Off unless the user's global config turns them on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WritesConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Ask the user through the client (MCP elicitation) before each merge or
    /// push. A client without elicitation is refused; `false` lets writes run
    /// unattended.
    #[serde(default = "confirm_by_default")]
    pub confirm: bool,
}

const fn confirm_by_default() -> bool {
    true
}

impl Default for WritesConfig {
    fn default() -> Self {
        Self { enabled: false, confirm: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteConfig {
    pub provider: String,

    #[serde(default)]
    pub url: Option<String>,

    #[serde(default)]
    pub cli_args: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProvidersConfig {
    #[serde(default)]
    pub github: Option<ProviderSettings>,

    #[serde(default)]
    pub gitlab: Option<ProviderSettings>,

    #[serde(default)]
    pub gitea: Option<ProviderSettings>,

    #[serde(default)]
    pub forgejo: Option<ProviderSettings>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderSettings {
    #[serde(default)]
    pub url: Option<String>,

    #[serde(default)]
    pub default_org: Option<String>,

    #[serde(default)]
    pub cli_args: Vec<String>,

    #[serde(default)]
    pub disabled: bool,
}

impl Config {
    /// The workspace's `skipper.toml` over the global config. `[writes]` comes
    /// from the global config alone: a checked-out repository must not be able
    /// to turn on merging or pushing for whoever opens it.
    pub fn load() -> Self {
        let global = dirs::config_dir().map(|p| p.join("skipper/config.toml"));
        Self::load_with(Path::new("skipper.toml"), global.as_deref())
    }

    /// Each file is read once: the global config is the base, `[writes]`
    /// included, and the local one merges over it without `[writes]`.
    pub fn load_with(local: &Path, global: Option<&Path>) -> Self {
        let mut config =
            global.filter(|path| path.exists()).and_then(Self::read_logged).unwrap_or_default();
        if local.exists()
            && let Some(local) = Self::read_logged(local)
        {
            config = config.merge(local);
        }
        config
    }

    /// A config that fails to read or parse is skipped, loudly.
    fn read_logged(path: &Path) -> Option<Self> {
        match Self::load_from(path) {
            Ok(config) => Some(config),
            Err(e) => {
                tracing::warn!(error = %e, "ignoring skipper config");
                None
            }
        }
    }

    pub fn load_from(path: &Path) -> Result<Self, ConfigError> {
        let contents =
            std::fs::read_to_string(path).map_err(|e| ConfigError::Io(path.to_path_buf(), e))?;

        toml::from_str(&contents).map_err(|e| ConfigError::Parse(path.to_path_buf(), e.to_string()))
    }

    /// `other` over `self`; `[writes]` stays `self`'s.
    fn merge(mut self, other: Self) -> Self {
        self.default_provider = other.default_provider.or(self.default_provider);
        self.remotes.extend(other.remotes);
        self.hosts.extend(other.hosts);

        let (ours, theirs) = (&mut self.providers, other.providers);
        ours.github = theirs.github.or_else(|| ours.github.take());
        ours.gitlab = theirs.gitlab.or_else(|| ours.gitlab.take());
        ours.gitea = theirs.gitea.or_else(|| ours.gitea.take());
        ours.forgejo = theirs.forgejo.or_else(|| ours.forgejo.take());

        self
    }

    pub fn remote(&self, name: &str) -> Option<&RemoteConfig> {
        self.remotes.get(name)
    }

    pub fn provider_settings(&self, provider: &str) -> Option<&ProviderSettings> {
        match provider {
            "github" => self.providers.github.as_ref(),
            "gitlab" => self.providers.gitlab.as_ref(),
            "gitea" | "tea" => self.providers.gitea.as_ref(),
            "forgejo" => self.providers.forgejo.as_ref(),
            _ => None,
        }
    }

    pub fn is_disabled(&self, provider: &str) -> bool {
        self.provider_settings(provider).is_some_and(|s| s.disabled)
    }

    pub fn provider_url(&self, provider: &str) -> Option<&str> {
        self.provider_settings(provider).and_then(|s| s.url.as_deref())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("failed to read {0}: {1}")]
    Io(PathBuf, std::io::Error),

    #[error("failed to parse {0}: {1}")]
    Parse(PathBuf, String),
}

mod dirs {
    use std::path::PathBuf;

    pub fn config_dir() -> Option<PathBuf> {
        #[cfg(target_os = "macos")]
        {
            std::env::var("HOME").ok().map(|h| PathBuf::from(h).join(".config"))
        }

        #[cfg(target_os = "linux")]
        {
            std::env::var("XDG_CONFIG_HOME")
                .ok()
                .map(PathBuf::from)
                .or_else(|| std::env::var("HOME").ok().map(|h| PathBuf::from(h).join(".config")))
        }

        #[cfg(target_os = "windows")]
        {
            std::env::var("APPDATA").ok().map(PathBuf::from)
        }

        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
        {
            None
        }
    }
}

#[cfg(test)]
mod tests;
