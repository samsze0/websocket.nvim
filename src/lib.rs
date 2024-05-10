use std::collections::HashMap;
use std::num::TryFromIntError;
use std::thread;
use std::time::Duration;

use lazy_static::lazy_static;
use nvim_oxi::conversion::{Error as ConversionError, FromObject, ToObject};
use nvim_oxi::libuv::{AsyncHandle, TimerHandle};
use nvim_oxi::serde::{Deserializer, Serializer};
use nvim_oxi::{api, lua, print, schedule, Dictionary, Error, Function, Object};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::time;
use tokio_tungstenite::tungstenite::client;
use uuid::Uuid;

lazy_static! {
    static ref WEBSOCKET_CLIENT_REGISTRY: Mutex<WebsocketClientRegistry> =
        Mutex::new(WebsocketClientRegistry::new());
}

#[nvim_oxi::module]
fn websocket_nvim() -> nvim_oxi::Result<Dictionary> {
    let api = Dictionary::from_iter([
        ("new_client", Object::from(Function::from_fn(new_client))),
        ("connect", Object::from(Function::from_fn(connect))),
        ("disconnect", Object::from(Function::from_fn(disconnect))),
        ("send_data", Object::from(Function::from_fn(send_data))),
        ("is_active", Object::from(Function::from_fn(is_active))),
        (
            "replay_messages",
            Object::from(Function::from_fn(replay_messages)),
        ),
        (
            "check_replay_messages",
            Object::from(Function::from_fn(check_replay_messages)),
        ),
    ]);

    Ok(api)
}

fn new_client(_: ()) -> nvim_oxi::Result<String> {
    let mut registry = WEBSOCKET_CLIENT_REGISTRY.lock();
    let client = WebsocketClient::new();
    let client_id = client.id.clone();
    registry.insert(client);
    Ok(client_id.to_string())
}

fn connect(client_id: String) -> nvim_oxi::Result<bool> {
    let client_id = Uuid::parse_str(&client_id).unwrap();

    let mut registry = WEBSOCKET_CLIENT_REGISTRY.lock();
    if let Some(client) = registry.get_mut(&client_id) {
        client.connect();
        Ok(true)
    } else {
        Ok(false)
    }
}

fn disconnect(client_id: String) -> nvim_oxi::Result<bool> {
    let client_id = Uuid::parse_str(&client_id).unwrap();

    let mut registry = WEBSOCKET_CLIENT_REGISTRY.lock();
    if let Some(client) = registry.get_mut(&client_id) {
        client.disconnect();
        Ok(true)
    } else {
        Ok(false)
    }
}

fn send_data((client_id, data): (String, String)) -> nvim_oxi::Result<bool> {
    let client_id = Uuid::parse_str(&client_id).unwrap();

    let mut registry = WEBSOCKET_CLIENT_REGISTRY.lock();
    if let Some(client) = registry.get_mut(&client_id) {
        client.send_data(data);
        Ok(true)
    } else {
        Ok(false)
    }
}

fn is_active(client_id: String) -> nvim_oxi::Result<(bool, Option<bool>)> {
    let client_id = Uuid::parse_str(&client_id).unwrap();

    let mut registry = WEBSOCKET_CLIENT_REGISTRY.lock();
    if let Some(client) = registry.get_mut(&client_id) {
        Ok((true, Some(client.is_active())))
    } else {
        Ok((false, None))
    }
}

fn replay_messages(client_id: String) -> nvim_oxi::Result<bool> {
    let client_id = Uuid::parse_str(&client_id).unwrap();

    let mut registry = WEBSOCKET_CLIENT_REGISTRY.lock();
    if let Some(client) = registry.get_mut(&client_id) {
        client.replay_messages();
        Ok(true)
    } else {
        Ok(false)
    }
}

fn check_replay_messages(client_id: String) -> nvim_oxi::Result<(bool, Option<Vec<String>>)> {
    let client_id = Uuid::parse_str(&client_id).unwrap();

    let mut registry = WEBSOCKET_CLIENT_REGISTRY.lock();
    if let Some(client) = registry.get_mut(&client_id) {
        Ok((true, Some(client.message_replay_buffer.messages.clone())))
    } else {
        Ok((false, None))
    }
}

// frame_size?: number, on_message: (fun(client: UtilsWebsocketClient, message: string): nil), on_disconnect?: (fun(client: UtilsWebsocketClient): nil), on_connect?: (fun(client: UtilsWebsocketClient): nil), on_upgrade?: (fun(client: UtilsWebsocketClient): nil), headers?: table<string, string>

struct WebsocketClient {
    id: Uuid,
    running: bool,
    message_replay_buffer: MessageReplayBuffer,
    sender_channel: UnboundedSender<i32>,
    receiver_channel: UnboundedReceiver<i32>,
}

impl WebsocketClient {
    fn new() -> Self {
        let (sender, receiver) = mpsc::unbounded_channel::<i32>();

        Self {
            // randomly generate a u64 id
            id: Uuid::new_v4(),
            running: false,
            message_replay_buffer: MessageReplayBuffer::new(),
            sender_channel: sender,
            receiver_channel: receiver,
        }
    }

    fn connect(&mut self) {
        self.running = true;
    }

    fn disconnect(&mut self) {
        self.running = false;
    }

    fn send_data(&mut self, data: String) {
        self.message_replay_buffer.add(data);
    }

    fn is_active(&self) -> bool {
        self.running
    }

    fn replay_messages(&self) {
        self.message_replay_buffer.replay();
    }
}

struct WebsocketClientRegistry {
    // Map of client IDs to clients
    clients: HashMap<Uuid, WebsocketClient>,
}

// https://users.rust-lang.org/t/defining-a-global-mutable-structure-to-be-used-across-several-threads/7872/3
impl WebsocketClientRegistry {
    fn new() -> Self {
        Self {
            clients: HashMap::new(),
        }
    }

    fn insert(&mut self, client: WebsocketClient) -> () {
        let id = client.id;
        self.clients.insert(id, client);
        ()
    }

    fn get(&self, id: &Uuid) -> Option<&WebsocketClient> {
        self.clients.get(id)
    }

    fn get_mut(&mut self, id: &Uuid) -> Option<&mut WebsocketClient> {
        self.clients.get_mut(id)
    }

    fn remove(&mut self, id: &Uuid) -> Option<WebsocketClient> {
        self.clients.remove(id)
    }
}

struct MessageReplayBuffer {
    messages: Vec<String>,
}

impl MessageReplayBuffer {
    fn new() -> Self {
        Self {
            messages: Vec::new(),
        }
    }

    fn add(&mut self, message: String) {
        self.messages.push(message);
    }

    fn replay(&self) {
        for message in &self.messages {
            print!("From Rust: {}", message);
        }
    }
}

#[tokio::main]
async fn start_websocket_client(handle: AsyncHandle, sender: UnboundedSender<i32>) {}
