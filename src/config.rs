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
    pub fn load() -> Self {
        Self::load_from_paths(&[
            PathBuf::from("skipper.toml"),
            dirs::config_dir().map(|p| p.join("skipper/config.toml")).unwrap_or_default(),
        ])
    }

    pub fn load_from_paths(paths: &[PathBuf]) -> Self {
        let mut config = Config::default();

        for path in paths.iter().rev() {
            if path.exists()
                && let Ok(contents) = std::fs::read_to_string(path)
                && let Ok(loaded) = toml::from_str::<Config>(&contents)
            {
                config = config.merge(loaded);
            }
        }

        config
    }

    pub fn load_from(path: &Path) -> Result<Self, ConfigError> {
        let contents =
            std::fs::read_to_string(path).map_err(|e| ConfigError::Io(path.to_path_buf(), e))?;

        toml::from_str(&contents).map_err(|e| ConfigError::Parse(path.to_path_buf(), e.to_string()))
    }

    fn merge(mut self, other: Config) -> Self {
        self.default_provider = other.default_provider.or(self.default_provider);
        self.remotes.extend(other.remotes);
        self.hosts.extend(other.hosts);

        let (ours, theirs) = (&mut self.providers, other.providers);
        ours.github = theirs.github.or(ours.github.take());
        ours.gitlab = theirs.gitlab.or(ours.gitlab.take());
        ours.gitea = theirs.gitea.or(ours.gitea.take());
        ours.forgejo = theirs.forgejo.or(ours.forgejo.take());

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
        self.provider_settings(provider).map(|s| s.disabled).unwrap_or(false)
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
