fn main() -> anyhow::Result<()> {
    let port: u16 = std::env::var("XVIDEO_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(21600);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(xv_server::serve(port))
}
