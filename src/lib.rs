use std::collections::HashMap;
use std::error::Error;
use std::num::TryFromIntError;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread;
use std::time::Duration;

use futures_util::{future, pin_mut, StreamExt, TryStreamExt};
use lazy_static::lazy_static;
use mlua::prelude::*;
use nvim_oxi::conversion::{Error as ConversionError, FromObject, ToObject};
use nvim_oxi::libuv::{AsyncHandle, TimerHandle};
use nvim_oxi::serde::{Deserializer, Serializer};
use nvim_oxi::{api, lua, mlua::lua, print, schedule, Dictionary, Function, Object};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::convert::TryFrom;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::time;
use url::Url;
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

fn new_client((client_id, connect_addr): (String, String)) -> nvim_oxi::Result<()> {
    let mut registry = WEBSOCKET_CLIENT_REGISTRY.lock();
    let client = WebsocketClient::new(client_id, connect_addr).unwrap();
    registry.insert(client);
    Ok(())
}

fn connect(client_id: String) -> nvim_oxi::Result<()> {
    let client_id = Uuid::parse_str(&client_id).unwrap();

    let mut registry = WEBSOCKET_CLIENT_REGISTRY.lock();
    let client = registry.get_mut(&client_id).unwrap();
    client.connect();
    Ok(())
}

fn disconnect(client_id: String) -> nvim_oxi::Result<()> {
    let client_id = Uuid::parse_str(&client_id).unwrap();

    let mut registry = WEBSOCKET_CLIENT_REGISTRY.lock();
    let client = registry.get_mut(&client_id).unwrap();
    client.disconnect();
    Ok(())
}

fn send_data((client_id, data): (String, String)) -> nvim_oxi::Result<()> {
    let client_id = Uuid::parse_str(&client_id).unwrap();

    let mut registry = WEBSOCKET_CLIENT_REGISTRY.lock();
    let client = registry.get_mut(&client_id).unwrap();
    client.send_data(data);
    Ok(())
}

fn is_active(client_id: String) -> nvim_oxi::Result<bool> {
    let client_id = Uuid::parse_str(&client_id).unwrap();

    let registry = WEBSOCKET_CLIENT_REGISTRY.lock();
    let client = registry.get(&client_id).unwrap();
    Ok(client.is_active())
}

fn replay_messages(client_id: String) -> nvim_oxi::Result<()> {
    let client_id = Uuid::parse_str(&client_id).unwrap();

    let mut registry = WEBSOCKET_CLIENT_REGISTRY.lock();
    let client = registry.get_mut(&client_id).unwrap();
    client.replay_messages();
    Ok(())
}

fn check_replay_messages(client_id: String) -> nvim_oxi::Result<Vec<String>> {
    let client_id = Uuid::parse_str(&client_id).unwrap();

    let registry = WEBSOCKET_CLIENT_REGISTRY.lock();
    let client = registry.get(&client_id).unwrap();
    Ok(client.message_replay_buffer.messages.clone())
}

#[derive(Clone)]
enum WebsocketClientEvent {
    Connected,
    Disconnected,
    Upgraded,
    NewMessage(String),
}

trait TryFromStr {
    fn try_from_string(s: String) -> Result<Self, Box<dyn Error>>
    where
        Self: Sized;
}

impl TryFromStr for WebsocketClientEvent {
    // Blanket implementation already exists, cannot override TryFrom trait's try_from
    fn try_from_string(s: String) -> Result<Self, Box<dyn Error>> {
        let parts: Vec<&str> = s.splitn(2, ' ').collect();

        match parts.as_slice() {
            ["Connected"] => Ok(WebsocketClientEvent::Connected),
            ["Disconnected"] => Ok(WebsocketClientEvent::Disconnected),
            ["Upgraded"] => Ok(WebsocketClientEvent::Upgraded),
            ["NewMessage", message] => Ok(WebsocketClientEvent::NewMessage(message.to_string())),
            _ => Err("Invalid message".into()),
        }
    }
}

struct WebsocketClient {
    id: Uuid,
    connect_addr: Url,
    running: Arc<AtomicBool>,
    message_replay_buffer: MessageReplayBuffer,
    event_publisher: UnboundedSender<WebsocketClientEvent>,
    handle: AsyncHandle,
}

impl WebsocketClient {
    fn new(client_id: String, connect_addr: String) -> Result<Self, Box<dyn Error>> {
        let id = Uuid::parse_str(&client_id)?;

        let callbacks = WebsocketClientCallbacks::new(id)?;

        let running = Arc::new(AtomicBool::new(false));
        let running_clone = Arc::clone(&running);

        let (event_publisher, mut event_subscriber) =
            mpsc::unbounded_channel::<WebsocketClientEvent>();

        let handle = AsyncHandle::new(move || {
            let event = event_subscriber.blocking_recv().unwrap();
            if !running_clone.load(Ordering::SeqCst) {
                return Ok::<_, nvim_oxi::Error>(());
            }
            let callbacks_clone = callbacks.clone();
            schedule(move |_| {
                match event {
                    WebsocketClientEvent::Connected => {
                        if let Some(on_connect) = callbacks_clone.on_connect {
                            on_connect.call::<_, ()>(())?;
                        }
                    }
                    WebsocketClientEvent::Disconnected => {
                        if let Some(on_disconnect) = callbacks_clone.on_disconnect {
                            on_disconnect.call::<_, ()>(())?;
                        }
                    }
                    WebsocketClientEvent::Upgraded => {
                        if let Some(on_upgrade) = callbacks_clone.on_upgrade {
                            on_upgrade.call::<_, ()>(())?;
                        }
                    }
                    WebsocketClientEvent::NewMessage(message) => {
                        if let Some(on_message) = callbacks_clone.on_message {
                            on_message.call::<_, ()>(message)?;
                        }
                    }
                }
                Ok(())
            });
            Ok(())
        })?;

        Ok(Self {
            id,
            connect_addr: Url::parse(connect_addr.as_str())?,
            running,
            message_replay_buffer: MessageReplayBuffer::new(),
            event_publisher,
            handle,
        })
    }

    fn connect(&mut self) {
        self.running.store(true, Ordering::SeqCst);
        let connect_addr = self.connect_addr.clone();
        let event_publisher = self.event_publisher.clone();
        let handle = self.handle.clone();
        let running = Arc::clone(&self.running);
        let _ = thread::spawn(move || {
            start_websocket_client(connect_addr, event_publisher, handle, running)
        });
    }

    fn disconnect(&mut self) {
        self.running.store(false, Ordering::SeqCst);
    }

    fn send_data(&mut self, data: String) {
        self.message_replay_buffer.add(data);
    }

    fn is_active(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    fn replay_messages(&self) {
        self.message_replay_buffer.replay();
    }
}

#[tokio::main]
async fn start_websocket_client(
    connect_addr: Url,
    event_publisher: UnboundedSender<WebsocketClientEvent>,
    handle: AsyncHandle,
    running: Arc<AtomicBool>,
) {
    let (ws_stream, response) = tokio_tungstenite::connect_async(connect_addr)
        .await
        .expect("Failed to connect");
    println!("From Rust: WebSocket handshake completed");

    let (write_stream, read_stream) = ws_stream.split();

    read_stream
        .for_each(|message| async {
            let data = message
                .unwrap()
                .into_text()
                .expect("Message received from server is not valid string");

            let event = WebsocketClientEvent::try_from_string(data).expect("Invalid message");
            event_publisher.send(event).unwrap();
            handle.send().unwrap();
        })
        .await;
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

#[derive(Clone)]
struct WebsocketClientCallbacks<'a> {
    on_message: Option<LuaFunction<'a>>,
    on_disconnect: Option<LuaFunction<'a>>,
    on_connect: Option<LuaFunction<'a>>,
    on_upgrade: Option<LuaFunction<'a>>,
}

impl<'a> WebsocketClientCallbacks<'a> {
    // TODO: possible performance impact by using lua() here?
    fn new(client_id: Uuid) -> Result<Self, Box<dyn Error>> {
        let lua = lua();
        let data_store = lua.globals().get::<_, LuaTable>("_WEBSOCKET_NVIM")?;
        let all_callbacks = data_store.get::<_, LuaTable>("callbacks")?;
        let callbacks = all_callbacks.get::<_, LuaTable>(client_id.to_string())?;

        Ok(Self {
            on_message: callbacks.get::<_, Option<LuaFunction>>("on_message")?,
            on_disconnect: callbacks.get::<_, Option<LuaFunction>>("on_disconnect")?,
            on_connect: callbacks.get::<_, Option<LuaFunction>>("on_connect")?,
            on_upgrade: callbacks.get::<_, Option<LuaFunction>>("on_upgrade")?,
        })
    }
}
