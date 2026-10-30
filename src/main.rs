#[tokio::main]
async fn main() {
    de_storage_server::cli::run().await;
}
