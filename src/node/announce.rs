use crate::model::Announcement;
use crate::proto::frame::{self, Frame};
use crate::proto::msg_type::{Announce, AnnounceAck, MsgType};
use crate::storage::Store;
use std::sync::Arc;
use tracing::{debug, info, warn};

/// Handle an incoming Announce message.
///
/// When a peer announces that it holds a record, we:
/// 1. Validate the announcement size
/// 2. Store it in the Tier 2 announcements table
/// 3. Send an AnnounceAck back
pub async fn handle_announce(
    send: &mut quinn::SendStream,
    msg: Announce,
    store: &Arc<Store>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    debug!(
        "Announce: record_id={} holder={}",
        msg.record_id, msg.holder_addr
    );

    let mut ann = Announcement {
        record_id: msg.record_id.clone(),
        source_hash: msg.source_hash.clone(),
        schema: msg.schema.clone(),
        tags: msg.tags.clone(),
        holder_addr: msg.holder_addr.clone(),
        expires_at: msg.expires_at,
        sig: msg.sig.clone(),
    };

    let accepted = match store.insert_announcement(&mut ann) {
        Ok(()) => {
            info!(
                "Announcement accepted: record_id={} holder={}",
                msg.record_id, msg.holder_addr
            );
            true
        }
        Err(e) => {
            warn!(
                "Announcement rejected: record_id={} error={}",
                msg.record_id, e
            );
            false
        }
    };

    let ack = AnnounceAck {
        record_id: msg.record_id.clone(),
        accepted,
        reason: if accepted {
            String::new()
        } else {
            "insert failed".to_string()
        },
    };
    let ack_frame = Frame::new(MsgType::AnnounceAck, serde_json::to_vec(&ack)?);
    frame::write_frame(send, &ack_frame).await?;

    Ok(())
}

/// Send an Announce message to a peer for a locally-held record.
///
/// Used when this node wants to announce that it holds a record
/// to a connected peer.
pub async fn send_announce(
    send: &mut quinn::SendStream,
    ann: &Announcement,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let msg = Announce {
        record_id: ann.record_id.clone(),
        source_hash: ann.source_hash.clone(),
        schema: ann.schema.clone(),
        tags: ann.tags.clone(),
        holder_addr: ann.holder_addr.clone(),
        expires_at: ann.expires_at,
        sig: ann.sig.clone(),
    };
    let frame = Frame::new(MsgType::Announce, serde_json::to_vec(&msg)?);
    frame::write_frame(send, &frame).await?;
    debug!("Sent Announce for record_id={}", ann.record_id);
    Ok(())
}
