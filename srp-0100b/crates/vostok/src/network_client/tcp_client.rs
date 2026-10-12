use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};

use crate::network_client::{NetworkError, NetworkRequest, NetworkResponse};
use crate::network_packet::TcpPacket;
use crate::serde::DeserializeError;

pub struct TcpClient {
    stream: TcpStream,
    read_buffer: Vec<u8>,
    read_buffer_len: usize,
    read_buffer_idx: usize,

    write_packet: TcpPacket,
}

impl TcpClient {
    pub fn new(stream: TcpStream) -> Self {
        Self {
            stream,
            // @NOTE: assumes that every single request can fit into 512 bytes
            read_buffer: vec![0; 512],
            read_buffer_len: 0,
            read_buffer_idx: 0,
            write_packet: TcpPacket::new(),
        }
    }

    /// The client's end of the connection.
    pub fn peer_addr(&self) -> io::Result<SocketAddr> {
        self.stream.peer_addr()
    }

    /// The server's end of the connection: the address the client reached.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.stream.local_addr()
    }

    pub fn get_read_buffer(&self) -> &[u8] {
        &self.read_buffer[self.read_buffer_idx..self.read_buffer_len]
    }

    pub fn read<T: NetworkRequest>(&mut self) -> Result<T, NetworkError> {
        let (msg, read_buffer_idx) = self.peek_impl()?;
        self.read_buffer_idx = read_buffer_idx;
        Ok(msg)
    }

    pub fn peek<T: NetworkRequest>(&mut self) -> Result<T, NetworkError> {
        self.peek_impl().map(|(msg, _)| msg)
    }

    fn peek_impl<T: NetworkRequest>(&mut self) -> Result<(T, usize), NetworkError> {
        loop {
            if self.read_buffer_len > self.read_buffer_idx {
                let read_buffer: &mut &[u8] =
                    &mut &self.read_buffer[self.read_buffer_idx..self.read_buffer_len];

                match T::deserialize(read_buffer) {
                    Ok(msg) => return Ok((msg, self.read_buffer_len - read_buffer.len())),
                    // The rest of the message is still on its way.
                    Err(DeserializeError::NotEnoughInput) => {}
                    Err(error) => return Err(error.into()),
                }
            }
            self.read_more()?;
        }
    }

    /// Keep the unread bytes and append whatever the peer sends next.
    fn read_more(&mut self) -> Result<(), NetworkError> {
        self.read_buffer
            .copy_within(self.read_buffer_idx..self.read_buffer_len, 0);
        self.read_buffer_len -= self.read_buffer_idx;
        self.read_buffer_idx = 0;
        if self.read_buffer_len == self.read_buffer.len() {
            self.read_buffer.resize(self.read_buffer.len() * 2, 0);
        }

        let bytes_read = self
            .stream
            .read(&mut self.read_buffer[self.read_buffer_len..])?;
        // A zero-length read is the peer closing the connection.
        if bytes_read == 0 {
            return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof).into());
        }
        self.read_buffer_len += bytes_read;
        Ok(())
    }

    pub fn send<T: NetworkResponse>(&mut self, message: T) -> Result<(), NetworkError> {
        message.serialize(&mut self.write_packet);
        self.stream.write_all(self.write_packet.get_buffer())?;
        self.write_packet.clear();

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serde::{Deserialize, DeserializeError};
    use std::net::TcpListener;

    struct AnyByte;

    impl Deserialize for AnyByte {
        fn deserialize(buffer: &mut &[u8]) -> Result<Self, DeserializeError> {
            *buffer = &buffer[1..];
            Ok(Self)
        }
    }

    impl NetworkRequest for AnyByte {}

    /// A `u8` length followed by that many bytes.
    struct Framed(Vec<u8>);

    impl Deserialize for Framed {
        fn deserialize(buffer: &mut &[u8]) -> Result<Self, DeserializeError> {
            let (&len, rest) = buffer
                .split_first()
                .ok_or(DeserializeError::NotEnoughInput)?;
            let body = rest
                .get(..len as usize)
                .ok_or(DeserializeError::NotEnoughInput)?;
            *buffer = &rest[len as usize..];
            Ok(Self(body.to_vec()))
        }
    }

    impl NetworkRequest for Framed {}

    #[test]
    fn a_message_split_across_reads_is_read_whole() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (stream, _) = listener.accept().unwrap();
        let mut client = TcpClient::new(stream);
        let long: Vec<u8> = (0..200).collect();

        let writer = std::thread::spawn(move || {
            peer.write_all(&[3, 1, 2, 3, 200]).unwrap();
            peer.flush().unwrap();
            std::thread::sleep(std::time::Duration::from_millis(50));
            peer.write_all(&(0..200).collect::<Vec<u8>>()).unwrap();
            peer
        });

        assert_eq!(client.read::<Framed>().unwrap().0, [1, 2, 3]);
        assert_eq!(client.read::<Framed>().unwrap().0, long);
        drop(writer.join().unwrap());
    }

    #[test]
    fn peer_close_is_reported_as_a_disconnect() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let peer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (stream, _) = listener.accept().unwrap();
        drop(peer);

        let error = TcpClient::new(stream).read::<AnyByte>().err().unwrap();
        assert!(error.is_disconnect(), "{error}");
    }
}
