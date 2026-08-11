//! WebSocket signaling for SDP/ICE bootstrap and connection lifecycle.
//!
//! Application commands do not travel through this socket. Once a connection
//! receives its offer, the WebRTC DataChannels are the application transport.

use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use log::{error, info, warn};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
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
        code: String,
        message: String,
    },
}

/// Commands passed between a signaling task and the WebRTC session manager.
#[derive(Debug)]
pub enum SessionCommand {
    ClientConnected {
        connection_id: u64,
        reply_tx: mpsc::Sender<SignalingMessage>,
    },
    ReceivedAnswer {
        connection_id: u64,
        sdp: String,
    },
    ReceivedIceCandidate {
        connection_id: u64,
        candidate: String,
        sdp_mid: Option<String>,
        sdp_mline_index: Option<u32>,
    },
    ClientDisconnected {
        connection_id: u64,
    },
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
    let next_connection_id = Arc::new(AtomicU64::new(1));

    info!(
        "[viewport-signaling] Listening on ws://{}",
        config.signaling_addr
    );

    loop {
        let (stream, peer_addr) = listener.accept().await?;
        let connection_id = next_connection_id.fetch_add(1, Ordering::Relaxed);
        info!(
            "[viewport-signaling] New connection {} from {}",
            connection_id, peer_addr
        );

        let session_tx = session_tx.clone();
        let auth_token = config.auth_token_secret.clone();
        tokio::spawn(async move {
            if let Err(error) =
                handle_connection(stream, peer_addr, connection_id, session_tx, auth_token).await
            {
                error!(
                    "[viewport-signaling] connection {} failed: {error}",
                    connection_id
                );
            }
        });
    }
}

async fn handle_connection(
    stream: TcpStream,
    peer_addr: SocketAddr,
    connection_id: u64,
    session_tx: mpsc::Sender<SessionCommand>,
    auth_token_secret: Option<String>,
) -> Result<()> {
    let ws_stream = tokio_tungstenite::accept_async(stream)
        .await
        .context("WebSocket handshake failed")?;
    let (mut ws_tx, mut ws_rx) = ws_stream.split();
    let (reply_tx, mut reply_rx) = mpsc::channel::<SignalingMessage>(32);
    let writer_connection_id = connection_id;

    tokio::spawn(async move {
        while let Some(message) = reply_rx.recv().await {
            if let Ok(json) = serde_json::to_string(&message)
                && ws_tx.send(Message::Text(json.into())).await.is_err()
            {
                warn!(
                    "[viewport-signaling] writer closed for connection {}",
                    writer_connection_id
                );
                break;
            }
        }
    });

    let mut joined = false;
    while let Some(message_result) = ws_rx.next().await {
        let message = match message_result {
            Ok(Message::Text(text)) => text,
            Ok(Message::Close(_)) | Err(_) => break,
            _ => continue,
        };

        let parsed: SignalingMessage = match serde_json::from_str(&message) {
            Ok(message) => message,
            Err(error) => {
                warn!(
                    "[viewport-signaling] invalid JSON from connection {}: {}",
                    connection_id, error
                );
                continue;
            }
        };

        match parsed {
            SignalingMessage::Join { token } => {
                if joined {
                    let _ = reply_tx
                        .send(SignalingMessage::Error {
                            code: "already_joined".to_owned(),
                            message: "signaling connection already joined".to_owned(),
                        })
                        .await;
                    continue;
                }

                if let Some(expected) = &auth_token_secret
                    && token.as_ref() != Some(expected)
                {
                    let _ = reply_tx
                        .send(SignalingMessage::Error {
                            code: "unauthorized".to_owned(),
                            message: "unauthorized token".to_owned(),
                        })
                        .await;
                    break;
                }

                joined = true;
                if session_tx
                    .send(SessionCommand::ClientConnected {
                        connection_id,
                        reply_tx: reply_tx.clone(),
                    })
                    .await
                    .is_err()
                {
                    break;
                }
            }
            SignalingMessage::Answer { sdp } if joined => {
                let _ = session_tx
                    .send(SessionCommand::ReceivedAnswer { connection_id, sdp })
                    .await;
            }
            SignalingMessage::Ice {
                candidate,
                sdp_mid,
                sdp_mline_index,
            } if joined => {
                let _ = session_tx
                    .send(SessionCommand::ReceivedIceCandidate {
                        connection_id,
                        candidate,
                        sdp_mid,
                        sdp_mline_index,
                    })
                    .await;
            }
            SignalingMessage::Answer { .. } | SignalingMessage::Ice { .. } => {
                let _ = reply_tx
                    .send(SignalingMessage::Error {
                        code: "join_required".to_owned(),
                        message: "send join before SDP or ICE messages".to_owned(),
                    })
                    .await;
            }
            _ => {}
        }
    }

    if joined {
        let _ = session_tx
            .send(SessionCommand::ClientDisconnected { connection_id })
            .await;
    }
    info!(
        "[viewport-signaling] connection {} closed for {}",
        connection_id, peer_addr
    );
    Ok(())
}
