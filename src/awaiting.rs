use std::collections::HashMap;

use async_std::{net::SocketAddr, sync::Mutex};
use futures::channel::oneshot;

use crate::message::RpcMessage;

#[derive(Default)]
pub struct AwaitingMap {
    map: Mutex<MapState>,
}

#[derive(Default)]
struct MapState {
    current_rid: u16,
    map: HashMap<(SocketAddr, u16), oneshot::Sender<RpcMessage>>,
}

impl AwaitingMap {
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

    pub async fn pop(
        &self,
        addr: SocketAddr,
        rid: u16,
    ) -> Option<oneshot::Sender<RpcMessage>> {
        let mut map_state = self.map.lock().await;
        map_state.map.remove(&(addr, rid))
    }
}
