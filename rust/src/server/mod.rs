use std::collections::HashMap;
use std::error::Error;
use std::sync::Arc;
use std::thread;

use mlua::prelude::{LuaFunction, LuaTable};
use nvim_oxi::libuv::AsyncHandle;
use nvim_oxi::{mlua::lua, schedule};
use parking_lot::Mutex;
use tokio::net::TcpListener;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use uuid::Uuid;

use log::info;

mod client;
mod ffi;
mod inbound_event;
mod outbound_message_replay_buffer;
mod registry;

pub use client::WebsocketServerClient;
pub use ffi::websocket_server_ffi;
pub use inbound_event::{WebsocketServerError, WebsocketServerInboundEvent};
pub use outbound_message_replay_buffer::OutboundMessageReplayBuffer;
pub use registry::WEBSOCKET_SERVER_REGISTRY;

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
                                    let client = client.lock();
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
        let _handle = thread::spawn(move || {
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
