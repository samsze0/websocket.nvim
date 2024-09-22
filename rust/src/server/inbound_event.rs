use mlua::prelude::{IntoLua, Lua, LuaResult, LuaValue};
use nvim_oxi::conversion::ToObject;
use nvim_oxi::Object;
use uuid::Uuid;

#[derive(Clone)]
pub enum WebsocketServerError {
    ClientConnectionError(String),
    ClientTerminationError(Uuid, String),
    ServerTerminationError(String),
    ReceiveMessageError(Uuid, String),
    SendMessageError(Uuid, String),
    BroadcastMessageError(String),
}

#[derive(Clone)]
pub enum WebsocketServerInboundEvent {
    ClientConnected(Uuid),
    ClientDisconnected(Uuid),
    NewMessage(Uuid, String),
    Error(WebsocketServerError),
}

// Not necessary (for now)
impl ToObject for WebsocketServerError {
    fn to_object(self) -> Result<Object, nvim_oxi::conversion::Error> {
        match self {
            WebsocketServerError::ClientConnectionError(message) => Ok(Object::from(message)),
            WebsocketServerError::ClientTerminationError(_client_id, message) => {
                Ok(Object::from(message))
            }
            WebsocketServerError::ReceiveMessageError(_client_id, message) => {
                Ok(Object::from(message))
            }
            WebsocketServerError::SendMessageError(_client_id, message) => {
                Ok(Object::from(message))
            }
            WebsocketServerError::BroadcastMessageError(message) => Ok(Object::from(message)),
            WebsocketServerError::ServerTerminationError(message) => Ok(Object::from(message)),
        }
    }
}

impl<'lua> IntoLua<'lua> for WebsocketServerError {
    fn into_lua(self, lua: &'lua Lua) -> LuaResult<LuaValue<'lua>> {
        let vec = match self {
            WebsocketServerError::ClientConnectionError(message) => {
                vec![
                    ("type", "client_connection_error"),
                    ("message", message.leak()),
                ]
            }
            WebsocketServerError::ClientTerminationError(client_id, message) => {
                vec![
                    ("type", "client_temrination_error"),
                    ("client_id", client_id.to_string().leak()),
                    ("message", message.leak()),
                ]
            }
            WebsocketServerError::ReceiveMessageError(client_id, message) => {
                vec![
                    ("type", "receive_message_error"),
                    ("client_id", client_id.to_string().leak()),
                    ("message", message.leak()),
                ]
            }
            WebsocketServerError::SendMessageError(client_id, message) => {
                vec![
                    ("type", "send_message_error"),
                    ("client_id", client_id.to_string().leak()),
                    ("message", message.leak()),
                ]
            }
            WebsocketServerError::BroadcastMessageError(message) => {
                vec![
                    ("type", "broadcast_message_error"),
                    ("message", message.leak()),
                ]
            }
            WebsocketServerError::ServerTerminationError(message) => {
                vec![
                    ("type", "server_termination_error"),
                    ("message", message.leak()),
                ]
            }
        };
        Ok(LuaValue::Table(lua.create_table_from(vec)?))
    }
}
