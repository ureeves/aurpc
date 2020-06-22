//! Asynchronous UDP RPCs.
//!
//! Exposes a socket-like interface allowing for sending requests and awaiting a
//! response as well as listening to requests, with UDP as transport.
//!
//! This is achieved by implementing an 24-bit protocol header on top of UDP
//! containing 8-bit flags and a 16-bit request id.
//!
//! ```text
//!                     1                   2
//! 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4
//! +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//! |     Flags     |           Request Id          |
//! +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//! ```
//!
//! Since a UDP datagram can carry a maximum of 65507 data bytes. This means
//! that, with the added overhead, each message can be a maximum of `65504
//! bytes`.
//!
//! # Examples
//!
//! ```no_run
//! # fn main() -> std::io::Result<()> { async_std::task::block_on(async {
//! use aurpc::RpcSocket;
//!
//! let socket = RpcSocket::bind("127.0.0.1:8080").await?;
//! let mut buf = vec![0u8; 1024];
//!
//! loop {
//!     let (n, responder) = socket.recv_from(&mut buf).await?;
//!     responder.respond(&buf[..n]).await?;
//! }
//! # }) }
//! ```
#![deny(missing_docs)]
#![deny(clippy::all)]

mod awaiting;
mod errors;
mod message;
mod result;

#[cfg(test)]
mod tests;

use async_std::{
    net::UdpSocket,
    sync::{Arc, Mutex},
    task::{self, JoinHandle},
};
use futures::{
    channel::{mpsc, oneshot},
    future::FutureExt,
    sink::SinkExt,
    stream::StreamExt,
};
use std::{
    future::Future,
    net::{SocketAddr, ToSocketAddrs},
    ops::Drop,
    pin::Pin,
    task::{Context, Poll},
};

use awaiting::AwaitingRequestMap;
use message::{RpcHeader, RpcMessage};
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
    awaiting_map: Arc<AwaitingRequestMap>,

    _handle: JoinHandle<()>,
    receiver: Mutex<mpsc::UnboundedReceiver<(RpcMessage, SocketAddr)>>,
}

async fn rpc_loop(
    udp: Arc<UdpSocket>,
    awaiting_map: Arc<AwaitingRequestMap>,
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
    // TODO Handle the possibility of errors better
    while let Ok((msg, addr)) = RpcMessage::read_from_socket(&udp).await {
        if msg_sender.send((msg, addr)).await.is_err() {
            break;
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
        let addr = get_addr(addrs)?;
        let udp = Arc::new(UdpSocket::bind(addr).await?);
        let awaiting_map = Arc::new(AwaitingRequestMap::default());

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
    #[allow(clippy::needless_lifetimes)]
    pub async fn send_to<'a, A: ToSocketAddrs>(
        &self,
        buf: &[u8],
        rsp_buf: &'a mut [u8],
        addrs: A,
    ) -> Result<(usize, ResponseFuture<'a>)> {
        let addr = get_addr(addrs)?;

        let (sender, receiver) = oneshot::channel();

        let rid = self.awaiting_map.put(addr, sender).await;
        let header = RpcHeader::request_from_rid(rid);

        let written =
            match RpcMessage::write_to_socket(&self.udp, addr, header, buf)
                .await
            {
                Ok(written) => written,
                Err(err) => {
                    self.awaiting_map.pop(addr, rid).await;
                    return Err(err);
                }
            };

        Ok((
            written,
            ResponseFuture {
                rsp_buf,
                addr,
                rid,
                awaiting_map: self.awaiting_map.clone(),
                receiver,
            },
        ))
    }

    /// Receives RPC from the socket.
    ///
    /// On success, returns the number of bytes read and the origin.
    pub async fn recv_from(
        &self,
        buf: &mut [u8],
    ) -> Result<(usize, RpcResponder)> {
        match self.receiver.lock().await.next().await {
            Some((msg, addr)) => {
                let read = msg.write_to_buffer(buf);
                let header = msg.split();
                Ok((
                    read,
                    RpcResponder {
                        origin: addr,
                        udp: self.udp.clone(),
                        header,
                    },
                ))
            }
            None => Err(errors::other("unexpected channel close")),
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

/// Future returned by [`send_to`].
///
/// Allows for awaiting in two steps - for sending the request first, and then
/// for receiving the response.
///
/// Dropping signals disinterest in the response.
///
/// [`send_to`]: struct.RpcSocket.html#method.send_to
pub struct ResponseFuture<'a> {
    rsp_buf: &'a mut [u8],
    addr: SocketAddr,
    rid: u16,
    awaiting_map: Arc<AwaitingRequestMap>,
    receiver: oneshot::Receiver<RpcMessage>,
}

impl<'a> ResponseFuture<'a> {
    /// Address one is expecting to receive a response from.
    ///
    /// Useful for situations where one doesn't know what `ToSocketAddrs`
    /// resolves into.
    pub fn remote_addr(&self) -> SocketAddr {
        self.addr
    }
}

impl<'a> Future for ResponseFuture<'a> {
    type Output = Result<usize>;

    fn poll(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Self::Output> {
        let this = &mut *self;
        this.receiver.poll_unpin(cx).map(|res| match res {
            Ok(rsp) => {
                let read = rsp.write_to_buffer(this.rsp_buf);
                Ok(read)
            }
            Err(_) => Err(errors::other("unexpected channel cancel")),
        })
    }
}

/// Explicitly remove from map when dropped.
///
/// This allows for timeouts to be implemented outside this crate, for instance.
impl<'a> Drop for ResponseFuture<'a> {
    fn drop(&mut self) {
        task::block_on(self.awaiting_map.pop(self.addr, self.rid));
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
        let written = RpcMessage::write_to_socket(
            &self.udp,
            self.origin,
            self.header,
            buf,
        )
        .await?;
        Ok(written)
    }
}

fn get_addr<A: ToSocketAddrs>(addrs: A) -> Result<SocketAddr> {
    match addrs.to_socket_addrs()?.next() {
        Some(addr) => Ok(addr),
        None => Err(errors::invalid_input("no addresses to send data to")),
    }
}
