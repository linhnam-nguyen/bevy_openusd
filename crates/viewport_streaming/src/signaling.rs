//! WebSocket signaling server for WebRTC SDP offer/answer exchange and ICE candidate relay.

use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use log::{error, info, warn};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use crate::config::StreamingConfig;

/// Wire messages exchanged over the WebSocket signaling channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SignalingMessage {
    Join {
        token: Option<String>,
    },
    Offer {
        sdp: String,
    },
    Answer {
        sdp: String,
    },
    Ice {
        candidate: String,
        sdp_mid: Option<String>,
        sdp_mline_index: Option<u32>,
    },
    Error {
        message: String,
    },
}

/// Commands passed between the signaling server task and the WebRTC session manager.
#[derive(Debug)]
pub enum SessionCommand {
    ClientConnected {
        reply_tx: mpsc::Sender<SignalingMessage>,
    },
    ReceivedAnswer {
        sdp: String,
    },
    ReceivedIceCandidate {
        candidate: String,
        sdp_mid: Option<String>,
        sdp_mline_index: Option<u32>,
    },
    ClientDisconnected,
}

/// Launches the WebSocket signaling listener task.
pub async fn run_signaling_server(
    config: StreamingConfig,
    session_tx: mpsc::Sender<SessionCommand>,
) -> Result<()> {
    let listener = TcpListener::bind(config.signaling_addr)
        .await
        .with_context(|| {
            format!(
                "Failed to bind signaling socket at {}",
                config.signaling_addr
            )
        })?;

    info!(
        "[viewport-signaling] Listening on ws://{}",
        config.signaling_addr
    );

    while let Ok((stream, peer_addr)) = listener.accept().await {
        info!("[viewport-signaling] New connection from {peer_addr}");
        let session_tx = session_tx.clone();
        let auth_token = config.auth_token_secret.clone();

        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, peer_addr, session_tx, auth_token).await {
                error!("[viewport-signaling] Connection error for {peer_addr}: {e}");
            }
        });
    }

    Ok(())
}

async fn handle_connection(
    stream: TcpStream,
    peer_addr: SocketAddr,
    session_tx: mpsc::Sender<SessionCommand>,
    auth_token_secret: Option<String>,
) -> Result<()> {
    let ws_stream = tokio_tungstenite::accept_async(stream)
        .await
        .context("WebSocket handshake failed")?;

    let (mut ws_tx, mut ws_rx) = ws_stream.split();
    let (reply_tx, mut reply_rx) = mpsc::channel::<SignalingMessage>(32);

    // Task forwarding outgoing signaling messages to the WebSocket client
    tokio::spawn(async move {
        while let Some(msg) = reply_rx.recv().await {
            if let Ok(json) = serde_json::to_string(&msg) {
                if ws_tx.send(Message::Text(json.into())).await.is_err() {
                    break;
                }
            }
        }
    });

    // Process incoming WebSocket messages
    while let Some(msg_result) = ws_rx.next().await {
        let msg = match msg_result {
            Ok(Message::Text(text)) => text,
            Ok(Message::Close(_)) | Err(_) => break,
            _ => continue,
        };

        let parsed: SignalingMessage = match serde_json::from_str(&msg) {
            Ok(m) => m,
            Err(e) => {
                warn!("[viewport-signaling] Invalid JSON message from {peer_addr}: {e}");
                continue;
            }
        };

        match parsed {
            SignalingMessage::Join { token } => {
                if let Some(expected) = &auth_token_secret {
                    if token.as_ref() != Some(expected) {
                        let _ = reply_tx
                            .send(SignalingMessage::Error {
                                message: "Unauthorized token".into(),
                            })
                            .await;
                        break;
                    }
                }
                let _ = session_tx
                    .send(SessionCommand::ClientConnected {
                        reply_tx: reply_tx.clone(),
                    })
                    .await;
            }
            SignalingMessage::Answer { sdp } => {
                let _ = session_tx
                    .send(SessionCommand::ReceivedAnswer { sdp })
                    .await;
            }
            SignalingMessage::Ice {
                candidate,
                sdp_mid,
                sdp_mline_index,
            } => {
                let _ = session_tx
                    .send(SessionCommand::ReceivedIceCandidate {
                        candidate,
                        sdp_mid,
                        sdp_mline_index,
                    })
                    .await;
            }
            _ => {}
        }
    }

    let _ = session_tx.send(SessionCommand::ClientDisconnected).await;
    info!("[viewport-signaling] Connection closed for {peer_addr}");
    Ok(())
}
