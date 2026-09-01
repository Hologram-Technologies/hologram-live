//! Host-neutral, bounded in-memory channels for Component contract profiles.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

pub const CHANNEL_MESSAGE_MAX_BYTES: usize = 64 * 1024;
pub const CHANNEL_MAILBOX_CAPACITY: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelError {
    MessageTooLarge,
    MailboxFull,
    StateUnavailable,
}

/// A runtime-owned FIFO work queue keyed by an exact canonical channel label.
///
/// Delivery is in-memory and at-most-once: one successful receive removes one
/// message. There is no replay, acknowledgement, or broadcast in v1.
pub struct ChannelBroker {
    message_max_bytes: usize,
    mailbox_capacity: usize,
    mailboxes: Mutex<HashMap<String, VecDeque<Vec<u8>>>>,
}

impl Default for ChannelBroker {
    fn default() -> Self {
        Self::new(CHANNEL_MESSAGE_MAX_BYTES, CHANNEL_MAILBOX_CAPACITY)
    }
}

impl ChannelBroker {
    pub fn new(message_max_bytes: usize, mailbox_capacity: usize) -> Self {
        Self {
            message_max_bytes: message_max_bytes.max(1),
            mailbox_capacity: mailbox_capacity.max(1),
            mailboxes: Mutex::new(HashMap::new()),
        }
    }

    pub fn publish(&self, channel: &str, message: Vec<u8>) -> Result<(), ChannelError> {
        if message.len() > self.message_max_bytes {
            return Err(ChannelError::MessageTooLarge);
        }
        let mut mailboxes = self
            .mailboxes
            .lock()
            .map_err(|_| ChannelError::StateUnavailable)?;
        let mailbox = mailboxes.entry(channel.to_owned()).or_default();
        if mailbox.len() >= self.mailbox_capacity {
            return Err(ChannelError::MailboxFull);
        }
        mailbox.push_back(message);
        Ok(())
    }

    pub fn try_receive(&self, channel: &str) -> Result<Option<Vec<u8>>, ChannelError> {
        let mut mailboxes = self
            .mailboxes
            .lock()
            .map_err(|_| ChannelError::StateUnavailable)?;
        let Some(mailbox) = mailboxes.get_mut(channel) else {
            return Ok(None);
        };
        let message = mailbox.pop_front();
        if mailbox.is_empty() {
            mailboxes.remove(channel);
        }
        Ok(message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_are_fifo_and_consumed_at_most_once() {
        let broker = ChannelBroker::new(8, 2);
        broker.publish("channel", b"one".to_vec()).expect("first");
        broker.publish("channel", b"two".to_vec()).expect("second");
        assert_eq!(broker.try_receive("channel"), Ok(Some(b"one".to_vec())));
        assert_eq!(broker.try_receive("channel"), Ok(Some(b"two".to_vec())));
        assert_eq!(broker.try_receive("channel"), Ok(None));
    }

    #[test]
    fn full_mailbox_rejects_without_overwriting() {
        let broker = ChannelBroker::new(8, 1);
        broker.publish("channel", b"kept".to_vec()).expect("first");
        assert_eq!(
            broker.publish("channel", b"rejected".to_vec()),
            Err(ChannelError::MailboxFull)
        );
        assert_eq!(broker.try_receive("channel"), Ok(Some(b"kept".to_vec())));
    }

    #[test]
    fn oversized_message_is_rejected_without_creating_a_mailbox() {
        let broker = ChannelBroker::new(3, 1);
        assert_eq!(
            broker.publish("channel", b"four".to_vec()),
            Err(ChannelError::MessageTooLarge)
        );
        assert_eq!(broker.try_receive("channel"), Ok(None));
    }
}
