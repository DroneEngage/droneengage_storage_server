//! The offline queue (offlineQueue.js): messages stored for a unit are
//! delivered to the comm server when the unit comes online (9009), and
//! delivered rows are purged after a day by an hourly job.

use serde_json::{json, Value};
use std::time::Duration;

use crate::db::{n, Arg};
use crate::server::Storage;
use crate::util::{js_interp, now_ms};

/// Fleet-coordination messages are meaningless after a delay and are
/// never stored for offline delivery (TASK-P4A-09).
pub const NEVER_QUEUE_MESSAGE_TYPES: [f64; 4] = [1090.0, 1091.0, 1093.0, 1094.0];

/// processQueueForUnit's result; None fields are undefined.
pub struct QueueResult {
    pub success: bool,
    pub processed: Option<f64>,
    pub failed: Option<f64>,
}

/// enqueue: stores a message for an offline unit unless its type must not
/// be queued or the unit's queue is full. (No message type enqueues today;
/// kept for parity with the JavaScript server.)
pub fn enqueue(
    p_storage: &Storage,
    p_unit: &Arg,
    p_type: f64,
    p_data: &Value,
    p_task: &Arg,
    p_priority: f64,
) -> Result<(), String> {
    let v_unit = js_interp(p_unit.as_ref());
    if NEVER_QUEUE_MESSAGE_TYPES.contains(&p_type) {
        p_storage.log.warn(&format!("Refusing to queue fleet message type {} for unit {}", p_type, v_unit));
        return Err("Message type must not be queued".into());
    }
    let v_max = crate::util::js_number(p_storage.cfg().get("queue").and_then(|q| q.get("maxQueueSize")));
    let v_res = (|| -> Result<Result<(), String>, String> {
        let v_size = p_storage.db.get_queue_size(p_unit)?;
        if v_size >= v_max {
            p_storage.log.warn(&format!("Queue full for unit {}, rejecting message", v_unit));
            return Ok(Err("Queue full".into()));
        }
        p_storage.db.add_to_queue(p_unit, &n(p_type), p_data, p_task, p_priority)?;
        p_storage.log.debug(&format!("Enqueued message for unit {}, queue size: {}", v_unit, v_size + 1.0));
        Ok(Ok(()))
    })();
    match v_res {
        Ok(r) => r,
        Err(e) => {
            p_storage.log.error(&format!("Error enqueuing message for unit {}: {}", v_unit, e));
            Err(e)
        }
    }
}

/// processQueueForUnit: delivers up to 50 pending messages to the connection.
pub fn process_queue_for_unit(p_storage: &Storage, p_unit: &Arg, p_conn: u64) -> QueueResult {
    let v_unit = js_interp(p_unit.as_ref());
    let v_res = (|| -> Result<QueueResult, String> {
        p_storage.log.info(&format!("Processing offline queue for unit {}", v_unit));
        let v_messages = p_storage.db.get_queued_messages(p_unit, 50.0)?;
        if v_messages.is_empty() {
            p_storage.log.info(&format!("No queued messages for unit {}", v_unit));
            return Ok(QueueResult { success: true, processed: Some(0.0), failed: None });
        }
        let (mut v_processed, mut v_failed) = (0.0, 0.0);
        for m in &v_messages {
            let g = |k: &str| m.get(k).cloned().unwrap_or(Value::Null);
            // deliverMessage
            p_storage.send(
                p_conn,
                &json!({
                    "type": "queued_message",
                    "messageType": g("message_type"),
                    "data": g("message_data"),
                    "taskId": g("task_id"),
                    "queueId": g("id"),
                    "timestamp": now_ms(),
                }),
            );
            match p_storage.db.mark_message_delivered(&m.get("id").cloned()) {
                Ok(_) => v_processed += 1.0,
                Err(e) => {
                    p_storage.log.error(&format!("Error delivering message {}: {}", js_interp(m.get("id")), e));
                    v_failed += 1.0;
                }
            }
        }
        p_storage.log.info(&format!("Processed {} messages for unit {}, {} failed", v_processed, v_unit, v_failed));
        p_storage.db.cleanup_delivered_messages()?;
        Ok(QueueResult { success: true, processed: Some(v_processed), failed: Some(v_failed) })
    })();
    v_res.unwrap_or_else(|e| {
        p_storage.log.error(&format!("Error processing queue for unit {}: {}", v_unit, e));
        QueueResult { success: false, processed: None, failed: None }
    })
}

/// startCleanupJob: purges delivered messages every hour.
pub fn start_cleanup_job(p_storage: std::sync::Arc<Storage>) {
    let v_storage = p_storage.clone();
    tokio::spawn(async move {
        let p_storage = v_storage;
        let mut v_tick = tokio::time::interval_at(
            tokio::time::Instant::now() + Duration::from_secs(3600),
            Duration::from_secs(3600),
        );
        loop {
            v_tick.tick().await;
            if let Err(e) = p_storage.db.cleanup_delivered_messages() {
                // an exception in a setInterval callback: printed, the job goes on
                p_storage.log.error(&e);
            }
        }
    });
    p_storage.log.info("Offline queue cleanup job started");
}
