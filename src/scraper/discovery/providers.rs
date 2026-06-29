use serde::{Deserialize, Serialize};
use std::path::Path;

/// A search provider configuration entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchProvider {
    pub name: String,
    pub enabled: bool,
    pub endpoint: String,
}

/// The full search_providers.toml structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchProvidersToml {
    #[serde(default)]
    pub providers: Vec<SearchProvider>,
}

/// Load search providers from search_providers.toml.
pub fn load_providers(data_dir: &Path) -> Vec<SearchProvider> {
    let path = data_dir.join("search_providers.toml");
    if !path.exists() {
        return Vec::new();
    }
    let contents = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let config: SearchProvidersToml = match toml::from_str(&contents) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    config.providers.into_iter().filter(|p| p.enabled).collect()
}

/// Resolve a keyword into a list of target URLs by querying enabled search providers.
/// Each provider's endpoint is called with the keyword as a query parameter.
/// Returns a list of (provider_name, url) pairs to scrape.
pub fn resolve_keyword_to_urls(
    providers: &[SearchProvider],
    keyword: &str,
) -> Vec<(String, String)> {
    let mut urls = Vec::new();
    for provider in providers {
        if !provider.enabled {
            continue;
        }
        // Build the query URL: endpoint?q=<keyword>
        let separator = if provider.endpoint.contains('?') {
            "&"
        } else {
            "?"
        };
        let url = format!(
            "{}{}q={}",
            provider.endpoint,
            separator,
            keyword.replace(' ', "+")
        );
        urls.push((provider.name.clone(), url));
    }
    urls
}

/// Add a provider to search_providers.toml.
pub fn add_provider(data_dir: &Path, name: &str, endpoint: &str) -> Result<(), String> {
    let path = data_dir.join("search_providers.toml");
    let mut config = if path.exists() {
        let contents = std::fs::read_to_string(&path).map_err(|e| format!("read: {}", e))?;
        toml::from_str::<SearchProvidersToml>(&contents).unwrap_or_else(|_| SearchProvidersToml {
            providers: Vec::new(),
        })
    } else {
        SearchProvidersToml {
            providers: Vec::new(),
        }
    };

    // Check for duplicate name
    if config.providers.iter().any(|p| p.name == name) {
        return Err(format!("provider '{}' already exists", name));
    }

    config.providers.push(SearchProvider {
        name: name.to_string(),
        enabled: true,
        endpoint: endpoint.to_string(),
    });

    let toml_str = toml::to_string_pretty(&config).map_err(|e| format!("serialize: {}", e))?;
    std::fs::write(&path, toml_str).map_err(|e| format!("write: {}", e))?;

    Ok(())
}

/// Remove a provider by name from search_providers.toml.
pub fn remove_provider(data_dir: &Path, name: &str) -> Result<bool, String> {
    let path = data_dir.join("search_providers.toml");
    if !path.exists() {
        return Ok(false);
    }

    let contents = std::fs::read_to_string(&path).map_err(|e| format!("read: {}", e))?;
    let mut config =
        toml::from_str::<SearchProvidersToml>(&contents).map_err(|e| format!("parse: {}", e))?;

    let before = config.providers.len();
    config.providers.retain(|p| p.name != name);
    if config.providers.len() == before {
        return Ok(false);
    }

    let toml_str = toml::to_string_pretty(&config).map_err(|e| format!("serialize: {}", e))?;
    std::fs::write(&path, toml_str).map_err(|e| format!("write: {}", e))?;

    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_resolve_keyword_to_urls() {
        let providers = vec![
            SearchProvider {
                name: "test".to_string(),
                enabled: true,
                endpoint: "https://search.example.com/v1".to_string(),
            },
            SearchProvider {
                name: "disabled".to_string(),
                enabled: false,
                endpoint: "https://disabled.example.com/v1".to_string(),
            },
        ];
        let urls = resolve_keyword_to_urls(&providers, "rust lang");
        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0].0, "test");
        assert!(urls[0].1.contains("q=rust+lang"));
    }

    #[test]
    fn test_resolve_keyword_with_existing_query() {
        let providers = vec![SearchProvider {
            name: "test".to_string(),
            enabled: true,
            endpoint: "https://search.example.com/v1?format=json".to_string(),
        }];
        let urls = resolve_keyword_to_urls(&providers, "test");
        assert_eq!(urls.len(), 1);
        assert!(urls[0].1.contains("&q=test"));
    }

    #[test]
    fn test_add_provider() {
        let dir = TempDir::new().unwrap();
        add_provider(dir.path(), "google", "https://google.com/search").unwrap();
        let providers = load_providers(dir.path());
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].name, "google");
        assert_eq!(providers[0].endpoint, "https://google.com/search");
        assert!(providers[0].enabled);
    }

    #[test]
    fn test_add_duplicate_provider_rejected() {
        let dir = TempDir::new().unwrap();
        add_provider(dir.path(), "google", "https://google.com/search").unwrap();
        let result = add_provider(dir.path(), "google", "https://google.com/search2");
        assert!(result.is_err());
    }

    #[test]
    fn test_remove_provider() {
        let dir = TempDir::new().unwrap();
        add_provider(dir.path(), "google", "https://google.com/search").unwrap();
        add_provider(dir.path(), "bing", "https://bing.com/search").unwrap();
        assert_eq!(load_providers(dir.path()).len(), 2);

        let removed = remove_provider(dir.path(), "google").unwrap();
        assert!(removed);
        let providers = load_providers(dir.path());
        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].name, "bing");
    }

    #[test]
    fn test_remove_nonexistent_provider() {
        let dir = TempDir::new().unwrap();
        let removed = remove_provider(dir.path(), "no-such-provider").unwrap();
        assert!(!removed);
    }

    #[test]
    fn test_load_providers_empty_dir() {
        let dir = TempDir::new().unwrap();
        let providers = load_providers(dir.path());
        assert!(providers.is_empty());
    }

    #[test]
    fn test_load_providers_disabled_filtered() {
        let dir = TempDir::new().unwrap();
        // Write a TOML with a disabled provider
        let toml_content = "[[providers]]\nname = \"off\"\nenabled = false\nendpoint = \"https://off.example.com\"\n";
        std::fs::write(dir.path().join("search_providers.toml"), toml_content).unwrap();
        let providers = load_providers(dir.path());
        assert!(providers.is_empty());
    }

    #[test]
    fn test_add_and_remove_provider_roundtrip() {
        let dir = TempDir::new().unwrap();
        add_provider(dir.path(), "ddg", "https://duckduckgo.com").unwrap();
        assert_eq!(load_providers(dir.path()).len(), 1);
        remove_provider(dir.path(), "ddg").unwrap();
        assert!(load_providers(dir.path()).is_empty());
    }
}
