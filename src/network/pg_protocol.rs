//! PostgreSQL Simple Query Protocol implementation
//!
//! M7: PostgreSQL 3.0 Simple Query Protocol handler

use rand::Rng;

/// Protocol state machine states
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProtocolState {
    Startup,
    Ready,
    Querying,
}

/// PostgreSQL protocol handler
pub struct PgProtocol {
    state: ProtocolState,
    process_id: u32,
    secret_key: u32,
    buffer: Vec<u8>,
}

impl PgProtocol {
    pub fn new() -> Self {
        let mut rng = rand::thread_rng();
        Self {
            state: ProtocolState::Startup,
            process_id: rng.gen::<u32>(),
            secret_key: rng.gen::<u32>(),
            buffer: Vec::with_capacity(8192),
        }
    }

    pub fn state(&self) -> &'static str {
        match self.state {
            ProtocolState::Startup => "Startup",
            ProtocolState::Ready => "Ready",
            ProtocolState::Querying => "Querying",
        }
    }

    pub fn process_id(&self) -> u32 {
        self.process_id
    }

    pub fn secret_key(&self) -> u32 {
        self.secret_key
    }
}

impl Default for PgProtocol {
    fn default() -> Self {
        Self::new()
    }
}

// TODO: Implement Protocol trait for PgProtocol
