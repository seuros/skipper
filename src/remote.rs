use std::collections::{BTreeSet, HashMap};

const DEFAULT_HOSTS: &[(&str, &str)] =
    &[("github.com", "github"), ("gitlab.com", "gitlab"), ("codeberg.org", "tea")];

pub fn normalize_provider(name: &str) -> Option<&'static str> {
    match name.trim().to_lowercase().as_str() {
        "github" | "gh" => Some("github"),
        "gitlab" | "glab" => Some("gitlab"),
        "gitea" | "forgejo" | "tea" => Some("tea"),
        _ => None,
    }
}

pub fn host_of(url: &str) -> Option<String> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }

    let authority = if let Some((scheme, rest)) = url.split_once("://") {
        if scheme.eq_ignore_ascii_case("file") {
            return None;
        }
        rest.split(['/', '?', '#']).next().unwrap_or(rest)
    } else if url.starts_with('/') || url.starts_with('.') || url.starts_with('~') {
        return None;
    } else {
        let colon = url.find(':')?;
        if url[..colon].contains('/') {
            return None;
        }
        &url[..colon]
    };

    let authority = authority.rsplit_once('@').map_or(authority, |(_, host)| host);

    let host = if let Some(end) = authority.strip_prefix('[').and_then(|r| r.find(']')) {
        &authority[1..=end]
    } else if authority.matches(':').count() == 1 {
        authority.split_once(':').map_or(authority, |(host, _)| host)
    } else {
        authority
    };

    let host = host.trim().trim_end_matches('.');
    if host.is_empty() { None } else { Some(host.to_lowercase()) }
}

pub fn repo_path_of(url: &str) -> Option<(String, String)> {
    let url = url.trim();

    let path = if let Some((_, rest)) = url.split_once("://") {
        rest.split_once('/').map(|(_, p)| p)?
    } else {
        let colon = url.find(':')?;
        if url[..colon].contains('/') {
            return None;
        }
        &url[colon + 1..]
    };

    let path = path.split(['?', '#']).next().unwrap_or(path);
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);

    let mut segments = path.rsplit('/');
    let name = segments.next()?.trim();
    let owner = segments.next()?.trim();

    if name.is_empty() || owner.is_empty() {
        return None;
    }
    Some((owner.to_string(), name.to_string()))
}

#[derive(Debug, Clone)]
pub struct ForgeHosts {
    hosts: HashMap<String, &'static str>,
}

impl ForgeHosts {
    pub fn with_defaults() -> Self {
        let hosts =
            DEFAULT_HOSTS.iter().map(|(host, provider)| ((*host).to_string(), *provider)).collect();
        Self { hosts }
    }

    pub fn extend(&mut self, mappings: &HashMap<String, String>) {
        for (host, provider) in mappings {
            let Some(provider) = normalize_provider(provider) else {
                tracing::warn!(
                    host = host,
                    provider = provider,
                    "unknown forge in [hosts]; expected github, gitlab, gitea/forgejo"
                );
                continue;
            };
            let key = host_of(host).unwrap_or_else(|| host.trim().to_lowercase());
            self.hosts.insert(key, provider);
        }
    }

    pub fn provider_for_host(&self, host: &str) -> Option<&'static str> {
        self.hosts.get(&host.to_lowercase()).copied()
    }

    pub fn provider_for_url(&self, url: &str) -> Option<&'static str> {
        self.provider_for_host(&host_of(url)?)
    }

    pub fn providers_for_urls<I, S>(&self, urls: I) -> BTreeSet<&'static str>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        urls.into_iter().filter_map(|url| self.provider_for_url(url.as_ref())).collect()
    }

    pub fn unknown_hosts<I, S>(&self, urls: I) -> BTreeSet<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        urls.into_iter()
            .filter_map(|url| host_of(url.as_ref()))
            .filter(|host| self.provider_for_host(host).is_none())
            .collect()
    }
}

impl Default for ForgeHosts {
    fn default() -> Self {
        Self::with_defaults()
    }
}

#[cfg(test)]
mod tests;

pub fn ordered_remotes(
    remotes: std::collections::BTreeMap<String, String>,
) -> Vec<(String, String)> {
    let mut ordered: Vec<(String, String)> = remotes.into_iter().collect();
    ordered.sort_by_key(|(name, _)| name != "origin");
    ordered
}
