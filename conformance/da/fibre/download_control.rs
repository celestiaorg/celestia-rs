use celestia_fibre::{BlobID, DownloadOptions, FibreClient, FibreClientConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let client = FibreClient::from_endpoint(args[1].as_str(), FibreClientConfig::new(&args[2])?)?;
    let id = BlobID::from_bytes(&hex::decode(&args[3])?)?;
    let blob = client.download(&id, DownloadOptions::default()).await?;
    std::fs::write(&args[4], blob.data())?;
    Ok(())
}
