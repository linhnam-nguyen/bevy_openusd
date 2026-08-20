//! Direction-aware JSON frame codecs.

use serde::{Serialize, de::DeserializeOwned};

use crate::{ClientCommandEnvelope, ServerEventEnvelope, ViewportWireMessage};

/// A client-to-server wire record for the decomposed contract.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum ClientWireMessage {
    Command(ClientCommandEnvelope),
}

/// A server-to-client wire record for the decomposed contract.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum ServerWireMessage {
    Event(ServerEventEnvelope),
}

/// Encodes a legacy or decomposed message as one JSON Lines record.
pub fn encode_json_frame<T: Serialize>(message: &T) -> serde_json::Result<String> {
    let mut line = serde_json::to_string(message)?;
    line.push('\n');
    Ok(line)
}

/// Decodes one JSON Lines record, accepting surrounding whitespace.
pub fn decode_json_frame<T: DeserializeOwned>(line: &str) -> serde_json::Result<T> {
    serde_json::from_str(line)
}

/// Legacy protocol-version-1 codec retained for stdio compatibility.
pub fn encode_json_line(message: &ViewportWireMessage) -> serde_json::Result<String> {
    encode_json_frame(message)
}

/// Legacy protocol-version-1 codec retained for stdio compatibility.
pub fn decode_json_line(line: &str) -> serde_json::Result<ViewportWireMessage> {
    decode_json_frame(line)
}

pub fn encode_client_json_line(message: &ClientCommandEnvelope) -> serde_json::Result<String> {
    encode_json_frame(&ClientWireMessage::Command(message.clone()))
}

pub fn decode_client_json_line(line: &str) -> serde_json::Result<ClientCommandEnvelope> {
    match decode_json_frame::<ClientWireMessage>(line)? {
        ClientWireMessage::Command(envelope) => Ok(envelope),
    }
}

pub fn encode_server_json_line(message: &ServerEventEnvelope) -> serde_json::Result<String> {
    encode_json_frame(&ServerWireMessage::Event(message.clone()))
}

pub fn decode_server_json_line(line: &str) -> serde_json::Result<ServerEventEnvelope> {
    match decode_json_frame::<ServerWireMessage>(line)? {
        ServerWireMessage::Event(envelope) => Ok(envelope),
    }
}
