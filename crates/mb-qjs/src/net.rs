//! The web for scripts (feature `net`): fetch/XHR requests and WebSockets
//! run on threads; what they bring is handed to the page's JS on its next
//! tick (`__mb_net_done`, `__mb_ws_event` in dom.js), so a slow server never
//! holds the page.

use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Something a request or socket did, for the page's JS.
pub enum Event {
    /// A request finished: id, status (0: failed), body, error text.
    Done { id: u32, status: u16, body: Vec<u8>, error: String },
    /// A socket event: id, "open" | "message" | "close" | "error", data.
    Ws { id: u32, kind: &'static str, data: String },
}

#[derive(Default)]
pub struct Net {
    seq: u32,
    events: Option<(Sender<Event>, Arc<Mutex<Receiver<Event>>>)>,
    /// Requests and sockets still running.
    pub running: Arc<Mutex<usize>>,
    /// What scripts send on each open socket.
    sockets: HashMap<u32, Sender<Option<String>>>,
}

impl Net {
    fn tx(&mut self) -> Sender<Event> {
        if self.events.is_none() {
            let (tx, rx) = channel();
            self.events = Some((tx, Arc::new(Mutex::new(rx))));
        }
        self.events.as_ref().unwrap().0.clone()
    }

    /// Events that arrived since the last call.
    pub fn take(&mut self) -> Vec<Event> {
        let Some((_, rx)) = &self.events else { return Vec::new() };
        let rx = rx.lock().unwrap();
        let mut out = Vec::new();
        loop {
            match rx.try_recv() {
                Ok(e) => out.push(e),
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
            }
        }
        out
    }

    pub fn busy(&self) -> bool {
        *self.running.lock().unwrap() > 0
    }

    /// Starts a request; its result comes as `Event::Done` with the returned id.
    pub fn request(&mut self, method: String, url: String, body: Option<String>, headers: Vec<(String, String)>) -> u32 {
        self.seq += 1;
        let id = self.seq;
        let tx = self.tx();
        let running = self.running.clone();
        *running.lock().unwrap() += 1;
        std::thread::spawn(move || {
            let (status, body, error) = match http(&method, &url, body, &headers) {
                Ok((s, b)) => (s, b, String::new()),
                Err(e) => (0, Vec::new(), e),
            };
            let _ = tx.send(Event::Done { id, status, body, error });
            *running.lock().unwrap() -= 1;
        });
        id
    }

    /// Opens a WebSocket (ws:// only); events come as `Event::Ws`.
    pub fn ws_open(&mut self, url: String) -> u32 {
        self.seq += 1;
        let id = self.seq;
        let tx = self.tx();
        let (out_tx, out_rx) = channel::<Option<String>>();
        self.sockets.insert(id, out_tx);
        let running = self.running.clone();
        *running.lock().unwrap() += 1;
        std::thread::spawn(move || {
            ws_thread(id, &url, &tx, out_rx);
            *running.lock().unwrap() -= 1;
        });
        id
    }

    pub fn ws_send(&mut self, id: u32, text: String) {
        if let Some(s) = self.sockets.get(&id) {
            let _ = s.send(Some(text));
        }
    }

    pub fn ws_close(&mut self, id: u32) {
        if let Some(s) = self.sockets.remove(&id) {
            let _ = s.send(None);
        }
    }

    /// A new page: its sockets close, late answers are dropped.
    pub fn reset(&mut self) {
        for (_, s) in self.sockets.drain() {
            let _ = s.send(None);
        }
        self.events = None;
    }
}

fn http(method: &str, url: &str, body: Option<String>, headers: &[(String, String)]) -> Result<(u16, Vec<u8>), String> {
    use std::io::Read;
    use std::sync::OnceLock;
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    let agent = AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            // Answers with an error status still reach the page (fetch's r.ok).
            .http_status_as_error(false)
            .user_agent("Mozilla/5.0 (X11; Linux armv7l) MiniBrowser/0.8")
            .build()
            .into()
    });
    let mut req = ureq::http::Request::builder().method(method).uri(url);
    for (k, v) in headers {
        req = req.header(k, v);
    }
    let req = req.body(body.unwrap_or_default()).map_err(|e| e.to_string())?;
    let mut resp = agent.run(req).map_err(|e| e.to_string())?;
    let status = resp.status().as_u16();
    let mut out = Vec::new();
    resp.body_mut().as_reader().take(32 << 20).read_to_end(&mut out).map_err(|e| e.to_string())?;
    Ok((status, out))
}

fn ws_thread(id: u32, url: &str, tx: &Sender<Event>, out: Receiver<Option<String>>) {
    use tungstenite::{Error, Message};
    let ev = |kind, data: String| tx.send(Event::Ws { id, kind, data }).is_ok();
    let (mut ws, _) = match tungstenite::connect(url) {
        Ok(x) => x,
        Err(e) => {
            ev("error", e.to_string());
            ev("close", String::new());
            return;
        }
    };
    // Reads time out often, so what the page sends goes out promptly.
    if let tungstenite::stream::MaybeTlsStream::Plain(s) = ws.get_ref() {
        let _ = s.set_read_timeout(Some(Duration::from_millis(20)));
    }
    if !ev("open", String::new()) {
        return;
    }
    loop {
        loop {
            match out.try_recv() {
                Ok(Some(text)) => {
                    if ws.send(Message::text(text)).is_err() {
                        ev("close", String::new());
                        return;
                    }
                }
                Ok(None) | Err(TryRecvError::Disconnected) => {
                    let _ = ws.close(None);
                    ev("close", String::new());
                    return;
                }
                Err(TryRecvError::Empty) => break,
            }
        }
        match ws.read() {
            Ok(Message::Text(t)) => {
                if !ev("message", t.to_string()) {
                    return;
                }
            }
            Ok(Message::Binary(b)) => {
                if !ev("message", String::from_utf8_lossy(&b).into_owned()) {
                    return;
                }
            }
            Ok(Message::Close(_)) => {
                ev("close", String::new());
                return;
            }
            Ok(_) => {}
            Err(Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(e) => {
                ev("error", e.to_string());
                ev("close", String::new());
                return;
            }
        }
    }
}
