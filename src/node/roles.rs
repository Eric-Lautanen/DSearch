use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeRole {
    Light,
    Full,
    Bootstrap,
    Relay,
    Stun,
    Scraper,
    Archive,
    Indexer,
    Gateway,
    Observer,
}

impl NodeRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Full => "full",
            Self::Bootstrap => "bootstrap",
            Self::Relay => "relay",
            Self::Stun => "stun",
            Self::Scraper => "scraper",
            Self::Archive => "archive",
            Self::Indexer => "indexer",
            Self::Gateway => "gateway",
            Self::Observer => "observer",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "light" => Some(Self::Light),
            "full" => Some(Self::Full),
            "bootstrap" => Some(Self::Bootstrap),
            "relay" => Some(Self::Relay),
            "stun" => Some(Self::Stun),
            "scraper" => Some(Self::Scraper),
            "archive" => Some(Self::Archive),
            "indexer" => Some(Self::Indexer),
            "gateway" => Some(Self::Gateway),
            "observer" => Some(Self::Observer),
            _ => None,
        }
    }

    pub fn all() -> &'static [NodeRole] {
        &[
            Self::Light,
            Self::Full,
            Self::Bootstrap,
            Self::Relay,
            Self::Stun,
            Self::Scraper,
            Self::Archive,
            Self::Indexer,
            Self::Gateway,
            Self::Observer,
        ]
    }
}

impl std::fmt::Display for NodeRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
