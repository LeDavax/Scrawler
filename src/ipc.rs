//! Local effect channel between the MCP bridge and the running native application.

use serde_json::Value;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::Sender;
use std::thread;

const ADDRESS: &str = "127.0.0.1:43173";

/// Listens only on localhost so MCP can have an effect applied by the window
/// without exposing the application to the network.
pub fn start_effect_listener(sender: Sender<Value>) -> io::Result<()> {
    let listener = TcpListener::bind(ADDRESS)?;
    thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut payload = String::new();
            if stream.read_to_string(&mut payload).is_ok()
                && let Ok(effect) = serde_json::from_str(&payload)
            {
                let _ = sender.send(effect);
            }
        }
    });
    Ok(())
}

/// Called by the MCP bridge after executing the Lua handler.
pub fn send_effect(effect: &Value) -> io::Result<()> {
    let mut stream = TcpStream::connect(ADDRESS)?;
    let payload = serde_json::to_vec(effect).expect("Scrawler effects serialize to JSON");
    stream.write_all(&payload)
}
