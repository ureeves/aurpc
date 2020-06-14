//! Asynchronous remote procedure call protocol using UDP.
#![deny(missing_docs)]
#![deny(clippy::all)]

pub use async_std;
use async_std::{
    io,
    net::{SocketAddr, ToSocketAddrs, UdpSocket},
    sync::{Arc, Mutex},
    task::{self, JoinHandle},
};
use futures::{
    channel::{mpsc, oneshot},
    sink::SinkExt,
    stream::StreamExt,
};
use std::{cmp, collections::HashMap};

type Result<T> = io::Result<T>;

const MAX_UDP_LEN: usize = 65507;

/// Message sent on the wire.
struct RpcMessage {
    flags: u8,
    rid: u16,

    len: usize,
    buf: [u8; MAX_UDP_LEN],
}

impl RpcMessage {
    /// Wraps the given buffer with flags and request id.
    ///
    /// Can fail if the buffer is longer than 65504 bytes or smaller than 3
    /// bytes.
    fn wrap(flags: u8, rid: u16, rbuf: &[u8]) -> Result<Self> {
        if rbuf.len() < 3 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "buffer too small",
            ));
        }

        if rbuf.len() > 65504 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "buffer too large",
            ));
        }

        let len = rbuf.len() + 3;
        let buf = {
            let mut buf = [0u8; MAX_UDP_LEN];
            let rid_bytes = rid.to_be_bytes();

            buf[0] = flags;
            buf[1] = rid_bytes[0];
            buf[2] = rid_bytes[1];
            buf[3..3 + rbuf.len()].copy_from_slice(rbuf);

            buf
        };

        Ok(Self {
            flags,
            rid,
            len,
            buf,
        })
    }

    /// Reads a message from the given buffer.
    ///
    /// Can fail if the buffer is longer than 65507 bytes or smaller than 3
    /// bytes.
    fn read(rbuf: &[u8]) -> Result<Self> {
        if rbuf.len() < 3 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "buffer too small",
            ));
        }

        if rbuf.len() > 65507 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "buffer too large",
            ));
        }

        let flags = rbuf[0];
        let rid = {
            let mut bytes = [0u8; 2];

            bytes[0] = rbuf[1];
            bytes[1] = rbuf[2];

            u16::from_be_bytes(bytes)
        };
        let len = rbuf.len();
        let buf = {
            let mut buf = [0u8; MAX_UDP_LEN];
            buf[..rbuf.len()].copy_from_slice(rbuf);
            buf
        };

        Ok(Self {
            flags,
            rid,
            len,
            buf,
        })
    }

    /// Get the value of the request bit. 0 if request, 1 if response.
    fn rbit(&self) -> u8 {
        self.flags & 1
    }

    /// Flips the value of the request bit. Returns the value after the flip.
    fn flip_rbit(&mut self) -> u8 {
        let flags = self.flags | 1;

        self.flags = flags;
        self.buf[0] = flags;

        self.rbit()
    }

    /// Writes data to a buffer.
    ///
    /// As much data will be written as the buffer can handle. If the buffer is
    /// too small to hold it all, the data will be truncated.
    fn write_data(&self, buf: &mut [u8]) -> usize {
        let written = cmp::min(buf.len(), self.len - 3);
        buf[..written].copy_from_slice(&self.buf[3..written + 3]);
        written
    }
}

impl AsRef<[u8]> for RpcMessage {
    fn as_ref(&self) -> &[u8] {
        &self.buf[..self.len]
    }
}

/// A RPC socket.
///
/// After creating a `RpcSocket` by [`bind`]ing it to a socket address, RPCs
/// can be [`sent to`] and [`received from`] any other socket address.
///
/// [`bind`]: #method.bind
/// [`sent to`]: #method.send_to
/// [`received from`]: #method.recv_from
pub struct RpcSocket {
    udp: Arc<UdpSocket>,
    awaiting_map: Arc<AwaitingMap>,

    _handle: JoinHandle<()>,
    receiver: Mutex<mpsc::UnboundedReceiver<(RpcMessage, SocketAddr)>>,
}

async fn rpc_loop(
    udp: Arc<UdpSocket>,
    awaiting_map: Arc<AwaitingMap>,
    mut sender: mpsc::UnboundedSender<(RpcMessage, SocketAddr)>,
) {
    let (msg_sender, mut msg_receiver) = mpsc::unbounded();
    let receiver_handle = task::spawn(receiver_loop(udp, msg_sender));

    while let Some((msg, addr)) = msg_receiver.next().await {
        if msg.rbit() == 0 {
            if sender.send((msg, addr)).await.is_err() {
                break;
            }
        } else if let Some(rsp_sender) = awaiting_map.pop(addr, msg.rid).await {
            let _ = rsp_sender.send(msg);
        }
    }

    drop(msg_receiver);
    receiver_handle.await;
}

async fn receiver_loop(
    udp: Arc<UdpSocket>,
    mut msg_sender: mpsc::UnboundedSender<(RpcMessage, SocketAddr)>,
) {
    let mut buf = [0u8; MAX_UDP_LEN];

    // TODO Handle the possibility of errors better
    while let Ok((bytes_read, addr)) = udp.recv_from(&mut buf[..]).await {
        if let Ok(msg) = RpcMessage::read(&buf[..bytes_read]) {
            if msg_sender.send((msg, addr)).await.is_err() {
                break;
            }
        }
    }
}

impl RpcSocket {
    /// Creates a RPC socket from the given address.
    ///
    /// Binding with a port number of 0 will request that the OS assigns a port
    /// to this socket. THe port allocated can be queried via the [`local_addr`]
    /// method.
    ///
    /// [`local_addr`]: #method.local_addr
    pub async fn bind<A: ToSocketAddrs>(addrs: A) -> Result<Self> {
        let udp = Arc::new(UdpSocket::bind(addrs).await?);
        let awaiting_map = Arc::new(AwaitingMap::default());

        let (sender, receiver) = mpsc::unbounded();
        let receiver = Mutex::new(receiver);

        let _handle =
            task::spawn(rpc_loop(udp.clone(), awaiting_map.clone(), sender));

        Ok(Self {
            udp,
            awaiting_map,
            receiver,
            _handle,
        })
    }

    /// Returns the local address that this listener is bound to.
    ///
    /// This can be useful, for example, when binding to port 0 to figure out
    /// which port was actually bound.
    pub fn local_addr(&self) -> Result<SocketAddr> {
        self.udp.local_addr()
    }

    /// Sends an RPC on the socket to the given address.
    ///
    /// On success, returns the number of bytes written to and read from the
    /// socket.
    pub async fn send<A: ToSocketAddrs>(
        &self,
        buf: &[u8],
        rsp_buf: &mut [u8],
        addrs: A,
    ) -> Result<(usize, usize)> {
        let addr = get_addr(addrs).await?;

        let (sender, receiver) = oneshot::channel();
        let flags = 0;
        let rid = self.awaiting_map.put(addr, sender).await;

        let msg = match RpcMessage::wrap(flags, rid, buf) {
            Ok(msg) => msg,
            Err(err) => {
                self.awaiting_map.pop(addr, rid).await;
                return Err(err);
            }
        };

        let written = match self.udp.send_to(msg.as_ref(), addr).await {
            Ok(written) => written,
            Err(err) => {
                self.awaiting_map.pop(addr, rid).await;
                return Err(err);
            }
        };

        let rsp = match receiver.await {
            Ok(rsp) => rsp,
            Err(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    "return channel canceled",
                ))
            }
        };
        let read = rsp.write_data(rsp_buf);
        Ok((written - 3, read))
    }

    /// Receives RPC from the socket.
    ///
    /// On success, returns the number of bytes read and the origin.
    pub async fn recv(&self, buf: &mut [u8]) -> Result<(usize, RpcResponder)> {
        match self.receiver.lock().await.next().await {
            Some((msg, addr)) => {
                let read = msg.write_data(buf);
                Ok((
                    read,
                    RpcResponder {
                        origin: addr,
                        udp: self.udp.clone(),
                        flags: msg.flags,
                        rid: msg.rid,
                    },
                ))
            }
            None => Err(io::Error::new(
                io::ErrorKind::Other,
                "unexpected channel close",
            )),
        }
    }

    /// Gets the value of the `IP_TTL` option for this socket.
    ///
    /// For more information about this option, see [`set_ttl`].
    ///
    /// [`set_ttl`]: #method.set_ttl
    pub fn ttl(&self) -> Result<u32> {
        self.udp.ttl()
    }

    /// Sets the value for the `IP_TTL` option on this socket.
    ///
    /// This value sets the time-to-live field that is used in every packet sent
    /// from this socket.
    pub fn set_ttl(&self, ttl: u32) -> Result<()> {
        self.udp.set_ttl(ttl)
    }
}

#[derive(Default)]
struct AwaitingMap {
    map: Mutex<MapState>,
}

#[derive(Default)]
struct MapState {
    current_rid: u16,
    map: HashMap<(SocketAddr, u16), oneshot::Sender<RpcMessage>>,
}

impl AwaitingMap {
    async fn put(
        &self,
        addr: SocketAddr,
        sender: oneshot::Sender<RpcMessage>,
    ) -> u16 {
        let mut map_state = self.map.lock().await;

        let mut rid = map_state.current_rid;
        while map_state.map.contains_key(&(addr, rid)) {
            rid += rid.wrapping_add(1);
        }
        map_state.current_rid = rid;

        map_state.map.insert((addr, rid), sender);
        rid
    }

    async fn pop(
        &self,
        addr: SocketAddr,
        rid: u16,
    ) -> Option<oneshot::Sender<RpcMessage>> {
        let mut map_state = self.map.lock().await;
        map_state.map.remove(&(addr, rid))
    }
}

/// Allows [`respond`]ing to RPCs.
///
/// [`respond`]: struct.RpcResponder.html#method.respond
pub struct RpcResponder {
    origin: SocketAddr,
    udp: Arc<UdpSocket>,

    flags: u8,
    rid: u16,
}

impl RpcResponder {
    /// The endpoint the RPC originates from.
    pub fn origin(&self) -> &SocketAddr {
        &self.origin
    }

    /// Responds to the received RPC and consumes the responder.
    ///
    /// On success, returns the number of bytes written.
    pub async fn respond(self, buf: &[u8]) -> Result<usize> {
        let mut msg = RpcMessage::wrap(self.flags, self.rid, buf)?;
        msg.flip_rbit();

        let written = self.udp.send_to(msg.as_ref(), self.origin).await?;
        Ok(written - 3)
    }
}

async fn get_addr<A: ToSocketAddrs>(addrs: A) -> Result<SocketAddr> {
    match addrs.to_socket_addrs().await?.next() {
        Some(addr) => Ok(addr),
        None => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "no addresses to send data to",
        )),
    }
}
