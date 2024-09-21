pub struct OutboundMessageReplayBuffer {
    pub(super) messages: Vec<String>,
}

impl OutboundMessageReplayBuffer {
    pub(super) fn new() -> Self {
        Self {
            messages: Vec::new(),
        }
    }

    pub(super) fn add(&mut self, message: String) {
        self.messages.push(message);
    }

    pub(super) fn replay(&self) {
        for message in &self.messages {
            print!("From Rust: {}", message);
        }
    }
}
