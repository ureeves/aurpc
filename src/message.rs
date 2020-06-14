use std::cmp;

use crate::{error::Error, result::Result};

const MAX_UDP_LEN: usize = 65507;

/// 8-bit bitmask flag sent in the header of each message.
struct RpcFlags(u8);

/// 16-bit identifier of a request-response cycle.
struct RpcId(u16);

/// Header sent with each RPC message.
///
/// An RPC header consists of 3 bytes. The first byte are flags and serve to
/// signal it being a request or a response in the protocol, and the last two
/// are a 16-bit big-endian number used to identify a particular
/// request-response cycle.
pub struct RpcHeader {
    /// 8-bit bitmask flag. The least significant bit is used to signify a
    /// request when unset and a response when set.
    flags: RpcFlags,
    /// RPC request ID. Allows for tracking a request-response cycle.
    rid: RpcId,
}

impl RpcHeader {
    /// Creates a new header with the given request id.
    pub fn request_from_rid(rid: u16) -> Self {
        Self {
            flags: RpcFlags(0b00000000),
            rid: RpcId(rid),
        }
    }

    /// Checks to see if the header is from a request message.
    pub fn is_request(&self) -> bool {
        (self.flags.0 & 0b00000001) == 0
    }

    /// Flips the bit in the flags specifying if the message is a request or a
    /// response. Returns the value after flipping.
    pub fn flip_request(&mut self) -> bool {
        self.flags.0 |= 0b00000001;
        self.is_request()
    }
}

/// Buffer used to hold data to be transmitted or received.
///
/// The first 3 bytes are reserved for the RPC header, and the rest of the 65504
/// can be used for data.
pub struct UdpBuffer {
    /// The amount of this buffer that has been filled.
    len: usize,
    /// Byte buffer for the wire.
    buf: [u8; MAX_UDP_LEN],
}

impl UdpBuffer {
    /// Creates a new, raw and zeroed UDP buffer. Useful for doing passing to a
    /// recv_from call on a UdpSocket.
    pub fn raw_udp_buffer() -> [u8; MAX_UDP_LEN] {
        [0u8; MAX_UDP_LEN]
    }
}

/// Message that goes through the wire.
pub struct RpcMessage {
    /// 3-byte message header.
    header: RpcHeader,
    /// Buffer to be transmitted. The first 3 bytes are reserved for the message
    /// header.
    buf: UdpBuffer,
}

impl RpcMessage {
    /// Splits the header and the buffer.
    ///
    /// This is useful to allow manipulation of the header before responding to
    /// a message.
    pub fn split(self) -> (RpcHeader, UdpBuffer) {
        (self.header, self.buf)
    }

    /// The request id of the message.
    pub fn request_id(&self) -> u16 {
        self.header.rid.0
    }

    /// Checks to see if the message is a request.
    pub fn is_request(&self) -> bool {
        self.header.is_request()
    }

    /// Creates a new message from a header and data.
    ///
    /// Can fail if the data buffer is longer than 65504 bytes.
    pub fn from_data(header: RpcHeader, data: &[u8]) -> Result<Self> {
        if data.len() > 65504 {
            return Err(Error::invalid_input("buffer longer than 65504 bytes"));
        }

        let buf = {
            let len = data.len() + 3;

            let mut buf = [0u8; MAX_UDP_LEN];
            let rid_bytes = header.rid.0.to_be_bytes();

            buf[0] = header.flags.0;
            buf[1] = rid_bytes[0];
            buf[2] = rid_bytes[1];
            buf[3..len].copy_from_slice(data);

            UdpBuffer { len, buf }
        };

        Ok(Self { header, buf })
    }

    /// Creates a new message from a buffer.
    ///
    /// Can fail if the buffer is longer than 65507 bytes or smaller than 3
    /// bytes.
    pub fn from_buffer(msg: &[u8]) -> Result<Self> {
        if msg.len() < 3 {
            return Err(Error::invalid_input("buffer smaller than 3 bytes"));
        }

        if msg.len() > 65507 {
            return Err(Error::invalid_input("buffer longer than 65504 bytes"));
        }
        let header = {
            let flags = RpcFlags(msg[0]);
            let rid = {
                let mut bytes = [0u8; 2];

                bytes[0] = msg[1];
                bytes[1] = msg[2];

                RpcId(u16::from_be_bytes(bytes))
            };

            RpcHeader { flags, rid }
        };

        let buf = {
            let mut buf = [0u8; MAX_UDP_LEN];
            let len = msg.len();

            buf[..msg.len()].copy_from_slice(msg);
            UdpBuffer { len, buf }
        };

        Ok(Self { header, buf })
    }

    /// Returns a slice over the underlying buffer.
    pub fn buffer_slice(&self) -> &[u8] {
        &self.buf.buf[..self.buf.len]
    }

    /// Writes the data of the message to the given slice. Returns the amount of
    /// bytes written to the slice.
    ///
    /// As much data will be written as the buffer can handle. If the buffer is
    /// too small to hold all data, it will be truncated.
    pub fn write_data(&self, buf: &mut [u8]) -> usize {
        let written = cmp::min(self.buf.len - 3, buf.len());
        buf[..written].copy_from_slice(&self.buf.buf[3..written + 3]);
        written
    }
}
