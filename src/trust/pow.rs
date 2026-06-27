/// Sybil resistance: Proof-of-Work node ID.
///
/// To create a node ID, a node must find a nonce such that
/// blake3(nonce || public_key) has at least `DIFFICULTY` leading zero bits.
/// This makes mass-generating node IDs expensive, raising the cost of
/// Sybil attacks without requiring a central authority.
use blake3::Hasher;

/// Number of leading zero bits required in the PoW hash.
/// 16 bits = ~65k attempts on average, which takes <1 second on modern hardware
/// but makes generating millions of IDs impractical.
pub const POW_DIFFICULTY: u16 = 16;

/// A Proof-of-Work result.
#[derive(Debug, Clone)]
pub struct PowResult {
    /// The nonce that satisfies the difficulty requirement.
    pub nonce: u64,
    /// The resulting hash.
    pub hash: [u8; 32],
    /// Number of leading zero bits in the hash.
    pub leading_zeros: u16,
}

/// Compute blake3(nonce || public_key_bytes).
fn pow_hash(nonce: u64, public_key_bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Hasher::new();
    hasher.update(&nonce.to_le_bytes());
    hasher.update(public_key_bytes);
    hasher.finalize().into()
}

/// Count the number of leading zero bits in a hash.
pub fn count_leading_zeros(hash: &[u8; 32]) -> u16 {
    let mut count: u16 = 0;
    for &byte in hash.iter() {
        if byte == 0 {
            count += 8;
        } else {
            count += byte.leading_zeros() as u16;
            break;
        }
    }
    count
}

/// Mine a PoW nonce for the given public key.
/// Returns the first nonce that produces a hash with at least `difficulty`
/// leading zero bits.
pub fn mine_pow(public_key_bytes: &[u8], difficulty: u16) -> PowResult {
    let mut nonce: u64 = 0;
    loop {
        let hash = pow_hash(nonce, public_key_bytes);
        let zeros = count_leading_zeros(&hash);
        if zeros >= difficulty {
            return PowResult {
                nonce,
                hash,
                leading_zeros: zeros,
            };
        }
        nonce += 1;
    }
}

/// Verify a PoW claim: that the given nonce produces a hash with at least
/// `difficulty` leading zero bits for the given public key.
pub fn verify_pow(public_key_bytes: &[u8], nonce: u64, difficulty: u16) -> bool {
    let hash = pow_hash(nonce, public_key_bytes);
    count_leading_zeros(&hash) >= difficulty
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_leading_zeros_all_zero() {
        let hash = [0u8; 32];
        assert_eq!(count_leading_zeros(&hash), 256);
    }

    #[test]
    fn count_leading_zeros_first_byte_1() {
        let mut hash = [0u8; 32];
        hash[0] = 1; // 00000001 — 7 leading zeros
        assert_eq!(count_leading_zeros(&hash), 7);
    }

    #[test]
    fn count_leading_zeros_no_zeros() {
        let hash = [0xFFu8; 32];
        assert_eq!(count_leading_zeros(&hash), 0);
    }

    #[test]
    fn mine_and_verify_pow() {
        // Use a low difficulty for fast tests
        let pubkey = b"test_public_key_12345";
        let result = mine_pow(pubkey, 8);
        assert!(result.leading_zeros >= 8);
        assert!(verify_pow(pubkey, result.nonce, 8));
    }

    #[test]
    fn verify_rejects_invalid_nonce() {
        let pubkey = b"test_public_key_12345";
        // A random nonce is unlikely to satisfy difficulty 16
        assert!(!verify_pow(pubkey, 0, 16));
    }

    #[test]
    fn pow_deterministic() {
        let pubkey = b"deterministic_test";
        let hash1 = pow_hash(42, pubkey);
        let hash2 = pow_hash(42, pubkey);
        assert_eq!(hash1, hash2);
    }
}
