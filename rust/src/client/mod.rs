use std::collections::HashMap;
use std::error::Error;
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use mlua::prelude::{LuaFunction, LuaTable};
use nvim_oxi::libuv::AsyncHandle;
use nvim_oxi::{mlua::lua, schedule};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use url::Url;
use uuid::Uuid;

use log::{debug, error, info};
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
    outbound_message_publisher: UnboundedSender<String>,
    inbound_event_publisher: UnboundedSender<WebsocketClientInboundEvent>,
    lua_handle: AsyncHandle,
    task_handle: JoinHandle<()>,
}

async fn start_client(
    connect_addr: Url,
    extra_headers: HashMap<String, String>,
    inbound_event_publisher: UnboundedSender<WebsocketClientInboundEvent>,
    lua_handle: AsyncHandle,
    mut close_connection_event_subscriber: UnboundedReceiver<WebsocketClientCloseConnectionEvent>,
    mut outbound_message_receiver: UnboundedReceiver<String>,
) {
    let send_event = move |event: WebsocketClientInboundEvent| {
        inbound_event_publisher.send(event).unwrap();
        lua_handle.send().unwrap();
    };

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
            send_event(WebsocketClientInboundEvent::Error(
                WebsocketClientError::ConnectionError(err.to_string()),
            ));
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

    let (ws_stream, _response) = tokio_tungstenite::connect_async(request)
        .await
        .map_err(|err| {
            send_event(WebsocketClientInboundEvent::Error(
                WebsocketClientError::ConnectionError(err.to_string()),
            ));
            err
        })
        .unwrap();
    info!("{} WebSocket handshake completed", connect_addr.as_str());
    send_event(WebsocketClientInboundEvent::Connected);

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
                            send_event(WebsocketClientInboundEvent::NewMessage(data));
                        } else if message.is_binary() {
                            send_event(WebsocketClientInboundEvent::Error(WebsocketClientError::ReceiveMessageError("Binary data is not supported".to_string())));
                            error!("Binary data is not supported");
                        } else if message.is_close() {
                            info!("Received close frame from server");
                            break;
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
                                ws_sender.send(tungstenite::Message::Close(None)).await.unwrap();
                            }
                            WebsocketClientCloseConnectionEvent::Forceful => {
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
                        ws_sender.send(tungstenite::Message::Text(message)).await.unwrap();
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

        let connect_addr_clone = connect_addr.clone();
        let extra_headers_clone = extra_headers.clone();
        let inbound_event_publisher_clone = inbound_event_publisher.clone();
        let lua_handle_clone = lua_handle.clone();

        let handle: JoinHandle<()> = ASYNC_RUNTIME.spawn(async move {
            start_client(
                connect_addr_clone,
                extra_headers_clone,
                inbound_event_publisher_clone,
                lua_handle_clone,
                close_connection_event_subscriber,
                outbound_message_receiver,
            )
            .await;

            println!("WebSocket client has finished.");
            WEBSOCKET_CLIENT_REGISTRY.lock().remove(&id);
        });

        // Store the handle in the WebsocketClient struct
        Ok(Self {
            id,
            connect_addr,
            extra_headers,
            close_connection_event_publisher,
            outbound_message_publisher,
            inbound_event_publisher,
            lua_handle,
            task_handle: handle, // Add this field to the struct
        })
    }

    fn send_event(&self, event: WebsocketClientInboundEvent) {
        self.inbound_event_publisher.send(event).unwrap();
        self.lua_handle.send().unwrap();
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
