//! A minimal HTTP/1.1 responder for `GET /metrics`.
//!
//! One request per connection, answered and closed. Anything other than
//! `GET /metrics` gets a 404; a request that does not arrive within
//! [`REQUEST_TIMEOUT`] or exceeds [`MAX_REQUEST_BYTES`] is dropped.

use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// Longest wait for a request's headers.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
/// Largest request head accepted.
pub const MAX_REQUEST_BYTES: usize = 8 * 1024;

const CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

/// Bind `listen` and serve `/metrics` with the text `render` produces for
/// each scrape. Returns the bound address.
pub async fn serve<F, Fut>(listen: SocketAddr, render: F) -> std::io::Result<SocketAddr>
where
    F: Fn() -> Fut + Send + Sync + 'static,
    Fut: Future<Output = String> + Send + 'static,
{
    let listener = TcpListener::bind(listen).await?;
    let local = listener.local_addr()?;
    let render = Arc::new(render);
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                tokio::time::sleep(Duration::from_millis(10)).await;
                continue;
            };
            let render = Arc::clone(&render);
            tokio::spawn(async move {
                let _ = tokio::time::timeout(REQUEST_TIMEOUT, respond(stream, render)).await;
            });
        }
    });
    Ok(local)
}

async fn respond<F, Fut>(mut stream: TcpStream, render: Arc<F>) -> std::io::Result<()>
where
    F: Fn() -> Fut,
    Fut: Future<Output = String>,
{
    let mut head = Vec::with_capacity(512);
    let mut chunk = [0u8; 1024];
    while !head.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = stream.read(&mut chunk).await?;
        if read == 0 || head.len() + read > MAX_REQUEST_BYTES {
            return Ok(());
        }
        head.extend_from_slice(&chunk[..read]);
    }
    let response = match route(&head) {
        Route::Metrics => {
            let body = render().await;
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {CONTENT_TYPE}\r\nContent-Length: {}\r\n\
                 Connection: close\r\n\r\n{body}",
                body.len()
            )
        }
        Route::NotFound => {
            "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string()
        }
    };
    stream.write_all(response.as_bytes()).await?;
    stream.shutdown().await
}

#[derive(Debug, PartialEq, Eq)]
enum Route {
    Metrics,
    NotFound,
}

fn route(head: &[u8]) -> Route {
    let line = head.split(|byte| *byte == b'\n').next().unwrap_or_default();
    let mut parts = line.split(|byte| *byte == b' ');
    let (method, target) = (parts.next(), parts.next());
    let path = target.map(|target| target.split(|byte| *byte == b'?').next().unwrap_or(target));
    match (method, path) {
        (Some(b"GET"), Some(b"/metrics")) => Route::Metrics,
        _ => Route::NotFound,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_get_metrics_is_served() {
        assert_eq!(route(b"GET /metrics HTTP/1.1\r\n\r\n"), Route::Metrics);
        assert_eq!(route(b"GET /metrics?x=1 HTTP/1.1\r\n\r\n"), Route::Metrics);
        assert_eq!(route(b"POST /metrics HTTP/1.1\r\n\r\n"), Route::NotFound);
        assert_eq!(route(b"GET / HTTP/1.1\r\n\r\n"), Route::NotFound);
        assert_eq!(route(b"GET /metricsx HTTP/1.1\r\n\r\n"), Route::NotFound);
        assert_eq!(route(b""), Route::NotFound);
    }

    async fn fetch(address: SocketAddr, request: &str) -> String {
        let mut stream = TcpStream::connect(address).await.unwrap();
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).await.unwrap();
        response
    }

    #[tokio::test]
    async fn serves_the_rendered_text_and_404s_the_rest() {
        let address = serve("127.0.0.1:0".parse().unwrap(), || async {
            "rvoip_bench_uas_active_calls 7\n".to_string()
        })
        .await
        .unwrap();
        assert_ne!(address.port(), 0);
        let ok = fetch(address, "GET /metrics HTTP/1.1\r\nHost: dut\r\n\r\n").await;
        assert!(ok.starts_with("HTTP/1.1 200 OK\r\n"), "{ok}");
        assert!(
            ok.contains("Content-Type: text/plain; version=0.0.4"),
            "{ok}"
        );
        assert!(ok.contains("Content-Length: 31\r\n"), "{ok}");
        assert!(
            ok.ends_with("\r\n\r\nrvoip_bench_uas_active_calls 7\n"),
            "{ok}"
        );
        let missing = fetch(address, "GET /healthz HTTP/1.1\r\n\r\n").await;
        assert!(
            missing.starts_with("HTTP/1.1 404 Not Found\r\n"),
            "{missing}"
        );
    }
}
