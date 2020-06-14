//! Asynchronous remote procedure call protocol using UDP.
#![deny(missing_docs)]
#![deny(clippy::all)]

mod awaiting;
mod error;
mod message;
mod result;

pub use async_std;
use async_std::{
    net::{SocketAddr, ToSocketAddrs, UdpSocket},
    sync::{Arc, Mutex},
    task::{self, JoinHandle},
};
use futures::{
    channel::{mpsc, oneshot},
    sink::SinkExt,
    stream::StreamExt,
};

use awaiting::AwaitingMap;
pub use error::Error;
use message::{RpcHeader, RpcMessage, UdpBuffer};
use result::Result;

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
        if msg.is_request() {
            if sender.send((msg, addr)).await.is_err() {
                break;
            }
        } else if let Some(rsp_sender) =
            awaiting_map.pop(addr, msg.request_id()).await
        {
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
    let mut buf = UdpBuffer::raw_udp_buffer();

    // TODO Handle the possibility of errors better
    while let Ok((bytes_read, addr)) = udp.recv_from(&mut buf[..]).await {
        if let Ok(msg) = RpcMessage::from_buffer(&buf[..bytes_read]) {
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
        Ok(self.udp.local_addr()?)
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

        let rid = self.awaiting_map.put(addr, sender).await;
        let header = RpcHeader::request_from_rid(rid);

        let msg = match RpcMessage::from_data(header, buf) {
            Ok(msg) => msg,
            Err(err) => {
                self.awaiting_map.pop(addr, rid).await;
                return Err(err);
            }
        };

        let written = match self.udp.send_to(msg.buffer_slice(), addr).await {
            Ok(written) => written,
            Err(err) => {
                self.awaiting_map.pop(addr, rid).await;
                return Err(err.into());
            }
        };

        let rsp = match receiver.await {
            Ok(rsp) => rsp,
            Err(_) => return Err(Error::other("return channel canceled")),
        };
        let read = rsp.write_data(rsp_buf);
        // TODO Strange here. Maybe use inversion of control by allowing
        // messages to call send_to themselves.
        Ok((written - 3, read))
    }

    /// Receives RPC from the socket.
    ///
    /// On success, returns the number of bytes read and the origin.
    pub async fn recv(&self, buf: &mut [u8]) -> Result<(usize, RpcResponder)> {
        match self.receiver.lock().await.next().await {
            Some((msg, addr)) => {
                let read = msg.write_data(buf);
                let (header, _) = msg.split();
                Ok((
                    read,
                    RpcResponder {
                        origin: addr,
                        udp: self.udp.clone(),
                        header,
                    },
                ))
            }
            None => Err(Error::other("unexpected channel close")),
        }
    }

    /// Gets the value of the `IP_TTL` option for this socket.
    ///
    /// For more information about this option, see [`set_ttl`].
    ///
    /// [`set_ttl`]: #method.set_ttl
    pub fn ttl(&self) -> Result<u32> {
        Ok(self.udp.ttl()?)
    }

    /// Sets the value for the `IP_TTL` option on this socket.
    ///
    /// This value sets the time-to-live field that is used in every packet sent
    /// from this socket.
    pub fn set_ttl(&self, ttl: u32) -> Result<()> {
        Ok(self.udp.set_ttl(ttl)?)
    }
}

/// Allows [`respond`]ing to RPCs.
///
/// [`respond`]: struct.RpcResponder.html#method.respond
pub struct RpcResponder {
    origin: SocketAddr,
    udp: Arc<UdpSocket>,

    header: RpcHeader,
}

impl RpcResponder {
    /// The endpoint the RPC originates from.
    pub fn origin(&self) -> &SocketAddr {
        &self.origin
    }

    /// Responds to the received RPC and consumes the responder.
    ///
    /// On success, returns the number of bytes written.
    pub async fn respond(mut self, buf: &[u8]) -> Result<usize> {
        self.header.flip_request();
        let msg = RpcMessage::from_data(self.header, buf)?;

        let written = self.udp.send_to(msg.buffer_slice(), self.origin).await?;
        // TODO Strange here. Maybe invert the control by allowing the message
        // to write itself in a send to call.
        Ok(written - 3)
    }
}

async fn get_addr<A: ToSocketAddrs>(addrs: A) -> Result<SocketAddr> {
    match addrs.to_socket_addrs().await?.next() {
        Some(addr) => Ok(addr),
        None => Err(Error::invalid_input("no addresses to send data to")),
    }
}
