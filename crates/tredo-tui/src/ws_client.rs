//! WebSocket client for real-time streaming from the backend.
//!
//! Runs in a background thread using blocking tungstenite. Parses incoming
//! JSON messages and sends them to the main event loop via an mpsc channel.
//! Auto-reconnects on disconnect with exponential backoff.
//!
//! Message types:
//! - `cot`    → COT log entry
//! - `price`  → Crypto price update
//! - `health` → System health status
//! - `portfolio` → Portfolio snapshot
//! - `signal` → Aggregated signal / skill vote
//! - `ping`   → Keepalive (no action needed)

use std::sync::mpsc;
use std::time::Duration;
use tungstenite::{connect, Message};

/// Start the WebSocket client in a background thread.
///
/// Messages are forwarded as raw JSON strings through the returned `mpsc::Receiver`.
/// On disconnect, the thread automatically retries with backoff (1s, 2s, 4s, max 30s).
pub fn start_ws_client(api_base: &str) -> mpsc::Receiver<String> {
    let ws_url = api_base
        .replace("http://", "ws://")
        .replace("https://", "wss://")
        + "/ws";

    let (tx, rx) = mpsc::channel::<String>();

    std::thread::spawn(move || {
        let mut retry_delay = Duration::from_secs(1);

        loop {
            match connect(&ws_url) {
                Ok((mut socket, _)) => {
                    retry_delay = Duration::from_secs(1); // Reset on successful connect
                    loop {
                        match socket.read() {
                            Ok(Message::Text(text)) => {
                                // Forward raw JSON string to the main thread
                                if tx.send(text).is_err() {
                                    // Main thread dropped the receiver — shutting down
                                    return;
                                }
                            }
                            Ok(Message::Binary(_data)) => {
                                // Binary not expected; ignore
                            }
                            Ok(Message::Ping(data)) => {
                                // Respond to server pings
                                let _ = socket.send(Message::Pong(data));
                            }
                            Ok(Message::Pong(_)) => {
                                // Server responded to our ping; nothing to do
                            }
                            Ok(Message::Close(_)) => {
                                // Server closed the connection; break to reconnect
                                break;
                            }
                            Err(_) => {
                                // Connection error; break to reconnect
                                break;
                            }
                            _ => {}
                        }
                    }
                }
                Err(_) => {
                    // Connection refused or failed — will retry
                }
            }

            // Exponential backoff with cap at 30s
            std::thread::sleep(retry_delay);
            retry_delay = (retry_delay * 2).min(Duration::from_secs(30));
        }
    });

    rx
}
