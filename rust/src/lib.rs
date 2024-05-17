use std::collections::HashMap;
use std::error::Error;
use std::{env, thread};

use futures_util::{SinkExt, StreamExt};
use lazy_static::lazy_static;
use mlua::prelude::*;
use nvim_oxi::conversion::ToObject;
use nvim_oxi::libuv::AsyncHandle;
use nvim_oxi::{mlua::lua, print, schedule, Dictionary, Function, Object};
use parking_lot::Mutex;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use url::Url;
use uuid::Uuid;

use log::{self, debug, info};
use log4rs::{
    append::file::FileAppender,
    config::{Appender, Config, Root},
    encode::pattern::PatternEncoder,
};
use tokio_tungstenite::tungstenite::{self};

lazy_static! {
    static ref WEBSOCKET_CLIENT_REGISTRY: Mutex<WebsocketClientRegistry> =
        Mutex::new(WebsocketClientRegistry::new());
}

#[nvim_oxi::module]
fn websocket_ffi() -> nvim_oxi::Result<Dictionary> {
    env::set_var("RUST_BACKTRACE", "1");

    let file_appender = FileAppender::builder()
        // Pattern: https://docs.rs/log4rs/*/log4rs/encode/pattern/index.html
        .encoder(Box::new(PatternEncoder::new(
            "[{l}] {d(%Y-%m-%d %H:%M:%S)} {m}\n",
        )))
        .build("/tmp/websocket-nvim.log")
        .expect("Failed to create file appender");

    let log_config = Config::builder()
        .appender(Appender::builder().build("file", Box::new(file_appender)))
        .build(
            Root::builder()
                .appender("file")
                .build(log::LevelFilter::Debug),
        )
        .expect("Failed to create log config");
    let _ = log4rs::init_config(log_config).expect("Failed to initialize logger");

    log_panics::init();

    let api = Dictionary::from_iter([
        (
            "connect",
            Object::from(Function::from_fn(create_client_and_connect)),
        ),
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

fn create_client_and_connect(
    (client_id, connect_addr, extra_headers): (String, String, HashMap<String, String>),
) -> nvim_oxi::Result<()> {
    let mut registry = WEBSOCKET_CLIENT_REGISTRY.lock();
    let client = WebsocketClient::new(client_id, connect_addr, extra_headers).unwrap();
    registry.insert(client);
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
    Ok(client.outbound_message_replay_buffer.messages.clone())
}

#[derive(Clone)]
enum WebsocketClientError {
    ConnectionError(String),
    DisconnectionError(String),
    ReceiveMessageError(String),
    SendMessageError(String),
}

#[derive(Clone)]
enum WebsocketClientInboundEvent {
    Connected,
    Disconnected,
    NewMessage(String),
    Error(WebsocketClientError),
}

// Not necessary (for now)
impl ToObject for WebsocketClientError {
    fn to_object(self) -> Result<Object, nvim_oxi::conversion::Error> {
        match self {
            WebsocketClientError::ConnectionError(message) => Ok(Object::from(message)),
            WebsocketClientError::DisconnectionError(message) => Ok(Object::from(message)),
            WebsocketClientError::ReceiveMessageError(message) => Ok(Object::from(message)),
            WebsocketClientError::SendMessageError(message) => Ok(Object::from(message)),
        }
    }
}

impl<'lua> IntoLua<'lua> for WebsocketClientError {
    fn into_lua(self, lua: &'lua Lua) -> LuaResult<LuaValue<'lua>> {
        let vec = match self {
            WebsocketClientError::ConnectionError(message) => {
                vec![("type", "connection_error"), ("message", message.leak())]
            }
            WebsocketClientError::DisconnectionError(message) => {
                vec![("type", "disconnection_error"), ("message", message.leak())]
            }
            WebsocketClientError::ReceiveMessageError(message) => {
                vec![
                    ("type", "receive_message_error"),
                    ("message", message.leak()),
                ]
            }
            WebsocketClientError::SendMessageError(message) => {
                vec![("type", "send_message_error"), ("message", message.leak())]
            }
        };
        Ok(LuaValue::Table(lua.create_table_from(vec)?))
    }
}

// Unnecessary trait. Could have just wrap the received message in an enum instead
trait TryFromStr {
    fn try_from_string(s: String) -> Result<Self, Box<dyn Error>>
    where
        Self: Sized;
}

impl TryFromStr for WebsocketClientInboundEvent {
    // Blanket implementation already exists, cannot override TryFrom trait's try_from
    fn try_from_string(s: String) -> Result<Self, Box<dyn Error>> {
        Ok(WebsocketClientInboundEvent::NewMessage(s))
    }
}

struct WebsocketClient {
    id: Uuid,
    connect_addr: Url,
    extra_headers: HashMap<String, String>,
    running: bool,
    running_publisher: UnboundedSender<bool>,
    // Currently not used. In the future we want to support appending data to the send queue with or without connection established
    outbound_message_replay_buffer: OutboundMessageReplayBuffer,
    outbound_message_publisher: UnboundedSender<String>,
    inbound_event_publisher: UnboundedSender<WebsocketClientInboundEvent>,
    inbound_event_handler: AsyncHandle,
}

impl WebsocketClient {
    fn new(
        client_id: String,
        connect_addr: String,
        extra_headers: HashMap<String, String>,
    ) -> Result<Self, Box<dyn Error>> {
        let id = Uuid::parse_str(&client_id)?;

        let connect_addr = Url::parse(connect_addr.as_str())?;

        let callbacks = WebsocketClientCallbacks::new(id)?;

        let mut running = false;

        let (inbound_event_publisher, mut inbound_event_receiver) =
            mpsc::unbounded_channel::<WebsocketClientInboundEvent>();

        let (outbound_message_publisher, outbound_message_receiver) =
            mpsc::unbounded_channel::<String>();

        let (running_publisher, running_subscriber) = mpsc::unbounded_channel::<bool>();

        let inbound_event_handler = AsyncHandle::new(move || {
            let event = inbound_event_receiver.blocking_recv().unwrap();
            let callbacks_clone = callbacks.clone();
            schedule(move |_| {
                match event {
                    WebsocketClientInboundEvent::Connected => {
                        if let Some(on_connect) = callbacks_clone.on_connect {
                            on_connect.call::<_, ()>(id.to_string())?;
                        }
                    }
                    WebsocketClientInboundEvent::Disconnected => {
                        if let Some(on_disconnect) = callbacks_clone.on_disconnect {
                            on_disconnect.call::<_, ()>(id.to_string())?;
                        }
                    }
                    WebsocketClientInboundEvent::NewMessage(message) => {
                        if let Some(on_message) = callbacks_clone.on_message {
                            on_message.call::<_, ()>((id.to_string(), message))?;
                        }
                    }
                    WebsocketClientInboundEvent::Error(error) => {
                        if let Some(on_error) = callbacks_clone.on_error {
                            on_error.call::<_, ()>((id.to_string(), error))?;
                        }
                    }
                }
                Ok(())
            });
            Ok::<_, nvim_oxi::Error>(())
        })?;

        #[tokio::main]
        async fn start_websocket_client(
            connect_addr: Url,
            extra_headers: HashMap<String, String>,
            inbound_event_publisher: UnboundedSender<WebsocketClientInboundEvent>,
            inbound_event_handler: AsyncHandle,
            mut running_subscriber: UnboundedReceiver<bool>,
            mut outbound_message_receiver: UnboundedReceiver<String>,
        ) {
            let default_port = match connect_addr.scheme() {
                "ws" => 80,
                "wss" => 443,
                _ => 80,
            };
            let mut request = tungstenite::http::Request::builder()
                .header(
                    "Host",
                    format!(
                        "{}:{}",
                        connect_addr.host_str().unwrap(),
                        connect_addr.port().unwrap_or(default_port)
                    ),
                )
                .header(
                    "Sec-WebSocket-Key",
                    tungstenite::handshake::client::generate_key(),
                )
                .header("Upgrade", "Websocket")
                .header("Connection", "Upgrade")
                .header("Sec-WebSocket-Version", 13)
                .uri(connect_addr.as_str())
                .body(())
                .map_err(|err| {
                    inbound_event_publisher
                        .send(WebsocketClientInboundEvent::Error(
                            WebsocketClientError::ConnectionError(err.to_string()),
                        ))
                        .unwrap();
                    inbound_event_handler.send().unwrap();
                    err
                })
                .unwrap();

            for (key, value) in extra_headers {
                debug!("Adding header: {}={}", key, value);

                // https://stackoverflow.com/questions/23975391/how-to-convert-a-string-into-a-static-str
                let key_static_ref: &'static str = key.leak();
                let value_static_ref: &'static str = value.leak();
                request.headers_mut().insert(
                    key_static_ref,
                    tungstenite::http::HeaderValue::from_static(value_static_ref),
                );
            }

            let (ws_stream, response) = tokio_tungstenite::connect_async(request)
                .await
                .map_err(|err| {
                    inbound_event_publisher
                        .send(WebsocketClientInboundEvent::Error(
                            WebsocketClientError::ConnectionError(err.to_string()),
                        ))
                        .unwrap();
                    inbound_event_handler.send().unwrap();
                    err
                })
                .unwrap();
            info!("{} WebSocket handshake completed", connect_addr.as_str());
            inbound_event_publisher
                .send(WebsocketClientInboundEvent::Connected)
                .unwrap();
            inbound_event_handler.send().unwrap();

            let (mut ws_sender, mut ws_receiver) = ws_stream.split();

            loop {
                tokio::select! {
                    message = ws_receiver.next() => {
                        match message {
                            Some(message) => {
                                let message = message.unwrap();
                                if message.is_text() {
                                    let data = message.into_text().expect("Message received from server is not valid string");
                                    info!("Received message: {}", data);
                                    let event = WebsocketClientInboundEvent::NewMessage(data);
                                    inbound_event_publisher.send(event).unwrap();
                                    inbound_event_handler.send().unwrap();
                                } else if message.is_binary() {
                                    inbound_event_publisher
                                        .send(WebsocketClientInboundEvent::Error(WebsocketClientError::ReceiveMessageError("Binary data is not supported".to_string())))
                                        .unwrap();
                                    inbound_event_handler.send().unwrap();
                                    panic!("Binary data is not supported")
                                } else if message.is_close() {
                                    break;
                                }
                            }
                            None => break,
                        }
                    }
                    running = running_subscriber.recv() => {
                        match running {
                            Some(running) => {
                                if !running {
                                    break;
                                }
                            }
                            None => (),
                        }
                    }
                    message = outbound_message_receiver.recv() => {
                        match message {
                            Some(message) => {
                                ws_sender.send(tungstenite::Message::Text(message)).await.unwrap();
                            }
                            None => break,
                        }
                    }
                }
            }

            inbound_event_publisher
                .send(WebsocketClientInboundEvent::Disconnected)
                .unwrap();
            inbound_event_handler.send().unwrap();
        }

        let connect_addr_clone = connect_addr.clone();
        let extra_headers_clone = extra_headers.clone();
        let inbound_event_publisher_clone = inbound_event_publisher.clone();
        let inbound_event_handler_clone = inbound_event_handler.clone();

        running = true;
        running_publisher.send(true).unwrap();
        let handle = thread::spawn(move || {
            start_websocket_client(
                connect_addr_clone,
                extra_headers_clone,
                inbound_event_publisher_clone,
                inbound_event_handler_clone,
                running_subscriber,
                outbound_message_receiver,
            )
        });

        Ok(Self {
            id,
            connect_addr,
            extra_headers,
            running,
            running_publisher,
            outbound_message_replay_buffer: OutboundMessageReplayBuffer::new(),
            outbound_message_publisher,
            inbound_event_publisher,
            inbound_event_handler,
        })
    }

    fn disconnect(&mut self) {
        self.running = false;
        let inbound_event_publisher = self.inbound_event_publisher.clone();
        let inbound_event_handler = self.inbound_event_handler.clone();
        self.running_publisher
            .send(false)
            .unwrap_or_else(move |err| {
                inbound_event_publisher
                    .send(WebsocketClientInboundEvent::Error(
                        WebsocketClientError::DisconnectionError(err.to_string()),
                    ))
                    .unwrap();
                inbound_event_handler.send().unwrap();
                ()
            });
    }

    fn send_data(&mut self, data: String) {
        let inbound_event_publisher = self.inbound_event_publisher.clone();
        let inbound_event_handler = self.inbound_event_handler.clone();
        self.outbound_message_publisher
            .send(data)
            .unwrap_or_else(move |err| {
                inbound_event_publisher
                    .send(WebsocketClientInboundEvent::Error(
                        WebsocketClientError::SendMessageError(err.to_string()),
                    ))
                    .unwrap();
                inbound_event_handler.send().unwrap();
                ()
            });
    }

    fn is_active(&self) -> bool {
        self.running
    }

    fn replay_messages(&self) {
        self.outbound_message_replay_buffer.replay();
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

struct OutboundMessageReplayBuffer {
    messages: Vec<String>,
}

impl OutboundMessageReplayBuffer {
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
    on_error: Option<LuaFunction<'a>>,
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
            on_error: callbacks.get::<_, Option<LuaFunction>>("on_error")?,
        })
    }
}
