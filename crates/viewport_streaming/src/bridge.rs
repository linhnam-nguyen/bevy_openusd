//! WebRTC DataChannel ↔ Bevy ECS protocol and input bridge.
//!
//! Deserializes incoming DataChannel text messages as `ViewportWireMessage::Command`
//! or `RemoteInputEvent`, and serializes outgoing `ViewportEventEnvelope` frames.

use serde::{Deserialize, Serialize};
use viewport_protocol::{
    ViewportCommandEnvelope, ViewportEventEnvelope, ViewportWireMessage, decode_json_line,
    encode_json_line,
};

/// Normalized mouse and keyboard inputs sent over DataChannel for remote viewport control.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum RemoteInputEvent {
    MouseMove { x: f32, y: f32 },
    MouseButton { button: u8, pressed: bool },
    MouseScroll { delta_x: f32, delta_y: f32 },
    KeyDown { key: String },
    KeyUp { key: String },
    ViewportResize { width: u32, height: u32 },
}

/// Dispatched DataChannel packet — either a viewport command or a raw input event.
#[derive(Debug, Clone)]
pub enum InboundPacket {
    Command(ViewportCommandEnvelope),
    Input(RemoteInputEvent),
}

/// Parses an incoming WebRTC DataChannel text frame into a typed packet.
pub fn parse_datachannel_frame(text: &str) -> Result<InboundPacket, String> {
    // Try parsing as standard ViewportWireMessage protocol envelope
    if let Ok(wire_message) = decode_json_line(text) {
        match wire_message {
            ViewportWireMessage::Command(envelope) => return Ok(InboundPacket::Command(envelope)),
            ViewportWireMessage::Event(_) => {
                return Err("received event envelope on incoming datachannel".to_owned());
            }
        }
    }

    // Try parsing as raw input event
    if let Ok(input_event) = serde_json::from_str::<RemoteInputEvent>(text) {
        return Ok(InboundPacket::Input(input_event));
    }

    Err(format!("unrecognized DataChannel payload: {text}"))
}

/// Serializes an outgoing ViewportEventEnvelope into a DataChannel text frame.
pub fn format_datachannel_event(event: ViewportEventEnvelope) -> Result<String, String> {
    let wire_message = ViewportWireMessage::Event(event);
    encode_json_line(&wire_message).map_err(|e| e.to_string())
}
