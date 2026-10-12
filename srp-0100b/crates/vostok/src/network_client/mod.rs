mod tcp_client;
mod udp_client;

pub use tcp_client::TcpClient;
pub use udp_client::UdpClient;

use crate::serde::{Deserialize, DeserializeError, Serialize};

pub trait NetworkRequest: Deserialize {}
pub trait NetworkResponse: Serialize {}

#[derive(Debug, thiserror::Error)]
pub enum NetworkError {
    #[error("Failed to deserialize the message: {0}")]
    DeserializeError(#[from] DeserializeError),
    #[error("Socket failure: {0}")]
    NetworkError(#[from] std::io::Error),
}

impl NetworkError {
    /// Whether the peer closed or reset the connection (a normal way for a
    /// client session to end) rather than sent something unreadable.
    pub fn is_disconnect(&self) -> bool {
        use std::io::ErrorKind;
        matches!(
            self,
            Self::NetworkError(error) if matches!(
                error.kind(),
                ErrorKind::UnexpectedEof
                    | ErrorKind::ConnectionReset
                    | ErrorKind::ConnectionAborted
                    | ErrorKind::BrokenPipe
            )
        )
    }
}
