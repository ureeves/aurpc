use std::collections::HashMap;

use async_std::{net::SocketAddr, sync::Mutex};
use futures::channel::oneshot;

use crate::message::RpcMessage;

/// A map for RPCs that have been sent and are awaiting for a response.
///
/// A tuple of a socket address and request id is used as a key for a
/// oneshot::Sender to send the response through.
#[derive(Default)]
pub struct AwaitingRequestMap {
    map: Mutex<MapState>,
}

#[derive(Default)]
struct MapState {
    current_rid: u16,
    map: HashMap<(SocketAddr, u16), oneshot::Sender<RpcMessage>>,
}

impl AwaitingRequestMap {
    /// Puts a record in the map representing the socket address the request was
    /// sent to and the channel through which to send a possible response
    /// through.
    ///
    /// Returns the next available request id.
    pub async fn put(
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

    /// Pops a record from the map, allowing the response to be sent through the
    /// returned channel.
    ///
    /// If there is no such record, then None will be returned.
    pub async fn pop(
        &self,
        addr: SocketAddr,
        rid: u16,
    ) -> Option<oneshot::Sender<RpcMessage>> {
        let mut map_state = self.map.lock().await;
        map_state.map.remove(&(addr, rid))
    }
}

#[cfg(test)]
mod tests {
    use crate::awaiting::AwaitingRequestMap;
    use futures::channel::oneshot;

    #[async_std::test]
    async fn put_and_pop_returns_some() {
        let map = AwaitingRequestMap::default();
        let (sender, mut receiver) = oneshot::channel();

        let socket = "0.0.0.0:12345".parse().unwrap();
        let rid = map.put(socket, sender).await;

        let _ = map.pop(socket, rid).await.expect("empty entry");
        assert!(receiver.try_recv().is_err(), "channel not cancelled");
    }

    #[async_std::test]
    async fn just_pop_returns_none() {
        let map = AwaitingRequestMap::default();

        let socket = "0.0.0.0:12345".parse().unwrap();
        let rid = 0;

        assert!(map.pop(socket, rid).await.is_none(), "received something");
    }
}
