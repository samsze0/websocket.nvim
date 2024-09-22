use std::{collections::HashMap, ops::Deref};

use nvim_oxi::{Dictionary, Function, Object};
use uuid::Uuid;

use super::{WebsocketServer, WebsocketServerClient, WEBSOCKET_SERVER_REGISTRY};

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
            "get_servers",
            Object::from(Function::from_fn(get_servers)),
        ),
        (
            "get_clients",
            Object::from(Function::from_fn(get_clients)),
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

fn get_servers((): ()) -> nvim_oxi::Result<Dictionary> {
    let registry = WEBSOCKET_SERVER_REGISTRY.lock();
    let servers: HashMap<nvim_oxi::String, Dictionary> = registry
        .get_all()
        .iter()
        .map(|(id, server)| (id.to_string().as_str().into(), server.into()))
        .collect();
    Ok(Dictionary::from_iter(servers))
}

fn get_clients(server_id: String) -> nvim_oxi::Result<Dictionary> {
    let registry = WEBSOCKET_SERVER_REGISTRY.lock();
    let server_id = Uuid::parse_str(&server_id).unwrap();
    let server = registry.get(&server_id).unwrap();
    let clients: HashMap<nvim_oxi::String, Dictionary> = server
        .clients
        .lock()
        .iter()
        .map(|(id, client)| (id.to_string().as_str().into(), client.lock().deref().into()))
        .collect();
    Ok(Dictionary::from_iter(clients))
}


impl From<&WebsocketServer> for Dictionary {
    fn from(server: &WebsocketServer) -> Self {
        Dictionary::from_iter::<[(&str, &str); 3]>([
            ("id", server.id.to_string().as_str().into()),
            ("host", server.host.to_string().as_str().into()),
            ("port", server.port.to_string().as_str().into())
        ])
    }
}


impl From<&WebsocketServerClient> for Dictionary {
    fn from(client: &WebsocketServerClient) -> Self {
        Dictionary::from_iter::<[(&str, &str); 2]>([
            ("id", client.id.to_string().as_str().into()),
            ("addr", client.addr.to_string().as_str().into())
        ])
    }
}