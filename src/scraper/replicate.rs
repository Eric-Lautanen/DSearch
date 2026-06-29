use crate::model::ContentRecord;
use crate::proto::frame::{self, Frame};
use crate::proto::msg_type::{MsgType, ReplicateAck, ReplicatePush};
use crate::storage::Store;
use std::sync::Arc;
use tracing::{debug, info, warn};

/// Push a record to a connected peer for replication.
///
/// Serializes the record into a ReplicatePush message and sends it
/// over the QUIC stream. The remote peer will store the record in
/// its Tier 3 database and reply with a ReplicateAck.
pub async fn push_record(
    send: &mut quinn::SendStream,
    record: &ContentRecord,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let record_json = serde_json::to_string(record)
        .map_err(|e| format!("serialize record for replication: {}", e))?;

    let msg = ReplicatePush {
        record_id: record.id.clone(),
        record_json,
        source_hash: record.source_hash.clone(),
        sig: record.sig.clone(),
    };
    let frame = Frame::new(MsgType::ReplicatePush, serde_json::to_vec(&msg)?);
    frame::write_frame(send, &frame).await?;
    debug!("Sent ReplicatePush for record_id={}", record.id);
    Ok(())
}

/// Handle an incoming ReplicatePush message.
///
/// Deserializes the record and inserts it into the local store.
/// Sends a ReplicateAck back indicating whether the insert succeeded.
pub async fn handle_replicate_push(
    send: &mut quinn::SendStream,
    msg: ReplicatePush,
    store: &Arc<Store>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    debug!(
        "ReplicatePush: record_id={} from peer",
        msg.record_id
    );

    let mut record: ContentRecord = match serde_json::from_str(&msg.record_json) {
        Ok(r) => r,
        Err(e) => {
            warn!(
                "ReplicatePush: failed to deserialize record {}: {}",
                msg.record_id, e
            );
            let ack = ReplicateAck {
                record_id: msg.record_id.clone(),
                accepted: false,
                reason: format!("deserialize error: {}", e),
            };
            let ack_frame = Frame::new(MsgType::ReplicateAck, serde_json::to_vec(&ack)?);
            frame::write_frame(send, &ack_frame).await?;
            return Ok(());
        }
    };

    let accepted = match store.insert_record(&mut record) {
        Ok(_) => {
            info!(
                "ReplicatePush: record {} accepted and stored",
                record.id
            );
            true
        }
        Err(e) => {
            warn!(
                "ReplicatePush: record {} rejected: {}",
                record.id, e
            );
            false
        }
    };

    let ack = ReplicateAck {
        record_id: msg.record_id.clone(),
        accepted,
        reason: if accepted {
            String::new()
        } else {
            "insert failed".to_string()
        },
    };
    let ack_frame = Frame::new(MsgType::ReplicateAck, serde_json::to_vec(&ack)?);
    frame::write_frame(send, &ack_frame).await?;

    Ok(())
}
