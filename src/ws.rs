//! WebSocket client (Postman "WebSocket request").

use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::time::Duration;

use tungstenite::client::IntoClientRequest;
use tungstenite::http::{HeaderName, HeaderValue};
use tungstenite::Message;

use crate::settings::Settings;

pub enum WsCmd {
    Text(String),
    Binary(Vec<u8>),
    Ping,
    Close,
}

#[derive(Clone, Debug)]
pub enum WsEvent {
    Connected(Vec<(String, String)>),
    Message { text: String, incoming: bool, binary: bool, time: String },
    Error(String),
    Closed(String),
}

pub struct WsConn {
    pub tx: Sender<WsCmd>,
    pub rx: Receiver<WsEvent>,
}

fn now() -> String {
    chrono::Local::now().format("%H:%M:%S%.3f").to_string()
}

pub fn connect(url: &str, headers: Vec<(String, String)>, settings: &Settings, repaint: impl Fn() + Send + 'static) -> WsConn {
    let (ctx, crx) = channel::<WsCmd>();
    let (etx, erx) = channel::<WsEvent>();
    let url = url.to_string();
    let settings = settings.clone();
    std::thread::spawn(move || {
        let send = |e: WsEvent| {
            let _ = etx.send(e);
            repaint();
        };
        let mut u = url.trim().to_string();
        if !u.contains("://") {
            u = format!("ws://{u}");
        }
        let parsed = match crate::http::parse_url(&u, true) {
            Ok(p) => p,
            Err(e) => return send(WsEvent::Error(e)),
        };
        let opts = crate::prepare::http_options(&settings, &serde_json::Map::new(), None);
        let conn = match crate::http::connect(&parsed, &opts, if settings.timeout_ms > 0 { settings.timeout_ms } else { 30_000 }, true) {
            Ok(c) => c,
            Err(e) => return send(WsEvent::Error(e)),
        };
        let mut req = match u.as_str().into_client_request() {
            Ok(r) => r,
            Err(e) => return send(WsEvent::Error(e.to_string())),
        };
        for (k, v) in headers {
            if let (Ok(n), Ok(v)) = (HeaderName::from_bytes(k.as_bytes()), HeaderValue::from_str(&v)) {
                req.headers_mut().insert(n, v);
            }
        }
        let (mut sock, resp) = match tungstenite::client::client(req, conn.stream) {
            Ok(x) => x,
            Err(e) => return send(WsEvent::Error(format!("Handshake failed: {e}"))),
        };
        let hs: Vec<(String, String)> = resp.headers().iter().map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string())).collect();
        send(WsEvent::Connected(hs));
        sock.get_ref().set_timeout(Some(Duration::from_millis(40)));
        loop {
            match crx.try_recv() {
                Ok(WsCmd::Text(t)) => {
                    if let Err(e) = sock.send(Message::text(t.clone())) {
                        return send(WsEvent::Error(e.to_string()));
                    }
                    send(WsEvent::Message { text: t, incoming: false, binary: false, time: now() });
                }
                Ok(WsCmd::Binary(b)) => {
                    let n = b.len();
                    if let Err(e) = sock.send(Message::binary(b)) {
                        return send(WsEvent::Error(e.to_string()));
                    }
                    send(WsEvent::Message { text: format!("<binary {n} bytes>"), incoming: false, binary: true, time: now() });
                }
                Ok(WsCmd::Ping) => {
                    let _ = sock.send(Message::Ping(Vec::new().into()));
                }
                Ok(WsCmd::Close) | Err(TryRecvError::Disconnected) => {
                    let _ = sock.close(None);
                    let _ = sock.flush();
                    return send(WsEvent::Closed("Disconnected by client".into()));
                }
                Err(TryRecvError::Empty) => {}
            }
            match sock.read() {
                Ok(Message::Text(t)) => send(WsEvent::Message { text: t.to_string(), incoming: true, binary: false, time: now() }),
                Ok(Message::Binary(b)) => {
                    let text = match std::str::from_utf8(&b) {
                        Ok(s) => s.to_string(),
                        Err(_) => format!("<binary {} bytes> {}", b.len(), hex::encode(&b[..b.len().min(64)])),
                    };
                    send(WsEvent::Message { text, incoming: true, binary: true, time: now() })
                }
                Ok(Message::Close(f)) => {
                    let reason = f.map(|f| format!("{} {}", u16::from(f.code), f.reason)).unwrap_or_default();
                    return send(WsEvent::Closed(format!("Closed by server {reason}")));
                }
                Ok(_) => {}
                Err(tungstenite::Error::Io(e)) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut => {}
                Err(tungstenite::Error::ConnectionClosed) | Err(tungstenite::Error::AlreadyClosed) => return send(WsEvent::Closed("Connection closed".into())),
                Err(e) => return send(WsEvent::Error(e.to_string())),
            }
        }
    });
    WsConn { tx: ctx, rx: erx }
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore]
    fn ws_echo() {
        let s = crate::settings::Settings { proxy_mode: "none".into(), ..Default::default() };
        let c = super::connect("ws://127.0.0.1:8767", vec![("X-Test".into(), "1".into())], &s, || {});
        let mut got = vec![];
        let start = std::time::Instant::now();
        let mut sent = false;
        while start.elapsed().as_secs() < 5 {
            if let Ok(e) = c.rx.recv_timeout(std::time::Duration::from_millis(200)) {
                match e {
                    super::WsEvent::Connected(_) => {
                        c.tx.send(super::WsCmd::Text("hello".into())).unwrap();
                        sent = true;
                    }
                    super::WsEvent::Message { text, incoming: true, .. } => {
                        got.push(text);
                        break;
                    }
                    super::WsEvent::Error(e) => panic!("{e}"),
                    _ => {}
                }
            }
        }
        assert!(sent);
        assert_eq!(got, vec!["echo:hello".to_string()]);
    }
}
