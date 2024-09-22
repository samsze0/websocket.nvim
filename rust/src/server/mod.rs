use std::collections::HashMap;
use std::error::Error;
use std::sync::Arc;

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

pub use super::ASYNC_RUNTIME;
pub use client::WebsocketServerClient;
pub use ffi::websocket_server_ffi;
pub use inbound_event::{WebsocketServerError, WebsocketServerInboundEvent};
pub use outbound_message_replay_buffer::OutboundMessageReplayBuffer;
pub use registry::WEBSOCKET_SERVER_REGISTRY;

type WebsocketServerClients = Arc<Mutex<HashMap<Uuid, Arc<Mutex<WebsocketServerClient>>>>>;

struct WebsocketServer {
    id: Uuid,
    host: String,
    port: u32,
    clients: WebsocketServerClients,
    close_connection_event_publisher: UnboundedSender<WebsocketServerCloseConnectionEvent>,
    message_replay_buffer: Arc<Mutex<OutboundMessageReplayBuffer>>,
    outbound_broadcast_message_publisher: UnboundedSender<String>,
    inbound_event_publisher: UnboundedSender<WebsocketServerInboundEvent>,
    lua_handle: AsyncHandle,
}

async fn start_server(
    host: String,
    port: u32,
    clients: WebsocketServerClients,
    inbound_event_publisher: UnboundedSender<WebsocketServerInboundEvent>,
    lua_handle: AsyncHandle,
    mut close_connection_event_subscriber: UnboundedReceiver<WebsocketServerCloseConnectionEvent>,
    mut outbound_broadcast_message_receiver: UnboundedReceiver<String>,
    extra_response_headers: HashMap<String, String>,
    message_replay_buffer: Arc<Mutex<OutboundMessageReplayBuffer>>,
) {
    // TODO: handle error. Port might be invalid (e.g. 1000000) / in use
    let listener = TcpListener::bind(format!("{host}:{port}")).await.unwrap();

    loop {
        tokio::select! {
            Ok((stream, addr)) = listener.accept() => {
                tokio::spawn(WebsocketServerClient::run(stream, addr, inbound_event_publisher.clone(), lua_handle.clone(), extra_response_headers.clone(), message_replay_buffer.clone(), clients.clone()));
            }
            maybe_close_connection_event = close_connection_event_subscriber.recv() => {
                match maybe_close_connection_event {
                    Some(close_connection_event) => {
                        match close_connection_event {
                            WebsocketServerCloseConnectionEvent::Graceful => {
                                info!("Server received termination signal. Propagating to server clients");
                                for client in clients.lock().values() {
                                    let mut client = client.lock();
                                    client.terminate();
                                }
                                break;
                            }
                            WebsocketServerCloseConnectionEvent::Forceful => {
                                break;
                            }
                        }
                    }
                    None => ()
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
                    None => ()
                }
            }
        }
    }
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

        let (inbound_event_publisher, mut inbound_event_receiver) =
            mpsc::unbounded_channel::<WebsocketServerInboundEvent>();

        let (outbound_broadcast_message_publisher, outbound_broadcast_message_receiver) =
            mpsc::unbounded_channel::<String>();

        let (close_connection_event_publisher, close_connection_event_subscriber) =
            mpsc::unbounded_channel::<WebsocketServerCloseConnectionEvent>();

        let clients = Arc::new(Mutex::new(HashMap::new()));

        let lua_handle = AsyncHandle::new(move || {
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

        let host_clone = host.clone();
        let clients_clone = clients.clone();
        let inbound_event_publisher_clone = inbound_event_publisher.clone();
        let inbound_event_handler_clone = lua_handle.clone();

        let message_replay_buffer = Arc::new(Mutex::new(OutboundMessageReplayBuffer::new()));
        let message_replay_buffer_clone = message_replay_buffer.clone();

        let _handle = ASYNC_RUNTIME.spawn(async move {
            start_server(
                host_clone,
                port,
                clients_clone,
                inbound_event_publisher_clone,
                inbound_event_handler_clone,
                close_connection_event_subscriber,
                outbound_broadcast_message_receiver,
                extra_response_headers,
                message_replay_buffer_clone,
            )
            .await;

            info!("Server stopped");
            WEBSOCKET_SERVER_REGISTRY.lock().remove(&id);
        });

        Ok(Self {
            id,
            host,
            port,
            clients,
            close_connection_event_publisher,
            message_replay_buffer,
            outbound_broadcast_message_publisher,
            inbound_event_publisher,
            lua_handle,
        })
    }

    fn send_event(&self, event: WebsocketServerInboundEvent) {
        self.inbound_event_publisher.send(event).unwrap();
        self.lua_handle.send().unwrap();
    }

    fn terminate(&self) {
        self.close_connection_event_publisher
            .send(WebsocketServerCloseConnectionEvent::Graceful)
            .unwrap_or_else(move |err| {
                self.send_event(WebsocketServerInboundEvent::Error(
                    WebsocketServerError::ServerTerminationError(err.to_string()),
                ));
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

    fn terminate_client(&mut self, client_id: String) {
        let client_id = Uuid::parse_str(&client_id).unwrap();
        let clients = self.clients.lock();
        let client = clients.get(&client_id).unwrap();
        let mut client = client.lock();
        client.terminate();
    }

    fn broadcast_data(&mut self, data: String) {
        self.message_replay_buffer.lock().add(data.clone());
        self.outbound_broadcast_message_publisher
            .send(data)
            .unwrap_or_else(|err| {
                self.send_event(WebsocketServerInboundEvent::Error(
                    WebsocketServerError::BroadcastMessageError(err.to_string()),
                ));
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
    fn new(server_id: Uuid) -> Result<Self, Box<dyn Error>> {
        let lua = lua();
        let callbacks = lua
            .globals()
            .get::<_, LuaTable>("_WEBSOCKET_NVIM")?
            .get::<_, LuaTable>("servers")?
            .get::<_, LuaTable>("callbacks")?
            .get::<_, LuaTable>(server_id.to_string())?;

        Ok(Self {
            on_message: callbacks.get::<_, Option<LuaFunction>>("on_message")?,
            on_client_disconnect: callbacks
                .get::<_, Option<LuaFunction>>("on_client_disconnect")?,
            on_client_connect: callbacks.get::<_, Option<LuaFunction>>("on_client_connect")?,
            on_error: callbacks.get::<_, Option<LuaFunction>>("on_error")?,
        })
    }
}

#[derive(Clone, Debug)]
pub enum WebsocketServerCloseConnectionEvent {
    Graceful,
    Forceful,
}
