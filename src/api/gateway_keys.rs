/// Gateway API key management — Blake3-hashed storage, per-key rate limiting.
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use rand::Rng;

/// In-memory rate limit tracker: identifier → (count, window_start_secs)
struct RateLimitEntry {
    count: u32,
    window_start: u64,
}

/// Gateway key store — keys are stored hashed (Blake3) in the redb meta table.
/// The raw secret is shown only once at creation time.
pub struct GatewayKeyStore {
    data_dir: PathBuf,
    rate_limits: Mutex<HashMap<String, RateLimitEntry>>,
}

/// A gateway key record (as stored/displayed, never includes the raw secret).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GatewayKeyInfo {
    pub nickname: String,
    pub key_hash: String,
    pub created_at: u64,
    pub last_used: u64,
    pub request_count: u64,
}

impl GatewayKeyStore {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            rate_limits: Mutex::new(HashMap::new()),
        }
    }

    /// Create a new API key. Returns the raw secret (shown once) and the key info.
    pub fn create_key(&self, nickname: &str) -> Result<(String, GatewayKeyInfo), String> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // Generate 256-bit random secret
        let secret_bytes: [u8; 32] = rand::rngs::OsRng.gen();
        let secret_hex = hex::encode(secret_bytes);

        // Hash with Blake3 for storage
        let hash = blake3::hash(&secret_bytes).to_hex().to_string();

        let info = GatewayKeyInfo {
            nickname: nickname.to_string(),
            key_hash: hash.clone(),
            created_at: now,
            last_used: 0,
            request_count: 0,
        };

        // Store in gateway_keys.json (simple approach; redb meta table would also work)
        let keys_path = self.data_dir.join("gateway_keys.json");
        let mut keys: Vec<GatewayKeyInfo> = if keys_path.exists() {
            let contents = std::fs::read_to_string(&keys_path).unwrap_or_default();
            serde_json::from_str(&contents).unwrap_or_default()
        } else {
            vec![]
        };

        // Check for duplicate nickname
        if keys.iter().any(|k| k.nickname == nickname) {
            return Err(format!("key with nickname '{}' already exists", nickname));
        }

        keys.push(info.clone());
        let json = serde_json::to_string_pretty(&keys)
            .map_err(|e| format!("serialize gateway keys: {}", e))?;
        std::fs::write(&keys_path, json)
            .map_err(|e| format!("write gateway_keys.json: {}", e))?;

        Ok((secret_hex, info))
    }

    /// List all gateway keys (never includes the raw secret).
    pub fn list_keys(&self) -> Result<Vec<GatewayKeyInfo>, String> {
        let keys_path = self.data_dir.join("gateway_keys.json");
        if !keys_path.exists() {
            return Ok(vec![]);
        }
        let contents = std::fs::read_to_string(&keys_path)
            .map_err(|e| format!("read gateway_keys.json: {}", e))?;
        let keys: Vec<GatewayKeyInfo> = serde_json::from_str(&contents)
            .map_err(|e| format!("parse gateway_keys.json: {}", e))?;
        Ok(keys)
    }

    /// Revoke a key by nickname. Immediate, no grace period.
    pub fn revoke_key(&self, nickname: &str) -> Result<bool, String> {
        let keys_path = self.data_dir.join("gateway_keys.json");
        if !keys_path.exists() {
            return Ok(false);
        }
        let contents = std::fs::read_to_string(&keys_path)
            .map_err(|e| format!("read gateway_keys.json: {}", e))?;
        let mut keys: Vec<GatewayKeyInfo> = serde_json::from_str(&contents)
            .map_err(|e| format!("parse gateway_keys.json: {}", e))?;

        let original_len = keys.len();
        keys.retain(|k| k.nickname != nickname);

        if keys.len() == original_len {
            return Ok(false); // Not found
        }

        let json = serde_json::to_string_pretty(&keys)
            .map_err(|e| format!("serialize gateway keys: {}", e))?;
        std::fs::write(&keys_path, json)
            .map_err(|e| format!("write gateway_keys.json: {}", e))?;

        Ok(true)
    }

    /// Validate an API key by hashing it and checking against stored hashes.
    pub fn validate_key(&self, raw_key: &str) -> bool {
        let secret_bytes = match hex::decode(raw_key) {
            Ok(bytes) if bytes.len() == 32 => bytes,
            _ => return false,
        };
        let hash = blake3::hash(&secret_bytes).to_hex().to_string();

        match self.list_keys() {
            Ok(keys) => keys.iter().any(|k| k.key_hash == hash),
            Err(_) => false,
        }
    }

    /// Check rate limit for an identifier (API key or "anonymous").
    /// Returns true if the request is allowed, false if rate-limited.
    pub fn check_rate_limit(&self, identifier: &str, limit_per_min: u32) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let mut limits = self.rate_limits.lock().unwrap_or_else(|e| e.into_inner());

        let entry = limits.entry(identifier.to_string()).or_insert_with(|| RateLimitEntry {
            count: 0,
            window_start: now,
        });

        // Reset window if more than 60 seconds have passed
        if now - entry.window_start >= 60 {
            entry.count = 0;
            entry.window_start = now;
        }

        entry.count += 1;
        entry.count <= limit_per_min
    }
}

/// Generate a random nickname like "swift-falcon-7x2".
pub fn generate_nickname() -> String {
    let adjectives = [
        "swift", "bright", "calm", "dark", "eager", "fair", "gold", "hazy",
        "iron", "jade", "keen", "lark", "misty", "noble", "opal", "pale",
        "quiet", "rare", "silver", "true", "vivid", "warm", "zephyr",
    ];
    let animals = [
        "falcon", "otter", "raven", "tiger", "viper", "wolf", "bear", "deer",
        "eagle", "fox", "hare", "ibis", "jay", "lynx", "mole", "newt",
        "owl", "puma", "quail", "rook", "stoat", "ursa", "vole", "wren",
    ];
    let mut rng = rand::thread_rng();
    let adj = adjectives[rng.gen_range(0..adjectives.len())];
    let animal = animals[rng.gen_range(0..animals.len())];
    let suffix: u16 = rng.gen_range(10..99);
    let letter = (b'a' + (rng.gen_range(0..26) as u8)) as char;
    format!("{}-{}-{}{}", adj, animal, suffix, letter)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn create_and_validate_key() {
        let dir = TempDir::new().unwrap();
        let store = GatewayKeyStore::new(dir.path().to_path_buf());

        let (secret, info) = store.create_key("test-key").unwrap();
        assert_eq!(info.nickname, "test-key");
        assert!(secret.len() == 64); // 32 bytes hex-encoded

        // Validate the key
        assert!(store.validate_key(&secret));
        // Invalid key
        assert!(!store.validate_key("0000000000000000000000000000000000000000000000000000000000000000"));
    }

    #[test]
    fn list_keys() {
        let dir = TempDir::new().unwrap();
        let store = GatewayKeyStore::new(dir.path().to_path_buf());

        store.create_key("key1").unwrap();
        store.create_key("key2").unwrap();

        let keys = store.list_keys().unwrap();
        assert_eq!(keys.len(), 2);
    }

    #[test]
    fn revoke_key() {
        let dir = TempDir::new().unwrap();
        let store = GatewayKeyStore::new(dir.path().to_path_buf());

        let (secret, _) = store.create_key("key1").unwrap();
        assert!(store.validate_key(&secret));

        assert!(store.revoke_key("key1").unwrap());
        assert!(!store.validate_key(&secret));
        assert!(!store.revoke_key("key1").unwrap()); // Already revoked
    }

    #[test]
    fn duplicate_nickname_rejected() {
        let dir = TempDir::new().unwrap();
        let store = GatewayKeyStore::new(dir.path().to_path_buf());

        store.create_key("my-key").unwrap();
        assert!(store.create_key("my-key").is_err());
    }

    #[test]
    fn rate_limiting() {
        let dir = TempDir::new().unwrap();
        let store = GatewayKeyStore::new(dir.path().to_path_buf());

        // Allow 3 per minute
        assert!(store.check_rate_limit("test-id", 3));
        assert!(store.check_rate_limit("test-id", 3));
        assert!(store.check_rate_limit("test-id", 3));
        assert!(!store.check_rate_limit("test-id", 3)); // 4th should be rejected
    }

    #[test]
    fn generate_nickname_format() {
        let name = generate_nickname();
        let parts: Vec<&str> = name.split('-').collect();
        assert_eq!(parts.len(), 3);
    }
}
