use blake3::Hasher;

/// Proof-of-Work difficulty: number of leading zero bits required.
const DEFAULT_DIFFICULTY: u8 = 16;

/// Maximum nonce value to try before giving up.
const MAX_NONCE: u64 = 10_000_000;

/// A proof-of-work solution.
#[derive(Debug, Clone)]
pub struct PowSolution {
    pub nonce: u64,
    pub difficulty: u8,
    pub hash: [u8; 32],
}

/// Generate a PoW solution for the given input (typically a node_id or public key).
///
/// Finds a nonce such that `blake3(input || nonce)` has at least `difficulty`
/// leading zero bits. This is used for sybil resistance — generating a valid
/// identity requires non-trivial CPU work, making it expensive to create
/// large numbers of identities.
pub fn mine_pow(input: &[u8], difficulty: u8) -> Option<PowSolution> {
    let threshold = difficulty_to_threshold(difficulty)?;
    for nonce in 0..MAX_NONCE {
        let hash = hash_pow(input, nonce);
        if meets_difficulty(&hash, &threshold) {
            return Some(PowSolution {
                nonce,
                difficulty,
                hash,
            });
        }
    }
    None
}

/// Verify a PoW solution.
///
/// Returns `true` if `blake3(input || nonce)` meets the claimed difficulty.
pub fn verify_pow(input: &[u8], solution: &PowSolution) -> bool {
    let hash = hash_pow(input, solution.nonce);
    if hash != solution.hash {
        return false;
    }
    let threshold = match difficulty_to_threshold(solution.difficulty) {
        Some(t) => t,
        None => return false,
    };
    meets_difficulty(&hash, &threshold)
}

/// Compute `blake3(input || nonce_bytes)`.
fn hash_pow(input: &[u8], nonce: u64) -> [u8; 32] {
    let mut hasher = Hasher::new();
    hasher.update(input);
    hasher.update(&nonce.to_le_bytes());
    hasher.finalize().into()
}

/// Convert a difficulty (number of leading zero bits) to a byte threshold.
///
/// For difficulty d, the first d/8 bytes must be zero, and the next byte
/// must have its top (d%8) bits zero.
///
/// Returns None if difficulty > 256 (impossible for a 32-byte hash).
fn difficulty_to_threshold(difficulty: u8) -> Option<[u8; 32]> {
    if difficulty == u8::MAX {
        return None;
    }
    let mut threshold = [0u8; 32];
    // Start with all bytes as 0xFF (any value passes)
    for b in threshold.iter_mut() {
        *b = 0xFF;
    }

    let full_bytes = (difficulty / 8) as usize;
    let remaining_bits = difficulty % 8;

    // First `full_bytes` bytes must be zero
    for i in 0..full_bytes {
        threshold[i] = 0x00;
    }

    // The partial byte: top `remaining_bits` bits must be zero
    if remaining_bits > 0 && full_bytes < 32 {
        threshold[full_bytes] = 0xFF >> remaining_bits;
    } else if remaining_bits == 0 && full_bytes < 32 && full_bytes > 0 {
        // When difficulty is a multiple of 8, the byte at full_bytes is already 0xFF
        // (no partial constraint), which is correct
    }

    Some(threshold)
}

/// Check whether a hash meets the difficulty threshold.
///
/// A hash meets difficulty if it is lexicographically less than or equal to
/// the threshold. For leading-zero-bit difficulty, this means the hash has
/// enough leading zero bits.
fn meets_difficulty(hash: &[u8; 32], threshold: &[u8; 32]) -> bool {
    for i in 0..32 {
        if hash[i] < threshold[i] {
            return true;
        }
        if hash[i] > threshold[i] {
            return false;
        }
    }
    true // equal
}

/// Return the default PoW difficulty.
pub fn default_difficulty() -> u8 {
    DEFAULT_DIFFICULTY
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mine_and_verify_pow() {
        // Use low difficulty for fast tests
        let input = b"test-node-id";
        let solution = mine_pow(input, 8).expect("should find solution at difficulty 8");
        assert!(verify_pow(input, &solution));
    }

    #[test]
    fn verify_rejects_wrong_input() {
        let input = b"test-node-id";
        let solution = mine_pow(input, 8).expect("should find solution");
        assert!(!verify_pow(b"wrong-input", &solution));
    }

    #[test]
    fn verify_rejects_wrong_nonce() {
        let input = b"test-node-id";
        let mut solution = mine_pow(input, 8).expect("should find solution");
        solution.nonce += 1;
        assert!(!verify_pow(input, &solution));
    }

    #[test]
    fn difficulty_to_threshold_works() {
        // 0 bits: everything passes
        let t = difficulty_to_threshold(0).unwrap();
        assert_eq!(t[0], 0xFF);

        // 8 bits: first byte must be 0
        let t = difficulty_to_threshold(8).unwrap();
        assert_eq!(t[0], 0x00);
        assert_eq!(t[1], 0xFF);

        // 16 bits: first two bytes must be 0
        let t = difficulty_to_threshold(16).unwrap();
        assert_eq!(t[0], 0x00);
        assert_eq!(t[1], 0x00);
        assert_eq!(t[2], 0xFF);

        // 12 bits: first byte 0, top 4 bits of second byte 0
        let t = difficulty_to_threshold(12).unwrap();
        assert_eq!(t[0], 0x00);
        assert_eq!(t[1], 0x0F); // 0xFF >> 4 = 0x0F
        assert_eq!(t[2], 0xFF);
    }

    #[test]
    fn difficulty_over_255_rejected() {
        assert!(difficulty_to_threshold(254).is_some());
        assert!(difficulty_to_threshold(255).is_none());
    }

    #[test]
    fn meets_difficulty_check() {
        let mut hash = [0u8; 32];
        hash[0] = 0x00;
        hash[1] = 0x00;
        let threshold = difficulty_to_threshold(16).unwrap();
        assert!(meets_difficulty(&hash, &threshold));

        hash[0] = 0xFF;
        assert!(!meets_difficulty(&hash, &threshold));
    }
}
