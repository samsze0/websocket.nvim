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

use super::{OutboundMessageReplayBuffer, WebsocketServerError, WebsocketServerInboundEvent};

pub struct WebsocketServerClient {
    id: Uuid,
    addr: SocketAddr,
    running: bool,
    running_publisher: UnboundedSender<bool>,
    outbound_message_publisher: UnboundedSender<String>,
    inbound_event_publisher: UnboundedSender<WebsocketServerInboundEvent>,
    inbound_event_handler: AsyncHandle,
}

impl WebsocketServerClient {
    pub(super) async fn run(
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
                                        tungstenite::Message::Binary(_data) => {
                                            info!("Server-client {} received binary data", id);
                                            inbound_event_publisher.send(WebsocketServerInboundEvent::Error(WebsocketServerError::ReceiveMessageError(id, "Binary data handling is not supported".to_string()))).unwrap();
                                            inbound_event_handler.send().unwrap();
                                        }
                                        tungstenite::Message::Frame(_frame) => {
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

    pub(super) fn terminate(&mut self) {
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

    pub(super) fn send_data(&self, data: String) {
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

    pub(super) fn is_active(&self) -> bool {
        self.running
    }
}
