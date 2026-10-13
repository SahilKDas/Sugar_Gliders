#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    maple_server::run(std::path::PathBuf::from("data")).await
}
