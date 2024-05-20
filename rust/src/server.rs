use std::collections::HashMap;
use std::error::Error;
use std::net::SocketAddr;
use std::sync::Arc;
use std::thread;

use futures_util::{SinkExt, StreamExt};
use lazy_static::lazy_static;
use mlua::prelude::*;
use nvim_oxi::conversion::ToObject;
use nvim_oxi::libuv::AsyncHandle;
use nvim_oxi::{mlua::lua, schedule, Dictionary, Function, Object};
use parking_lot::Mutex;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use uuid::Uuid;

use log::{self, error, info};
use tokio_tungstenite::tungstenite::{self};

lazy_static! {
    static ref WEBSOCKET_SERVER_REGISTRY: Mutex<WebsocketServerRegistry> =
        Mutex::new(WebsocketServerRegistry::new());
}

pub fn websocket_server_ffi() -> Dictionary {
    Dictionary::from_iter([
        (
            "start",
            Object::from(Function::from_fn(create_and_start_server)),
        ),
        ("terminate", Object::from(Function::from_fn(terminate))),
        (
            "send_data_to_client",
            Object::from(Function::from_fn(send_data_to_client)),
        ),
        ("is_active", Object::from(Function::from_fn(is_active))),
        (
            "check_replay_messages",
            Object::from(Function::from_fn(check_replay_messages)),
        ),
        (
            "broadcast_data",
            Object::from(Function::from_fn(broadcast_data)),
        ),
        (
            "terminate_client",
            Object::from(Function::from_fn(terminate_client)),
        ),
        (
            "is_client_active",
            Object::from(Function::from_fn(is_client_active)),
        ),
    ])
}

fn create_and_start_server(
    (server_id, host, port, extra_response_headers): (String, String, u32, HashMap<String, String>),
) -> nvim_oxi::Result<()> {
    let mut registry = WEBSOCKET_SERVER_REGISTRY.lock();
    let server = WebsocketServer::new(server_id, host, port, extra_response_headers).unwrap();
    registry.insert(server);
    Ok(())
}

fn terminate(server_id: String) -> nvim_oxi::Result<()> {
    let mut registry = WEBSOCKET_SERVER_REGISTRY.lock();
    let server_id = Uuid::parse_str(&server_id).unwrap();
    let server = registry.remove(&server_id).unwrap();
    server.terminate();
    Ok(())
}

fn send_data_to_client(
    (server_id, client_id, data): (String, String, String),
) -> nvim_oxi::Result<()> {
    let mut registry = WEBSOCKET_SERVER_REGISTRY.lock();
    let server_id = Uuid::parse_str(&server_id).unwrap();
    let server = registry.get_mut(&server_id).unwrap();
    server.send_data_to_client(client_id, data);
    Ok(())
}

fn is_active(server_id: String) -> nvim_oxi::Result<bool> {
    let registry = WEBSOCKET_SERVER_REGISTRY.lock();
    let server_id = Uuid::parse_str(&server_id).unwrap();
    let server = registry.get(&server_id).unwrap();
    Ok(server.is_active())
}

fn check_replay_messages(server_id: String) -> nvim_oxi::Result<Vec<String>> {
    let registry = WEBSOCKET_SERVER_REGISTRY.lock();
    let server_id = Uuid::parse_str(&server_id).unwrap();
    let server = registry.get(&server_id).unwrap();
    let replay_buffer = server.message_replay_buffer.lock();
    Ok(replay_buffer.messages.clone())
}

fn broadcast_data((server_id, data): (String, String)) -> nvim_oxi::Result<()> {
    let mut registry = WEBSOCKET_SERVER_REGISTRY.lock();
    let server_id = Uuid::parse_str(&server_id).unwrap();
    let server = registry.get_mut(&server_id).unwrap();
    server.broadcast_data(data);
    Ok(())
}

fn terminate_client((server_id, client_id): (String, String)) -> nvim_oxi::Result<()> {
    let mut registry = WEBSOCKET_SERVER_REGISTRY.lock();
    let server_id = Uuid::parse_str(&server_id).unwrap();
    let server = registry.get_mut(&server_id).unwrap();
    server.terminate_client(client_id);
    Ok(())
}

fn is_client_active((server_id, client_id): (String, String)) -> nvim_oxi::Result<bool> {
    let registry = WEBSOCKET_SERVER_REGISTRY.lock();
    let server_id = Uuid::parse_str(&server_id).unwrap();
    let server = registry.get(&server_id).unwrap();
    Ok(server.is_client_active(client_id))
}

#[derive(Clone)]
enum WebsocketServerError {
    ClientConnectionError(String),
    ClientTerminationError(Uuid, String),
    ServerTerminationError(String),
    ReceiveMessageError(Uuid, String),
    SendMessageError(Uuid, String),
    BroadcastMessageError(String),
}

#[derive(Clone)]
enum WebsocketServerInboundEvent {
    ClientConnected(Uuid),
    ClientDisconnected(Uuid),
    NewMessage(Uuid, String),
    Error(WebsocketServerError),
}

// Not necessary (for now)
impl ToObject for WebsocketServerError {
    fn to_object(self) -> Result<Object, nvim_oxi::conversion::Error> {
        match self {
            WebsocketServerError::ClientConnectionError(message) => Ok(Object::from(message)),
            WebsocketServerError::ClientTerminationError(client_id, message) => {
                Ok(Object::from(message))
            }
            WebsocketServerError::ReceiveMessageError(client_id, message) => {
                Ok(Object::from(message))
            }
            WebsocketServerError::SendMessageError(client_id, message) => Ok(Object::from(message)),
            WebsocketServerError::BroadcastMessageError(message) => Ok(Object::from(message)),
            WebsocketServerError::ServerTerminationError(message) => Ok(Object::from(message)),
        }
    }
}

impl<'lua> IntoLua<'lua> for WebsocketServerError {
    fn into_lua(self, lua: &'lua Lua) -> LuaResult<LuaValue<'lua>> {
        let vec = match self {
            WebsocketServerError::ClientConnectionError(message) => {
                vec![
                    ("type", "client_connection_error"),
                    ("message", message.leak()),
                ]
            }
            WebsocketServerError::ClientTerminationError(client_id, message) => {
                vec![
                    ("type", "client_temrination_error"),
                    ("client_id", client_id.to_string().leak()),
                    ("message", message.leak()),
                ]
            }
            WebsocketServerError::ReceiveMessageError(client_id, message) => {
                vec![
                    ("type", "receive_message_error"),
                    ("client_id", client_id.to_string().leak()),
                    ("message", message.leak()),
                ]
            }
            WebsocketServerError::SendMessageError(client_id, message) => {
                vec![
                    ("type", "send_message_error"),
                    ("client_id", client_id.to_string().leak()),
                    ("message", message.leak()),
                ]
            }
            WebsocketServerError::BroadcastMessageError(message) => {
                vec![
                    ("type", "broadcast_message_error"),
                    ("message", message.leak()),
                ]
            }
            WebsocketServerError::ServerTerminationError(message) => {
                vec![
                    ("type", "server_termination_error"),
                    ("message", message.leak()),
                ]
            }
        };
        Ok(LuaValue::Table(lua.create_table_from(vec)?))
    }
}

struct WebsocketServer {
    id: Uuid,
    host: String,
    port: u32,
    clients: Arc<Mutex<HashMap<Uuid, Arc<Mutex<WebsocketServerClient>>>>>,
    running: bool,
    running_publisher: UnboundedSender<bool>,
    message_replay_buffer: Arc<Mutex<OutboundMessageReplayBuffer>>,
    outbound_broadcast_message_publisher: UnboundedSender<String>,
    inbound_event_publisher: UnboundedSender<WebsocketServerInboundEvent>,
    inbound_event_handler: AsyncHandle,
}

impl WebsocketServer {
    fn new(
        server_id: String,
        host: String,
        port: u32,
        extra_response_headers: HashMap<String, String>,
    ) -> Result<Self, Box<dyn Error>> {
        let id = Uuid::parse_str(&server_id)?;

        let callbacks = WebsocketServerCallbacks::new(id)?;

        let mut running = false;

        let (inbound_event_publisher, mut inbound_event_receiver) =
            mpsc::unbounded_channel::<WebsocketServerInboundEvent>();

        let (outbound_broadcast_message_publisher, outbound_broadcast_message_receiver) =
            mpsc::unbounded_channel::<String>();

        let (running_publisher, running_subscriber) = mpsc::unbounded_channel::<bool>();

        let clients = Arc::new(Mutex::new(HashMap::new()));

        let inbound_event_handler = AsyncHandle::new(move || {
            let event = inbound_event_receiver.blocking_recv().unwrap();
            let callbacks_clone = callbacks.clone();
            schedule(move |_| {
                match event {
                    WebsocketServerInboundEvent::ClientConnected(client_id) => {
                        if let Some(on_connect) = callbacks_clone.on_client_connect {
                            on_connect.call::<_, ()>((id.to_string(), client_id.to_string()))?;
                        }
                    }
                    WebsocketServerInboundEvent::ClientDisconnected(client_id) => {
                        if let Some(on_disconnect) = callbacks_clone.on_client_disconnect {
                            on_disconnect.call::<_, ()>((id.to_string(), client_id.to_string()))?;
                        }
                    }
                    WebsocketServerInboundEvent::NewMessage(client_id, message) => {
                        if let Some(on_message) = callbacks_clone.on_message {
                            on_message.call::<_, ()>((
                                id.to_string(),
                                client_id.to_string(),
                                message,
                            ))?;
                        }
                    }
                    WebsocketServerInboundEvent::Error(error) => {
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
        async fn start_websocket_server(
            host: String,
            port: u32,
            clients: Arc<Mutex<HashMap<Uuid, Arc<Mutex<WebsocketServerClient>>>>>,
            inbound_event_publisher: UnboundedSender<WebsocketServerInboundEvent>,
            inbound_event_handler: AsyncHandle,
            mut running_subscriber: UnboundedReceiver<bool>,
            mut outbound_broadcast_message_receiver: UnboundedReceiver<String>,
            extra_response_headers: HashMap<String, String>,
            message_replay_buffer: Arc<Mutex<OutboundMessageReplayBuffer>>,
        ) {
            let listener = TcpListener::bind(format!("{host}:{port}")).await.unwrap();

            loop {
                tokio::select! {
                    Ok((stream, addr)) = listener.accept() => {
                        tokio::spawn(WebsocketServerClient::run(stream, addr, inbound_event_publisher.clone(), inbound_event_handler.clone(), extra_response_headers.clone(), message_replay_buffer.clone(), clients.clone()));
                    }
                    maybe_running = running_subscriber.recv() => {
                        match maybe_running {
                            Some(running) => {
                                if !running {
                                    info!("Server received termination signal. Propagating to server clients");
                                    for client in clients.lock().values() {
                                        let mut client = client.lock();
                                        client.terminate();
                                    }
                                    break;
                                }
                            }
                            None => {
                                panic!("Server running subscriber channel closed unexpecetedly");
                            }
                        }
                    }
                    maybe_message = outbound_broadcast_message_receiver.recv() => {
                        match maybe_message {
                            Some(message) => {
                                info!("Server broadcasting message: {}", message);
                                for client in clients.lock().values() {
                                    let mut client = client.lock();
                                    client.send_data(message.clone());
                                }
                            }
                            None => {
                                panic!("Server broadcast message receiver channel closed unexpecetedly");
                            }
                        }
                    }
                }
            }
        }

        let host_clone = host.clone();
        let clients_clone = clients.clone();
        let inbound_event_publisher_clone = inbound_event_publisher.clone();
        let inbound_event_handler_clone = inbound_event_handler.clone();

        let message_replay_buffer = Arc::new(Mutex::new(OutboundMessageReplayBuffer::new()));
        let message_replay_buffer_clone = message_replay_buffer.clone();

        running = true;
        running_publisher.send(true).unwrap();
        let handle = thread::spawn(move || {
            start_websocket_server(
                host_clone,
                port,
                clients_clone,
                inbound_event_publisher_clone,
                inbound_event_handler_clone,
                running_subscriber,
                outbound_broadcast_message_receiver,
                extra_response_headers,
                message_replay_buffer_clone,
            )
        });

        Ok(Self {
            id,
            host,
            port,
            clients,
            running,
            running_publisher,
            message_replay_buffer,
            outbound_broadcast_message_publisher,
            inbound_event_publisher,
            inbound_event_handler,
        })
    }

    fn terminate(&self) {
        self.running_publisher
            .send(false)
            .unwrap_or_else(move |err| {
                self.inbound_event_publisher
                    .send(WebsocketServerInboundEvent::Error(
                        WebsocketServerError::ServerTerminationError(err.to_string()),
                    ))
                    .unwrap();
                self.inbound_event_handler.send().unwrap();
                ()
            });
    }

    fn send_data_to_client(&mut self, client_id: String, data: String) {
        let client_id = Uuid::parse_str(&client_id).unwrap();
        let clients = self.clients.lock();
        let client = clients.get(&client_id).unwrap();
        let client = client.lock();
        client.send_data(data);
    }

    fn is_client_active(&self, client_id: String) -> bool {
        let client_id = Uuid::parse_str(&client_id).unwrap();
        let clients = self.clients.lock();
        let client = clients.get(&client_id).unwrap();
        let client = client.lock();
        client.is_active()
    }

    fn terminate_client(&mut self, client_id: String) {
        let client_id = Uuid::parse_str(&client_id).unwrap();
        let clients = self.clients.lock();
        let client = clients.get(&client_id).unwrap();
        let mut client = client.lock();
        client.terminate();
    }

    fn is_active(&self) -> bool {
        self.running
    }

    fn broadcast_data(&mut self, data: String) {
        self.message_replay_buffer.lock().add(data.clone());
        self.outbound_broadcast_message_publisher
            .send(data)
            .unwrap_or_else(|err| {
                self.inbound_event_publisher
                    .send(WebsocketServerInboundEvent::Error(
                        WebsocketServerError::BroadcastMessageError(err.to_string()),
                    ))
                    .unwrap();
                self.inbound_event_handler.send().unwrap();
                ()
            });
    }
}

struct WebsocketServerClient {
    id: Uuid,
    addr: SocketAddr,
    running: bool,
    running_publisher: UnboundedSender<bool>,
    outbound_message_publisher: UnboundedSender<String>,
    inbound_event_publisher: UnboundedSender<WebsocketServerInboundEvent>,
    inbound_event_handler: AsyncHandle,
}

impl WebsocketServerClient {
    async fn run(
        stream: TcpStream,
        addr: SocketAddr,
        inbound_event_publisher: UnboundedSender<WebsocketServerInboundEvent>,
        inbound_event_handler: AsyncHandle,
        extra_response_headers: HashMap<String, String>,
        message_replay_buffer: Arc<Mutex<OutboundMessageReplayBuffer>>,
        clients: Arc<Mutex<HashMap<Uuid, Arc<Mutex<WebsocketServerClient>>>>>,
    ) -> () {
        let id = Uuid::new_v4();
        let (running_publisher, mut running_subscriber) = mpsc::unbounded_channel::<bool>();
        let (outbound_message_publisher, mut outbound_message_subscriber) =
            mpsc::unbounded_channel::<String>();

        // https://github.com/snapview/tokio-tungstenite/blob/master/examples/server-headers.rs
        let ws_stream = tokio_tungstenite::accept_hdr_async(
            stream,
            |request: &tungstenite::handshake::server::Request,
             mut response: tungstenite::handshake::server::Response| {
                info!(
                    "Received a new ws handshake from path: {} with request headers {:?}",
                    request.uri().path(),
                    request.headers()
                );

                // Expose ways to check if client connection is allowed. Something like a predicate that
                // returns the list of extra headers to add if the connection is allowed.

                let headers = response.headers_mut();
                for (key, value) in extra_response_headers {
                    let key_static_ref: &'static str = key.leak();
                    headers.append(key_static_ref, value.parse().unwrap());
                }

                Ok(response)
            },
        )
        .await
        .unwrap();

        inbound_event_publisher
            .send(WebsocketServerInboundEvent::ClientConnected(id))
            .unwrap();
        inbound_event_handler.send().unwrap();

        // Replay all messages to the new client (append to "queue")
        for message in message_replay_buffer.lock().messages.clone() {
            outbound_message_publisher.send(message).unwrap();
        }

        let inbound_event_publisher_clone = inbound_event_publisher.clone();
        let inbound_event_handler_clone = inbound_event_handler.clone();

        let client = Self {
            id,
            addr,
            running: true,
            running_publisher,
            outbound_message_publisher,
            inbound_event_publisher: inbound_event_publisher_clone,
            inbound_event_handler: inbound_event_handler_clone,
        };

        clients.lock().insert(id, Arc::new(Mutex::new(client)));

        let (mut ws_sender, mut ws_receiver) = ws_stream.split();

        loop {
            tokio::select! {
                maybe_item = ws_receiver.next() => {
                    match maybe_item {
                        Some(item) => {
                            match item {
                                Ok(message) => {
                                    match message {
                                        tungstenite::Message::Text(data) => {
                                            info!("Server-client {} received message: {}", id, data);
                                            let event = WebsocketServerInboundEvent::NewMessage(id, data);
                                            inbound_event_publisher.send(event).unwrap();
                                            inbound_event_handler.send().unwrap();
                                        }
                                        tungstenite::Message::Binary(data) => {
                                            info!("Server-client {} received binary data", id);
                                            inbound_event_publisher.send(WebsocketServerInboundEvent::Error(WebsocketServerError::ReceiveMessageError(id, "Binary data handling is not supported".to_string()))).unwrap();
                                            inbound_event_handler.send().unwrap();
                                        }
                                        tungstenite::Message::Frame(frame) => {
                                            info!("Server-client {} received raw frame data", id);
                                            inbound_event_publisher.send(WebsocketServerInboundEvent::Error(WebsocketServerError::ReceiveMessageError(id, "Raw frame data handling is not supported".to_string()))).unwrap();
                                            inbound_event_handler.send().unwrap();
                                        }
                                        tungstenite::Message::Ping(_) => {
                                            info!("Server-client {} received ping", id);
                                            inbound_event_publisher.send(WebsocketServerInboundEvent::Error(WebsocketServerError::ReceiveMessageError(id, "Ping handling is not supported".to_string()))).unwrap();
                                            inbound_event_handler.send().unwrap();
                                        }
                                        tungstenite::Message::Pong(_) => {
                                            info!("Server-client {} received pong", id);
                                            inbound_event_publisher.send(WebsocketServerInboundEvent::Error(WebsocketServerError::ReceiveMessageError(id, "Pong handling is not supported".to_string()))).unwrap();
                                            inbound_event_handler.send().unwrap();
                                        }
                                        tungstenite::Message::Close(_) => {
                                            info!("Server-client {} received close", id);
                                            break;
                                        }
                                    }
                                },
                                Err(err) => {
                                    error!("Server-client {} received error: {}", id, err);
                                    inbound_event_publisher
                                        .send(WebsocketServerInboundEvent::Error(WebsocketServerError::ReceiveMessageError(id, err.to_string())))
                                        .unwrap();
                                    inbound_event_handler.send().unwrap();
                                }
                            }
                        }
                        None => {
                            panic!("Server-client {} websocket receiver channel closed unexpecetedly", id);
                        },
                    }
                }
                maybe_running = running_subscriber.recv() => {
                    match maybe_running {
                        Some(running) => {
                            if !running {
                                info!("Server-client {} received termination signal", id);
                                break;
                            }
                        }
                        None => {
                            panic!("Server-client {} running subscriber channel closed unexpecetedly", id);
                        },
                    }
                }
                maybe_message = outbound_message_subscriber.recv() => {
                    match maybe_message {
                        Some(message) => {
                            info!("Server-client {} sending message: {}", id, message);
                            ws_sender.send(tungstenite::Message::Text(message)).await.unwrap();
                        }
                        None => {
                            panic!("Server-client {} message subscriber channel closed unexpecetedly", id);
                        },
                    }
                }
            }
        }

        inbound_event_publisher
            .send(WebsocketServerInboundEvent::ClientDisconnected(id))
            .unwrap();
        inbound_event_handler.send().unwrap();
    }

    fn terminate(&mut self) {
        self.running = false;
        self.running_publisher
            .send(false)
            .unwrap_or_else(move |err| {
                self.inbound_event_publisher
                    .send(WebsocketServerInboundEvent::Error(
                        WebsocketServerError::ClientTerminationError(self.id, err.to_string()),
                    ))
                    .unwrap();
                self.inbound_event_handler.send().unwrap();
                ()
            });
    }

    fn send_data(&self, data: String) {
        self.outbound_message_publisher
            .send(data)
            .unwrap_or_else(move |err| {
                self.inbound_event_publisher
                    .send(WebsocketServerInboundEvent::Error(
                        WebsocketServerError::SendMessageError(self.id, err.to_string()),
                    ))
                    .unwrap();
                self.inbound_event_handler.send().unwrap();
                ()
            });
    }

    fn is_active(&self) -> bool {
        self.running
    }
}

struct WebsocketServerRegistry {
    // Map of client IDs to servers
    servers: HashMap<Uuid, WebsocketServer>,
}

// https://users.rust-lang.org/t/defining-a-global-mutable-structure-to-be-used-across-several-threads/7872/3
impl WebsocketServerRegistry {
    fn new() -> Self {
        Self {
            servers: HashMap::new(),
        }
    }

    fn insert(&mut self, server: WebsocketServer) -> () {
        let id = server.id;
        self.servers.insert(id, server);
        ()
    }

    fn get(&self, id: &Uuid) -> Option<&WebsocketServer> {
        self.servers.get(id)
    }

    fn get_mut(&mut self, id: &Uuid) -> Option<&mut WebsocketServer> {
        self.servers.get_mut(id)
    }

    fn remove(&mut self, id: &Uuid) -> Option<WebsocketServer> {
        self.servers.remove(id)
    }
}

#[derive(Clone)]
struct WebsocketServerCallbacks<'a> {
    on_message: Option<LuaFunction<'a>>,
    on_client_disconnect: Option<LuaFunction<'a>>,
    on_client_connect: Option<LuaFunction<'a>>,
    on_error: Option<LuaFunction<'a>>,
}

impl<'a> WebsocketServerCallbacks<'a> {
    // TODO: possible performance impact by using lua() here?
    fn new(client_id: Uuid) -> Result<Self, Box<dyn Error>> {
        let lua = lua();
        let callbacks = lua
            .globals()
            .get::<_, LuaTable>("_WEBSOCKET_NVIM")?
            .get::<_, LuaTable>("servers")?
            .get::<_, LuaTable>("callbacks")?
            .get::<_, LuaTable>(client_id.to_string())?;

        Ok(Self {
            on_message: callbacks.get::<_, Option<LuaFunction>>("on_message")?,
            on_client_disconnect: callbacks
                .get::<_, Option<LuaFunction>>("on_client_disconnect")?,
            on_client_connect: callbacks.get::<_, Option<LuaFunction>>("on_client_connect")?,
            on_error: callbacks.get::<_, Option<LuaFunction>>("on_error")?,
        })
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
}
