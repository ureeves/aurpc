use std::cmp;

use async_std::{
    net::{SocketAddr, UdpSocket},
    sync::Arc,
};

use crate::{error::Error, result::Result};

const MAX_UDP_LEN: usize = 65507;

/// Header sent with each RPC message.
///
/// An RPC header consists of 3 bytes. The first byte are flags and serve to
/// signal it being a request or a response in the protocol, and the last two
/// are a 16-bit big-endian number used to identify a particular
/// request-response cycle.
pub struct RpcHeader {
    /// 8-bit bitmask flag. The least significant bit is used to signify a
    /// request when unset and a response when set.
    flags: u8,
    /// RPC request ID. Allows for tracking a request-response cycle.
    rid: u16,
}

impl RpcHeader {
    /// Creates a new header with the given request id.
    pub fn request_from_rid(rid: u16) -> Self {
        Self {
            flags: 0b00000000,
            rid,
        }
    }

    /// Checks to see if the header is from a request message.
    pub fn is_request(&self) -> bool {
        (self.flags & 0b00000001) == 0
    }

    /// Flips the bit in the flags specifying if the message is a request or a
    /// response. Returns the value after flipping.
    pub fn flip_request(&mut self) -> bool {
        self.flags |= 0b00000001;
        self.is_request()
    }
}

/// Message that goes through the wire.
pub struct RpcMessage {
    /// 3-byte message header.
    header: RpcHeader,
    /// Buffer to be transmitted. The first 3 bytes are reserved for the message
    /// header.
    buf: [u8; MAX_UDP_LEN],
    /// Amount of the buffer that's filled.
    len: usize,
}

impl RpcMessage {
    /// Consumes the message and returns the header.
    ///
    /// This is useful to allow manipulation of the header before responding to
    /// a message.
    pub fn split(self) -> RpcHeader {
        self.header
    }

    /// The request id of the message.
    pub fn request_id(&self) -> u16 {
        self.header.rid
    }

    /// Checks to see if the message is a request.
    pub fn is_request(&self) -> bool {
        self.header.is_request()
    }

    /// Sends a message through the socket.
    ///
    /// Can fail if the data buffer is longer than 65504 bytes, or due to any
    /// underlying IO error from the socket.
    pub async fn write_to_socket(
        udp: &Arc<UdpSocket>,
        addr: SocketAddr,
        header: RpcHeader,
        data: &[u8],
    ) -> Result<usize> {
        if data.len() > 65504 {
            return Err(Error::invalid_input("buffer longer than 65504 bytes"));
        }

        let len = data.len() + 3;
        let buf = {
            let mut buf = [0u8; MAX_UDP_LEN];
            let rid_bytes = header.rid.to_be_bytes();

            buf[0] = header.flags;
            buf[1] = rid_bytes[0];
            buf[2] = rid_bytes[1];
            buf[3..len].copy_from_slice(data);

            buf
        };

        Ok(udp.send_to(&buf[..len], addr).await? - 3)
    }

    /// Reads a message from the socket.
    ///
    /// Will keep trying to read while the socket doesn't error out. Only
    /// returns when the message is valid - at least three bytes in length.
    pub async fn read_from_socket(
        udp: &Arc<UdpSocket>,
    ) -> Result<(Self, SocketAddr)> {
        let mut buf = [0u8; MAX_UDP_LEN];

        loop {
            // Loop ensures a message large enough length is read - otherwise its
            // not compliant to the protocol anyway.
            let (len, addr) = udp.recv_from(&mut buf[..]).await?;
            if len < 3 {
                continue;
            }

            let header = {
                let flags = buf[0];
                let rid = {
                    let mut bytes = [0u8; 2];

                    bytes[0] = buf[1];
                    bytes[1] = buf[2];

                    u16::from_be_bytes(bytes)
                };

                RpcHeader { flags, rid }
            };

            return Ok((RpcMessage { header, buf, len }, addr));
        }
    }

    /// Writes the message data through to the buffer, ignoring the header.
    ///
    /// Returns the amount of bytes written to the buffer.
    ///
    /// If the buffer is too small, the data will be truncated.
    pub fn write_to_buffer(&self, buf: &mut [u8]) -> usize {
        let len = cmp::min(self.len - 3, buf.len());
        buf[..len].copy_from_slice(&self.buf[3..self.len]);
        len
    }
}
