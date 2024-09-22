use std::collections::HashMap;

use lazy_static::lazy_static;
use parking_lot::Mutex;
use uuid::Uuid;

use super::WebsocketServer;

lazy_static! {
    pub static ref WEBSOCKET_SERVER_REGISTRY: Mutex<WebsocketServerRegistry> =
        Mutex::new(WebsocketServerRegistry::new());
}

pub struct WebsocketServerRegistry {
    // Map of client IDs to servers
    servers: HashMap<Uuid, WebsocketServer>,
}

// https://users.rust-lang.org/t/defining-a-global-mutable-structure-to-be-used-across-several-threads/7872/3
impl WebsocketServerRegistry {
    pub(super) fn new() -> Self {
        Self {
            servers: HashMap::new(),
        }
    }

    pub(super) fn insert(&mut self, server: WebsocketServer) -> () {
        let id = server.id;
        self.servers.insert(id, server);
        ()
    }

    pub(super) fn get(&self, id: &Uuid) -> Option<&WebsocketServer> {
        self.servers.get(id)
    }

    pub(super) fn get_mut(&mut self, id: &Uuid) -> Option<&mut WebsocketServer> {
        self.servers.get_mut(id)
    }

    pub(super) fn remove(&mut self, id: &Uuid) -> Option<WebsocketServer> {
        self.servers.remove(id)
    }

    pub(super) fn get_all(&self) -> &HashMap<Uuid, WebsocketServer> {
        &self.servers
    }
}
