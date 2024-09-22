use std::collections::HashMap;

use nvim_oxi::{Dictionary, Function, Object};
use uuid::Uuid;

use super::WebsocketClient;
use super::WEBSOCKET_CLIENT_REGISTRY;

pub fn websocket_client_ffi() -> Dictionary {
    Dictionary::from_iter([
        (
            "connect",
            Object::from(Function::from_fn(create_client_and_connect)),
        ),
        ("disconnect", Object::from(Function::from_fn(disconnect))),
        ("send_data", Object::from(Function::from_fn(send_data))),
        ("is_active", Object::from(Function::from_fn(is_active)))
    ])
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
    let is_active = registry.get(&client_id).is_some();
    Ok(is_active)
}
