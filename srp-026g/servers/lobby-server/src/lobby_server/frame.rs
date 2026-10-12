//! Lobby and chat frames: `u8 len` and the payload, or for 256 bytes and
//! more `00`, `u16 len` and the payload (spec R1). The client sends the
//! short form; it reads both.

use vostok::network_client::{NetworkRequest, NetworkResponse};
use vostok::network_packet::Packet;
use vostok::serde::{Deserialize, DeserializeError, Serialize};

/// One payload the client sent, its id byte first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame(pub Vec<u8>);

impl Deserialize for Frame {
    fn deserialize(buffer: &mut &[u8]) -> Result<Self, DeserializeError> {
        let (&first, rest) = buffer
            .split_first()
            .ok_or(DeserializeError::NotEnoughInput)?;
        let (len, rest) = match first {
            0 => {
                let len = rest.get(..2).ok_or(DeserializeError::NotEnoughInput)?;
                (
                    usize::from(u16::from_le_bytes([len[0], len[1]])),
                    &rest[2..],
                )
            }
            len => (usize::from(len), rest),
        };
        let payload = rest.get(..len).ok_or(DeserializeError::NotEnoughInput)?;
        *buffer = &rest[len..];
        Ok(Self(payload.to_vec()))
    }
}

impl NetworkRequest for Frame {}

/// One payload to send; the packet adds the length.
pub struct Payload(pub Vec<u8>);

impl Serialize for Payload {
    fn serialize(self, packet: &mut impl Packet) {
        packet.append_bytes(self.0);
    }
}

impl NetworkResponse for Payload {}

/// Little-endian fields off a payload; `None` once it runs out.
pub struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    pub fn new(payload: &'a [u8]) -> Self {
        Self(payload)
    }

    fn take<const N: usize>(&mut self) -> Option<[u8; N]> {
        let (bytes, rest) = self.0.split_first_chunk::<N>()?;
        self.0 = rest;
        Some(*bytes)
    }

    pub fn u8(&mut self) -> Option<u8> {
        self.take::<1>().map(|[byte]| byte)
    }

    pub fn u16(&mut self) -> Option<u16> {
        self.take().map(u16::from_le_bytes)
    }

    pub fn u32(&mut self) -> Option<u32> {
        self.take().map(u32::from_le_bytes)
    }

    /// `u8 len` and the bytes (`buffer_writer::w_string`).
    pub fn string(&mut self) -> Option<&'a [u8]> {
        let len = usize::from(self.u8()?);
        let (bytes, rest) = self.0.split_at_checked(len)?;
        self.0 = rest;
        Some(bytes)
    }

    /// The client writes `00` or `FF` (spec R4).
    pub fn bool(&mut self) -> Option<bool> {
        self.u8().map(|byte| byte != 0)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_and_long_frames() {
        let mut bytes: &[u8] = &[2, 0x30, 0x00, 0, 3, 1, 7, 8, 9];
        assert_eq!(Frame::deserialize(&mut bytes), Ok(Frame(vec![0x30, 0])));
        // A long frame cut short waits for the rest.
        let mut cut: &[u8] = &bytes[..5];
        assert_eq!(
            Frame::deserialize(&mut cut),
            Err(DeserializeError::NotEnoughInput)
        );
        let mut long: Vec<u8> = vec![0, 0, 1];
        long.extend([5; 256]);
        let mut long: &[u8] = &long;
        assert_eq!(Frame::deserialize(&mut long).unwrap().0.len(), 256);
    }

    #[test]
    fn reader_stops_at_the_end() {
        let mut reader = Reader::new(&[1, 2, 0, 0xFF]);
        assert_eq!(reader.u8(), Some(1));
        assert_eq!(reader.u16(), Some(2));
        assert_eq!(reader.bool(), Some(true));
        assert_eq!(reader.u32(), None);
        assert!(reader.is_empty());
    }
}
