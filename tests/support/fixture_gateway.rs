//! A stand-in for his model gateway on a private loopback port. Tests that start
//! a Q-ROC generation open their store on it, so no test can reach the live
//! gateway on 8080 or 8092. Every request is answered "unavailable" and counted.
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

/// Start the stand-in; returns its host:port and the count of requests it answered.
pub fn start() -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = listener.local_addr().unwrap().to_string();
    let answered = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&answered);
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
            let mut chunk = [0_u8; 8192];
            let _ = stream.read(&mut chunk);
            let body = b"{\"error\":\"fixture gateway: no model\"}";
            let _ = write!(
                stream,
                "HTTP/1.1 503 Service Unavailable\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(body);
            count.fetch_add(1, Ordering::SeqCst);
        }
    });
    (endpoint, answered)
}
