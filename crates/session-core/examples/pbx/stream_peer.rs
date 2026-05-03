mod common;

#[tokio::main]
async fn main() -> common::ExampleResult<()> {
    common::run_stream_peer_surface().await
}
