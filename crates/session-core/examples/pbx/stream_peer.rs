mod common;

#[tokio::main]
async fn main() -> common::ExampleResult<()> {
    common::run_streampeer_surface().await
}
