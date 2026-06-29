use std::net::UdpSocket;
use std::time::Duration;

/// Simple STUN-like binding request to discover the public address.
/// Sends a classic STUN Binding Request (RFC 5389) to a STUN server
/// and parses the XOR-MAPPED-ADDRESS from the response.
///
/// Returns (ip, port) of the public address as seen by the STUN server.
pub fn stun_bind(stun_addr: &str, timeout: Duration) -> Result<String, String> {
    let socket = UdpSocket::bind("0.0.0.0:0").map_err(|e| format!("bind UDP socket: {}", e))?;
    socket
        .set_read_timeout(Some(timeout))
        .map_err(|e| format!("set read timeout: {}", e))?;

    // Classic STUN Binding Request: 2-byte type (0x0001), 2-byte length (0x0000),
    // 4-byte magic cookie (0x2112A442), 12-byte transaction ID
    let mut request = [0u8; 20];
    request[0] = 0x00;
    request[1] = 0x01; // Binding Request
    request[2] = 0x00;
    request[3] = 0x00; // Length = 0 (no attributes)
    request[4] = 0x21;
    request[5] = 0x12;
    request[6] = 0xA4;
    request[7] = 0x42; // Magic cookie
                       // Transaction ID: use random bytes
    let txn_id: [u8; 12] = rand_transaction_id();
    request[8..20].copy_from_slice(&txn_id);

    socket
        .send_to(&request, stun_addr)
        .map_err(|e| format!("send to STUN server: {}", e))?;

    let mut response = [0u8; 576];
    let (n, _src) = socket
        .recv_from(&mut response)
        .map_err(|e| format!("recv from STUN server: {}", e))?;

    if n < 20 {
        return Err("STUN response too short".to_string());
    }

    // Verify it's a Binding Response (type 0x0101) and same transaction ID
    let msg_type = u16::from_be_bytes([response[0], response[1]]);
    if msg_type != 0x0101 {
        return Err(format!("unexpected STUN message type: 0x{:04x}", msg_type));
    }
    if response[8..20] != txn_id {
        return Err("transaction ID mismatch".to_string());
    }

    // Parse attributes to find XOR-MAPPED-ADDRESS (0x0020) or MAPPED-ADDRESS (0x0001)
    let msg_len = u16::from_be_bytes([response[2], response[3]]) as usize;
    let attr_end = 20 + msg_len;
    if attr_end > n {
        return Err("STUN response truncated".to_string());
    }

    let mut offset = 20;
    while offset + 4 <= attr_end {
        let attr_type = u16::from_be_bytes([response[offset], response[offset + 1]]);
        let attr_len = u16::from_be_bytes([response[offset + 2], response[offset + 3]]) as usize;
        let attr_data_start = offset + 4;

        if attr_data_start + attr_len > attr_end {
            break;
        }

        match attr_type {
            0x0020 => {
                // XOR-MAPPED-ADDRESS
                if attr_len < 4 {
                    return Err("XOR-MAPPED-ADDRESS too short".to_string());
                }
                let family = response[attr_data_start] ^ 0x21; // XOR with first byte of magic cookie
                let xport = u16::from_be_bytes([
                    response[attr_data_start + 2],
                    response[attr_data_start + 3],
                ]) ^ 0x2112; // XOR with magic cookie top 2 bytes

                match family {
                    0x01 => {
                        // IPv4
                        if attr_len < 8 {
                            return Err("XOR-MAPPED-ADDRESS IPv4 too short".to_string());
                        }
                        let xip = u32::from_be_bytes([
                            response[attr_data_start + 4],
                            response[attr_data_start + 5],
                            response[attr_data_start + 6],
                            response[attr_data_start + 7],
                        ]) ^ 0x2112A442; // XOR with magic cookie
                        let ip = std::net::Ipv4Addr::from(xip);
                        return Ok(format!("{}:{}", ip, xport));
                    }
                    0x02 => {
                        // IPv6
                        if attr_len < 20 {
                            return Err("XOR-MAPPED-ADDRESS IPv6 too short".to_string());
                        }
                        let mut xip = [0u8; 16];
                        xip.copy_from_slice(&response[attr_data_start + 4..attr_data_start + 20]);
                        // XOR with magic cookie + transaction ID
                        let mask: [u8; 16] = [
                            0x21, 0x12, 0xA4, 0x42, // magic cookie
                            txn_id[0], txn_id[1], txn_id[2], txn_id[3], txn_id[4], txn_id[5],
                            txn_id[6], txn_id[7], txn_id[8], txn_id[9], txn_id[10], txn_id[11],
                        ];
                        let mut ip_bytes = [0u8; 16];
                        for i in 0..16 {
                            ip_bytes[i] = xip[i] ^ mask[i];
                        }
                        let ip = std::net::Ipv6Addr::from(ip_bytes);
                        return Ok(format!("[{}]:{}", ip, xport));
                    }
                    _ => return Err(format!("unknown address family: {}", family)),
                }
            }
            0x0001 => {
                // MAPPED-ADDRESS (legacy, no XOR)
                if attr_len < 4 {
                    return Err("MAPPED-ADDRESS too short".to_string());
                }
                let family = response[attr_data_start];
                let port = u16::from_be_bytes([
                    response[attr_data_start + 2],
                    response[attr_data_start + 3],
                ]);
                match family {
                    0x01 => {
                        // IPv4
                        if attr_len < 8 {
                            return Err("MAPPED-ADDRESS IPv4 too short".to_string());
                        }
                        let ip = std::net::Ipv4Addr::new(
                            response[attr_data_start + 4],
                            response[attr_data_start + 5],
                            response[attr_data_start + 6],
                            response[attr_data_start + 7],
                        );
                        return Ok(format!("{}:{}", ip, port));
                    }
                    _ => {
                        // Skip unknown families
                    }
                }
            }
            _ => {}
        }

        // Attributes are padded to 4-byte boundaries
        let padded_len = (attr_len + 3) & !3;
        offset = attr_data_start + padded_len;
    }

    Err("no MAPPED-ADDRESS or XOR-MAPPED-ADDRESS found in STUN response".to_string())
}

/// Generate a random 12-byte transaction ID for STUN requests.
fn rand_transaction_id() -> [u8; 12] {
    let mut id = [0u8; 12];
    use std::time::SystemTime;
    let t = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    // Simple pseudo-random from timestamp + counter
    let nanos = t.as_nanos() as u64;
    id[0..8].copy_from_slice(&nanos.to_le_bytes());
    // Fill remaining bytes with a simple hash
    let mut h = nanos;
    for byte in id.iter_mut().skip(8) {
        h = h.wrapping_mul(6364136223846793005).wrapping_add(1);
        *byte = (h >> 33) as u8;
    }
    id
}

/// Well-known public STUN servers for probing.
pub fn default_stun_servers() -> &'static [&'static str] {
    &[
        "stun.l.google.com:19302",
        "stun1.l.google.com:19302",
        "stun2.l.google.com:19302",
        "stun3.l.google.com:19302",
        "stun4.l.google.com:19302",
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rand_transaction_id_not_zero() {
        let id = rand_transaction_id();
        // Should not be all zeros
        assert!(id.iter().any(|&b| b != 0));
    }

    #[test]
    fn test_stun_bind_invalid_addr() {
        // Should fail gracefully with an unreachable address
        let result = stun_bind("0.0.0.0:1", Duration::from_millis(100));
        assert!(result.is_err());
    }
}
