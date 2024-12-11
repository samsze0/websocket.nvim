use std::collections::HashMap;
use std::error::Error;
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt, TryFutureExt};
use mlua::prelude::{LuaFunction, LuaTable};
use nvim_oxi::libuv::AsyncHandle;
use nvim_oxi::{mlua::lua, schedule};
use tokio::net::TcpStream;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use url::Url;
use uuid::Uuid;

use log::{debug, error, info, warn};
use tokio_tungstenite::tungstenite::{self};

mod ffi;
mod inbound_event;
mod registry;

use inbound_event::{WebsocketClientError, WebsocketClientInboundEvent};
use registry::WEBSOCKET_CLIENT_REGISTRY;

pub use super::ASYNC_RUNTIME;
pub use ffi::websocket_client_ffi;

use tokio::task::JoinHandle;

struct WebsocketClient {
    id: Uuid,
    connect_addr: Url,
    extra_headers: HashMap<String, String>,
    close_connection_event_publisher: UnboundedSender<WebsocketClientCloseConnectionEvent>,
    close_connection_event_subscriber:
        Option<UnboundedReceiver<WebsocketClientCloseConnectionEvent>>,
    outbound_message_publisher: UnboundedSender<String>,
    outbound_message_receiver: Option<UnboundedReceiver<String>>,
    inbound_event_publisher: UnboundedSender<WebsocketClientInboundEvent>,

    lua_handle: AsyncHandle,
    task_handle: Option<JoinHandle<()>>,
}

async fn start_loop(
    ws_stream: WebSocketStream<MaybeTlsStream<TcpStream>>,
    inbound_event_publisher: &UnboundedSender<WebsocketClientInboundEvent>,
    lua_handle: &AsyncHandle,
    mut close_connection_event_subscriber: UnboundedReceiver<WebsocketClientCloseConnectionEvent>,
    mut outbound_message_receiver: UnboundedReceiver<String>,
) {
    let send_event = move |event: WebsocketClientInboundEvent| {
        inbound_event_publisher
            .send(event)
            .unwrap_or_else(|err| error!("Failed to send event: {}", err));
        lua_handle
            .send()
            .unwrap_or_else(|err| error!("Failed to send event: {}", err));
    };

    let (mut ws_sender, mut ws_receiver) = ws_stream.split();

    loop {
        tokio::select! {
            message = ws_receiver.next() => {
                match message {
                    Some(message) => {
                        match message {
                            Ok(message) => {
                                if message.is_text() {
                                    let data = message.into_text().expect("Message received from server is not valid string");
                                    info!("Received message: {}", data);
                                    send_event(WebsocketClientInboundEvent::NewMessage(data));
                                } else if message.is_binary() {
                                    send_event(WebsocketClientInboundEvent::Error(WebsocketClientError::ReceiveMessageError("Binary data is not supported".to_string())));
                                    error!("Binary data is not supported");
                                } else if message.is_close() {
                                    info!("Received close frame from server");
                                    break;
                                }
                            }
                            Err(err) => {
                                send_event(WebsocketClientInboundEvent::Error(WebsocketClientError::ReceiveMessageError(err.to_string())));
                                error!("Failed to receive message: {}", err);
                            }
                        }
                    }
                    None => (),
                }
            }
            close_event = close_connection_event_subscriber.recv() => {
                match close_event {
                    Some(close_event) => {
                        match close_event {
                            WebsocketClientCloseConnectionEvent::Graceful => {
                                if let Err(err) = ws_sender.send(tungstenite::Message::Close(None)).await {
                                    send_event(WebsocketClientInboundEvent::Error(WebsocketClientError::ConnectionError(err.to_string())));
                                    error!("Failed to send close message: {}", err);
                                }
                            }
                            WebsocketClientCloseConnectionEvent::Forceful => {
                                warn!("Forcefully closing WebSocket connection");
                                break
                            }
                        }
                    }
                    None => (),
                }
            }
            message = outbound_message_receiver.recv() => {
                match message {
                    Some(message) => {
                        if let Err(err) = ws_sender.send(tungstenite::Message::Text(message)).await {
                            send_event(WebsocketClientInboundEvent::Error(WebsocketClientError::ConnectionError(err.to_string())));
                            error!("Failed to forward message to websocket: {}", err);
                        }
                    }
                    None => (),
                }
            }
        }
    }

    info!("Closing WebSocket connection. Sending out event - \"Disconnected\"");
    send_event(WebsocketClientInboundEvent::Disconnected);
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

        let (inbound_event_publisher, mut inbound_event_receiver) =
            mpsc::unbounded_channel::<WebsocketClientInboundEvent>();

        let (outbound_message_publisher, outbound_message_receiver) =
            mpsc::unbounded_channel::<String>();

        let (close_connection_event_publisher, close_connection_event_subscriber) =
            mpsc::unbounded_channel::<WebsocketClientCloseConnectionEvent>();

        // TODO: destroy this handle when the client is destroyed
        let lua_handle = AsyncHandle::new(move || {
            let event = inbound_event_receiver.blocking_recv().unwrap();
            info!("Received event - \"{:?}\"", event);
            let callbacks = callbacks.clone();
            schedule(move |_| {
                match event {
                    WebsocketClientInboundEvent::Connected => {
                        if let Some(on_connect) = callbacks.on_connect.clone() {
                            on_connect.call::<_, ()>(id.to_string())?;
                        }
                    }
                    WebsocketClientInboundEvent::Disconnected => {
                        if let Some(on_disconnect) = callbacks.on_disconnect {
                            on_disconnect.call::<_, ()>(id.to_string())?;
                        }
                    }
                    WebsocketClientInboundEvent::NewMessage(message) => {
                        if let Some(on_message) = callbacks.on_message.clone() {
                            on_message.call::<_, ()>((id.to_string(), message))?;
                        }
                    }
                    WebsocketClientInboundEvent::Error(error) => {
                        if let Some(on_error) = callbacks.on_error.clone() {
                            on_error.call::<_, ()>((id.to_string(), error))?;
                        }
                    }
                }
                Ok(())
            });
            Ok::<_, nvim_oxi::Error>(())
        })?;

        Ok(Self {
            id,
            connect_addr,
            extra_headers,
            close_connection_event_publisher,
            outbound_message_publisher,
            inbound_event_publisher,
            lua_handle,
            task_handle: None,
            close_connection_event_subscriber: Some(close_connection_event_subscriber),
            outbound_message_receiver: Some(outbound_message_receiver),
        })
    }

    fn connect_sync(&mut self) -> Result<(), WebsocketClientError> {
        let default_port = match self.connect_addr.scheme() {
            "ws" => 80,
            "wss" => 443,
            _ => 80,
        };
        let mut request =
            tungstenite::http::Request::builder()
                .header(
                    "Host",
                    format!(
                        "{}:{}",
                        self.connect_addr.host_str().ok_or(
                            WebsocketClientError::ConnectionError("Host is not set".to_string())
                        )?,
                        self.connect_addr.port().unwrap_or_else(|| {
                            info!("Port is not set. Using default port: {}", default_port);
                            default_port
                        })
                    ),
                )
                .header(
                    "Sec-WebSocket-Key",
                    tungstenite::handshake::client::generate_key(),
                )
                .header("Upgrade", "Websocket")
                .header("Connection", "Upgrade")
                .header("Sec-WebSocket-Version", 13)
                .uri(self.connect_addr.as_str())
                .body(())
                .map_err(|err| WebsocketClientError::ConnectionError(err.to_string()))?;

        for (key, value) in self.extra_headers.clone() {
            debug!("Adding header: {}={}", key, value);

            // https://stackoverflow.com/questions/23975391/how-to-convert-a-string-into-a-static-str
            let key_static_ref: &'static str = key.leak();
            let value_static_ref: &'static str = value.leak();
            request.headers_mut().insert(
                key_static_ref,
                tungstenite::http::HeaderValue::from_static(value_static_ref),
            );
        }

        let connect_result: Result<
            WebSocketStream<MaybeTlsStream<TcpStream>>,
            WebsocketClientError,
        > = ASYNC_RUNTIME.block_on(async move {
            let (ws_stream, _response) = tokio_tungstenite::connect_async(request)
                .await
                .map_err(|err| WebsocketClientError::ConnectionError(err.to_string()))?;

            Ok(ws_stream)
        });

        match connect_result {
            Ok(ws_stream) => {
                info!(
                    "{} WebSocket handshake completed",
                    self.connect_addr.as_str()
                );
                self.send_event(WebsocketClientInboundEvent::Connected);

                let inbound_event_publisher = self.inbound_event_publisher.clone();
                let lua_handle = self.lua_handle.clone();
                let close_connection_event_subscriber =
                    self.close_connection_event_subscriber.take().unwrap();
                let outbound_message_receiver = self.outbound_message_receiver.take().unwrap();
                let id = self.id.clone();

                let handle: JoinHandle<()> = ASYNC_RUNTIME.spawn(async move {
                    start_loop(
                        ws_stream,
                        &inbound_event_publisher,
                        &lua_handle,
                        close_connection_event_subscriber,
                        outbound_message_receiver,
                    )
                    .await;

                    WEBSOCKET_CLIENT_REGISTRY.lock().remove(&id);
                });
                self.task_handle = Some(handle);
            }
            Err(err) => {
                return Err(err);
            }
        }

        Ok(())
    }

    fn send_event(&self, event: WebsocketClientInboundEvent) {
        self.inbound_event_publisher
            .send(event)
            .unwrap_or_else(|err| error!("Failed to send event: {}", err));
        self.lua_handle
            .send()
            .unwrap_or_else(|err| error!("Failed to send event: {}", err));
    }

    fn disconnect(&mut self) {
        self.close_connection_event_publisher
            .send(WebsocketClientCloseConnectionEvent::Graceful)
            .unwrap_or_else(move |err| {
                self.send_event(WebsocketClientInboundEvent::Error(
                    WebsocketClientError::SendMessageError(err.to_string()),
                ));
                ()
            });
    }

    fn send_data(&mut self, data: String) {
        self.outbound_message_publisher
            .send(data)
            .unwrap_or_else(move |err| {
                self.send_event(WebsocketClientInboundEvent::Error(
                    WebsocketClientError::SendMessageError(err.to_string()),
                ));
                ()
            });
    }
}

#[derive(Clone, Debug)]
pub enum WebsocketClientCloseConnectionEvent {
    Graceful,
    Forceful,
}

#[derive(Clone)]
struct WebsocketClientCallbacks {
    on_message: Option<Arc<LuaFunction<'static>>>,
    on_disconnect: Option<Arc<LuaFunction<'static>>>,
    on_connect: Option<Arc<LuaFunction<'static>>>,
    on_error: Option<Arc<LuaFunction<'static>>>,
}

impl WebsocketClientCallbacks {
    fn new(client_id: Uuid) -> Result<Self, Box<dyn Error>> {
        let lua = lua();
        let callbacks = lua
            .globals()
            .get::<_, LuaTable>("_WEBSOCKET_NVIM")?
            .get::<_, LuaTable>("clients")?
            .get::<_, LuaTable>("callbacks")?
            .get::<_, LuaTable>(client_id.to_string())?;

        Ok(Self {
            on_message: callbacks
                .get::<_, Option<LuaFunction>>("on_message")?
                .map(Arc::new),
            on_disconnect: callbacks
                .get::<_, Option<LuaFunction>>("on_disconnect")?
                .map(Arc::new),
            on_connect: callbacks
                .get::<_, Option<LuaFunction>>("on_connect")?
                .map(Arc::new),
            on_error: callbacks
                .get::<_, Option<LuaFunction>>("on_error")?
                .map(Arc::new),
        })
    }
}
