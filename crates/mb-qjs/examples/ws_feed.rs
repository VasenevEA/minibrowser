//! A WebSocket test server: echoes what it gets and pushes a reading every
//! 500 ms, like a controller's live feed.
//!   cargo run -p mb-qjs --features net --example ws_feed -- [127.0.0.1:8765]
use std::net::TcpListener;
use std::time::{Duration, Instant};

use tungstenite::{accept, Message};

fn main() {
    let addr = std::env::args().nth(1).unwrap_or_else(|| "127.0.0.1:8765".into());
    let server = TcpListener::bind(&addr).expect("bind");
    eprintln!("ws feed on ws://{addr}");
    for stream in server.incoming().flatten() {
        std::thread::spawn(move || {
            stream.set_read_timeout(Some(Duration::from_millis(50))).ok();
            let Ok(mut ws) = accept(stream) else { return };
            let start = Instant::now();
            let mut next = Duration::ZERO;
            let mut n = 0u32;
            loop {
                if start.elapsed() >= next {
                    n += 1;
                    let t = start.elapsed().as_secs_f64();
                    let msg = format!(r#"{{"n":{n},"temp":{:.1},"pressure":{:.2}}}"#, 70.0 + 5.0 * (t * 0.7).sin(), 3.2 + 0.3 * (t * 1.3).cos());
                    if ws.send(Message::text(msg)).is_err() {
                        return;
                    }
                    next += Duration::from_millis(500);
                }
                match ws.read() {
                    Ok(Message::Text(t)) => {
                        if ws.send(Message::text(format!(r#"{{"echo":{}}}"#, serde_json::Value::String(t.to_string())))).is_err() {
                            return;
                        }
                    }
                    Ok(Message::Close(_)) => return,
                    Ok(_) => {}
                    Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
                    Err(_) => return,
                }
            }
        });
    }
}
