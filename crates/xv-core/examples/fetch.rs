use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let url = std::env::args().nth(1).unwrap_or_else(|| "https://example.com".into());
    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0")
        .timeout(std::time::Duration::from_secs(30))
        .no_proxy()
        .build()?;
    let resp = client.get(&url).send().await?;
    println!("url: {url}");
    println!("status: {}", resp.status());
    println!("content-type: {:?}", resp.headers().get("content-type"));
    println!("content-encoding: {:?}", resp.headers().get("content-encoding"));
    match resp.bytes().await {
        Ok(b) => println!("bytes ok: {}", b.len()),
        Err(e) => {
            println!("bytes err: {e}");
            let mut src = std::error::Error::source(&e);
            while let Some(s) = src {
                println!("  caused by: {s}");
                src = s.source();
            }
        }
    }
    Ok(())
}
