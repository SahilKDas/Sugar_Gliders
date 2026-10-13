#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    maple_server::run(maple_server::data_directory()).await
}
