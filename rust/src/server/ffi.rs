use std::collections::HashMap;

use nvim_oxi::{Dictionary, Function, Object};
use uuid::Uuid;

use super::{WebsocketServer, WEBSOCKET_SERVER_REGISTRY};

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
    let is_active = registry.get(&server_id).is_some();
    Ok(is_active)
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
