//! Bounded DataChannel queue policy.
//!
//! The reliable control channel must not silently drop semantic messages. This
//! module provides the queue primitive used by later event publishing while
//! keeping replaceable input traffic separate.

use std::collections::VecDeque;

use gstreamer_webrtc::WebRTCDataChannel;

pub const CONTROL_HIGH_WATER_MARK: u64 = 256 * 1024;
pub const CONTROL_LOW_WATER_MARK: u64 = 64 * 1024;
pub const CONTROL_QUEUE_LIMIT: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnqueueOutcome {
    Queued,
    Backpressured,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueError {
    MessageTooLarge { bytes: usize, limit: usize },
    QueueFull { queued_bytes: usize, limit: usize },
    Send(String),
}

/// Reliable messages waiting for a channel whose buffered amount is high.
#[derive(Debug, Default)]
pub struct ReliableChannelQueue {
    pending: VecDeque<String>,
    queued_bytes: usize,
}

impl ReliableChannelQueue {
    pub fn enqueue(&mut self, message: String) -> Result<EnqueueOutcome, QueueError> {
        let bytes = message.len();
        if bytes > CONTROL_QUEUE_LIMIT {
            return Err(QueueError::MessageTooLarge {
                bytes,
                limit: CONTROL_QUEUE_LIMIT,
            });
        }
        if self.queued_bytes.saturating_add(bytes) > CONTROL_QUEUE_LIMIT {
            return Err(QueueError::QueueFull {
                queued_bytes: self.queued_bytes,
                limit: CONTROL_QUEUE_LIMIT,
            });
        }

        self.queued_bytes += bytes;
        self.pending.push_back(message);
        Ok(EnqueueOutcome::Queued)
    }

    /// Sends as much queued reliable data as the channel can currently accept.
    pub fn flush(&mut self, channel: &WebRTCDataChannel) -> Result<usize, QueueError> {
        if channel.buffered_amount() >= CONTROL_HIGH_WATER_MARK {
            return Ok(0);
        }

        let mut sent = 0;
        while let Some(message) = self.pending.front() {
            if channel.buffered_amount() >= CONTROL_HIGH_WATER_MARK {
                break;
            }

            let message = message.clone();
            channel
                .send_string_full(Some(&message))
                .map_err(|error| QueueError::Send(error.to_string()))?;
            self.pending.pop_front();
            self.queued_bytes = self.queued_bytes.saturating_sub(message.len());
            sent += 1;
        }

        Ok(sent)
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    pub fn queued_bytes(&self) -> usize {
        self.queued_bytes
    }
}

/// Replaceable input keeps only the latest unsent packet.
#[derive(Debug, Default)]
pub struct LatestInputQueue {
    pending: Option<String>,
}

impl LatestInputQueue {
    pub fn replace(&mut self, message: String) {
        self.pending = Some(message);
    }

    pub fn take(&mut self) -> Option<String> {
        self.pending.take()
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reliable_queue_preserves_order_and_byte_accounting() {
        let mut queue = ReliableChannelQueue::default();
        assert_eq!(
            queue.enqueue("first".to_owned()).unwrap(),
            EnqueueOutcome::Queued
        );
        assert_eq!(
            queue.enqueue("second".to_owned()).unwrap(),
            EnqueueOutcome::Queued
        );
        assert_eq!(queue.queued_bytes(), 11);
        assert!(!queue.is_empty());
    }

    #[test]
    fn reliable_queue_rejects_a_message_larger_than_its_bound() {
        let mut queue = ReliableChannelQueue::default();
        let error = queue
            .enqueue("x".repeat(CONTROL_QUEUE_LIMIT + 1))
            .unwrap_err();

        assert_eq!(
            error,
            QueueError::MessageTooLarge {
                bytes: CONTROL_QUEUE_LIMIT + 1,
                limit: CONTROL_QUEUE_LIMIT,
            }
        );
    }

    #[test]
    fn latest_input_replaces_stale_motion() {
        let mut queue = LatestInputQueue::default();
        queue.replace("old".to_owned());
        queue.replace("new".to_owned());
        assert_eq!(queue.take().as_deref(), Some("new"));
        assert!(queue.is_empty());
    }
}
