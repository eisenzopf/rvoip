mod common;

#[tokio::main]
async fn main() -> common::ExampleResult<()> {
    common::run_callback_builder_surface().await
}
