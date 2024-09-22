use std::collections::HashMap;

use lazy_static::lazy_static;
use parking_lot::Mutex;
use uuid::Uuid;

use super::WebsocketClient;

pub struct WebsocketClientRegistry {
    // Map of client IDs to clients
    clients: HashMap<Uuid, WebsocketClient>,
}

lazy_static! {
    pub static ref WEBSOCKET_CLIENT_REGISTRY: Mutex<WebsocketClientRegistry> =
        Mutex::new(WebsocketClientRegistry::new());
}

// https://users.rust-lang.org/t/defining-a-global-mutable-structure-to-be-used-across-several-threads/7872/3
impl WebsocketClientRegistry {
    pub(super) fn new() -> Self {
        Self {
            clients: HashMap::new(),
        }
    }

    pub(super) fn insert(&mut self, client: WebsocketClient) -> () {
        let id = client.id;
        self.clients.insert(id, client);
        ()
    }

    pub(super) fn get(&self, id: &Uuid) -> Option<&WebsocketClient> {
        self.clients.get(id)
    }

    pub(super) fn get_mut(&mut self, id: &Uuid) -> Option<&mut WebsocketClient> {
        self.clients.get_mut(id)
    }

    pub(super) fn remove(&mut self, id: &Uuid) -> Option<WebsocketClient> {
        self.clients.remove(id)
    }

    pub(super) fn get_all(&self) -> HashMap<Uuid, &WebsocketClient> {
        self.clients.iter().map(|(id, client)| (*id, client)).collect()
    }
}
