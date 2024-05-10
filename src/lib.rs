use nvim_oxi::conversion::{Error as ConversionError, FromObject, ToObject};
use nvim_oxi::serde::{Deserializer, Serializer};
use nvim_oxi::{api, lua, print, Dictionary, Function, Object};
use serde::{Deserialize, Serialize};

#[nvim_oxi::module]
fn websocket_nvim() -> nvim_oxi::Result<Dictionary> {
    let api = Dictionary::from_iter([
        ("new_client",  Object::from(Function::from_fn(new_client))),
        ("connect",     Object::from(Function::from_fn(connect))),
        ("disconnect",  Object::from(Function::from_fn(disconnect))),
        ("send_data",   Object::from(Function::from_fn(send_data))),
        ("is_active",   Object::from(Function::from_fn(is_active))),
        ("replay_messages", Object::from(Function::from_fn(replay_messages))),
        ("check_replay_messages", Object::from(Function::from_fn(check_replay_messages))),
    ]);

    Ok(api)
}

fn new_client(_: ()) -> nvim_oxi::Result<WebsocketClient> {
    Ok(WebsocketClient::new())
}

fn connect(mut client: WebsocketClient) -> nvim_oxi::Result<WebsocketClient> {
    client.connect();
    Ok(client)
}

fn disconnect(mut client: WebsocketClient) -> nvim_oxi::Result<WebsocketClient> {
    client.disconnect();
    Ok(client)
}

fn send_data((mut client, data): (WebsocketClient, String)) -> nvim_oxi::Result<WebsocketClient> {
    client.send_data(data);
    Ok(client)
}

fn is_active(client: WebsocketClient) -> nvim_oxi::Result<bool> {
    Ok(client.is_active())
}

fn replay_messages(client: WebsocketClient) -> nvim_oxi::Result<WebsocketClient> {
    client.replay_messages();
    Ok(client)
}

fn check_replay_messages(client: WebsocketClient) -> nvim_oxi::Result<Vec<String>> {
    Ok(client.message_replay_buffer.messages.clone())
}

// frame_size?: number, on_message: (fun(client: UtilsWebsocketClient, message: string): nil), on_disconnect?: (fun(client: UtilsWebsocketClient): nil), on_connect?: (fun(client: UtilsWebsocketClient): nil), on_upgrade?: (fun(client: UtilsWebsocketClient): nil), headers?: table<string, string>

#[derive(Serialize, Deserialize)]
struct WebsocketClient {
    running: bool,
    message_replay_buffer: MessageReplayBuffer,
}

impl WebsocketClient {
    fn new() -> Self {
        Self {
            running: false,
            message_replay_buffer: MessageReplayBuffer::new(),
        }
    }

    fn connect(&mut self) {
        self.running = true;
    }

    fn disconnect(&mut self) {
        self.running = false;
    }

    fn send_data(&mut self, data: String) {
        self.message_replay_buffer.add(data);
    }

    fn is_active(&self) -> bool {
        self.running
    }

    fn replay_messages(&self) {
        self.message_replay_buffer.replay();
    }
}

impl FromObject for WebsocketClient {
    fn from_object(obj: Object) -> Result<Self, ConversionError> {
        Self::deserialize(Deserializer::new(obj)).map_err(Into::into)
    }
}

impl ToObject for WebsocketClient {
    fn to_object(self) -> Result<Object, ConversionError> {
        self.serialize(Serializer::new()).map_err(Into::into)
    }
}

impl lua::Poppable for WebsocketClient {
    unsafe fn pop(
        lstate: *mut lua::ffi::lua_State,
    ) -> Result<Self, lua::Error> {
        let obj = Object::pop(lstate)?;
        Self::from_object(obj)
            .map_err(lua::Error::pop_error_from_err::<Self, _>)
    }
}

impl lua::Pushable for WebsocketClient {
    unsafe fn push(
        self,
        lstate: *mut lua::ffi::lua_State,
    ) -> Result<std::ffi::c_int, lua::Error> {
        self.to_object()
            .map_err(lua::Error::push_error_from_err::<Self, _>)?
            .push(lstate)
    }
}

#[derive(Serialize, Deserialize)]
struct MessageReplayBuffer {
    messages: Vec<String>,
}

impl MessageReplayBuffer {
    fn new() -> Self {
        Self { messages: Vec::new() }
    }

    fn add(&mut self, message: String) {
        self.messages.push(message);
    }

    fn replay(&self) {
        for message in &self.messages {
            print!("From Rust: {}", message);
        }
    }
}