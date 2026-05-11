mod common;

#[tokio::main]
async fn main() -> common::ExampleResult<()> {
    common::run_endpoint_surface().await
}
