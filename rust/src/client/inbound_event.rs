use std::error::Error;

use mlua::prelude::*;

#[derive(Clone, Debug)]
pub enum WebsocketClientError {
    ConnectionError(String),
    DisconnectionError(String),
    ReceiveMessageError(String),
    SendMessageError(String),
}

#[derive(Clone, Debug)]
pub enum WebsocketClientInboundEvent {
    Connected,
    Disconnected,
    NewMessage(String),
    Error(WebsocketClientError),
}

impl nvim_oxi::conversion::ToObject for WebsocketClientError {
    fn to_object(self) -> Result<nvim_oxi::Object, nvim_oxi::conversion::Error> {
        match self {
            WebsocketClientError::ConnectionError(message) => Ok(nvim_oxi::Object::from(message)),
            WebsocketClientError::DisconnectionError(message) => {
                Ok(nvim_oxi::Object::from(message))
            }
            WebsocketClientError::ReceiveMessageError(message) => {
                Ok(nvim_oxi::Object::from(message))
            }
            WebsocketClientError::SendMessageError(message) => Ok(nvim_oxi::Object::from(message)),
        }
    }
}

impl<'lua> IntoLua<'lua> for WebsocketClientError {
    fn into_lua(self, lua: &'lua Lua) -> LuaResult<LuaValue<'lua>> {
        let vec = match self {
            WebsocketClientError::ConnectionError(message) => {
                vec![("type", "connection_error"), ("message", message.leak())]
            }
            WebsocketClientError::DisconnectionError(message) => {
                vec![("type", "disconnection_error"), ("message", message.leak())]
            }
            WebsocketClientError::ReceiveMessageError(message) => {
                vec![
                    ("type", "receive_message_error"),
                    ("message", message.leak()),
                ]
            }
            WebsocketClientError::SendMessageError(message) => {
                vec![("type", "send_message_error"), ("message", message.leak())]
            }
        };
        Ok(LuaValue::Table(lua.create_table_from(vec)?))
    }
}

// Unnecessary trait. Could have just wrap the received message in an enum instead
trait TryFromStr {
    fn try_from_string(s: String) -> Result<Self, Box<dyn Error>>
    where
        Self: Sized;
}

impl TryFromStr for WebsocketClientInboundEvent {
    // Blanket implementation already exists, cannot override TryFrom trait's try_from
    fn try_from_string(s: String) -> Result<Self, Box<dyn Error>> {
        Ok(WebsocketClientInboundEvent::NewMessage(s))
    }
}
