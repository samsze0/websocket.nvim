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
        ("get_clients", Object::from(Function::from_fn(get_clients)))
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

fn get_clients((): ()) -> nvim_oxi::Result<Dictionary> {
    let registry = WEBSOCKET_CLIENT_REGISTRY.lock();
    let clients: HashMap<nvim_oxi::String, Dictionary> = registry
        .get_all()
        .into_iter()
        .map(|(id, client)| (id.to_string().as_str().into(), client.into()))
        .collect();
    Ok(Dictionary::from_iter(clients))
}

impl From<&WebsocketClient> for Dictionary {
    fn from(client: &WebsocketClient) -> Self {
        Dictionary::from_iter::<[(&str, &str); 2]>([
            ("id", client.id.to_string().as_str().into()),
            ("connect_addr", client.connect_addr.to_string().as_str().into())
        ])
    }
}