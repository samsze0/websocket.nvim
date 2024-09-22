use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use nvim_oxi::libuv::AsyncHandle;
use parking_lot::Mutex;
use tokio::net::TcpStream;
use tokio::sync::mpsc::{self, UnboundedSender};
use uuid::Uuid;

use log::{self, error, info};
use tokio_tungstenite::tungstenite::{self};

use super::{
    OutboundMessageReplayBuffer, WebsocketServerCloseConnectionEvent, WebsocketServerError,
    WebsocketServerInboundEvent,
};

pub struct WebsocketServerClient {
    id: Uuid,
    addr: SocketAddr,
    outbound_message_publisher: UnboundedSender<String>,
    inbound_event_publisher: UnboundedSender<WebsocketServerInboundEvent>,
    lua_handle: AsyncHandle,
    close_connection_event_publisher: UnboundedSender<WebsocketServerCloseConnectionEvent>,
}

impl WebsocketServerClient {
    pub(super) async fn run(
        stream: TcpStream,
        addr: SocketAddr,
        inbound_event_publisher: UnboundedSender<WebsocketServerInboundEvent>,
        lua_handle: AsyncHandle,
        extra_response_headers: HashMap<String, String>,
        message_replay_buffer: Arc<Mutex<OutboundMessageReplayBuffer>>,
        clients: Arc<Mutex<HashMap<Uuid, Arc<Mutex<WebsocketServerClient>>>>>,
    ) -> () {
        let id = Uuid::new_v4();
        let (outbound_message_publisher, mut outbound_message_subscriber) =
            mpsc::unbounded_channel::<String>();
        let (close_connection_event_publisher, mut close_connection_event_subscriber) =
            mpsc::unbounded_channel::<WebsocketServerCloseConnectionEvent>();

        let inbound_event_publisher_clone = inbound_event_publisher.clone();
        let lua_handle_clone = lua_handle.clone();
        let send_event = move |event: WebsocketServerInboundEvent| {
            inbound_event_publisher_clone.send(event).unwrap();
            lua_handle_clone.send().unwrap();
        };

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

                // TODO: Expose ways to check if client connection is allowed. Something like a predicate that
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

        send_event(WebsocketServerInboundEvent::ClientConnected(id));

        // Replay all messages to the new client (append to "queue")
        for message in message_replay_buffer.lock().messages.clone() {
            outbound_message_publisher.send(message).unwrap();
        }

        let client = Self {
            id,
            addr,
            outbound_message_publisher,
            inbound_event_publisher: inbound_event_publisher,
            close_connection_event_publisher: close_connection_event_publisher,
            lua_handle: lua_handle,
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
                                            send_event(WebsocketServerInboundEvent::NewMessage(id, data));
                                        }
                                        tungstenite::Message::Binary(_data) => {
                                            info!("Server-client {} received binary data", id);
                                            send_event(WebsocketServerInboundEvent::Error(WebsocketServerError::ReceiveMessageError(id, "Binary data handling is not supported".to_string())));
                                        }
                                        tungstenite::Message::Frame(_frame) => {
                                            info!("Server-client {} received raw frame data", id);
                                            send_event(WebsocketServerInboundEvent::Error(WebsocketServerError::ReceiveMessageError(id, "Raw frame data handling is not supported".to_string())));
                                        }
                                        // Ping, pong, close are handled by tokio-tungstenite
                                        _ => {}
                                    }
                                },
                                Err(err) => {
                                    error!("Server-client {} received error: {}", id, err);
                                    send_event(WebsocketServerInboundEvent::Error(WebsocketServerError::ReceiveMessageError(id, err.to_string())));
                                }
                            }
                        }
                        None => {
                            panic!("Server-client {} websocket receiver channel closed unexpecetedly", id);
                        },
                    }
                }
                maybe_close_connection_event = close_connection_event_subscriber.recv() => {
                    match maybe_close_connection_event {
                        Some(close_connection_event) => {
                            match close_connection_event {
                                WebsocketServerCloseConnectionEvent::Graceful => {
                                    info!("Server-client {} received termination signal", id);
                                    ws_sender.send(tungstenite::Message::Close(None)).await.unwrap();
                                    break;
                                }
                                WebsocketServerCloseConnectionEvent::Forceful => {
                                    info!("Server-client {} received forceful termination signal", id);
                                    break;
                                }
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

        send_event(WebsocketServerInboundEvent::ClientDisconnected(id));
    }

    fn send_event(&self, event: WebsocketServerInboundEvent) {
        self.inbound_event_publisher.send(event).unwrap();
        self.lua_handle.send().unwrap();
    }

    pub(super) fn terminate(&mut self) {
        self.close_connection_event_publisher
            .send(WebsocketServerCloseConnectionEvent::Graceful)
            .unwrap_or_else(move |err| {
                self.send_event(WebsocketServerInboundEvent::Error(
                    WebsocketServerError::ClientTerminationError(self.id, err.to_string()),
                ));
                ()
            });
    }

    pub(super) fn send_data(&self, data: String) {
        self.outbound_message_publisher
            .send(data)
            .unwrap_or_else(move |err| {
                self.send_event(WebsocketServerInboundEvent::Error(
                    WebsocketServerError::SendMessageError(self.id, err.to_string()),
                ));
                ()
            });
    }
}
