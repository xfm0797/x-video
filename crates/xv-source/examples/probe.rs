//! 手动验证订阅解析链路：
//!   cargo run -p xv-source --example probe -- <订阅地址> [搜索词]

use xv_source::{http, live, tvbox};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let url = args.next().expect("用法：probe <订阅地址> [搜索词]");
    let word = args.next().unwrap_or_else(|| "庆余年".to_string());
    let client = http::client();

    let fetched = http::fetch_text(&client, &url, None).await?;
    println!("拉取 {} 字节 ← {}", fetched.text.len(), fetched.final_url);

    if fetched.text.trim_start().starts_with('{') {
        let cfg = tvbox::parse_config(&fetched.text)?;
        let supported = cfg.sites.iter().filter(|s| s.supported()).count();
        let searchable = cfg
            .sites
            .iter()
            .filter(|s| s.supported() && s.searchable())
            .count();
        println!(
            "TVBox 配置：{} 站点（可直接用 {supported}，可搜索 {searchable}），{} 组直播，{} 个解析器",
            cfg.sites.len(),
            cfg.lives.len(),
            cfg.parses.len()
        );

        let mut sources: Vec<(String, String)> = Vec::new();
        for entry in &cfg.lives {
            for piece in entry.url.split('#') {
                let real = live::unwrap_proxy(piece.trim());
                let abs = http::join(&url, &real);
                if abs.contains("://") {
                    sources.push((entry.name.clone(), abs));
                }
            }
        }
        if !sources.is_empty() {
            let groups = live::load(&client, &sources, None).await;
            let n: usize = groups.iter().map(|g| g.channels.len()).sum();
            println!("直播：{} 组 / {} 频道", groups.len(), n);
            for g in groups.iter().take(3) {
                println!("  · {}（{}）", g.name, g.channels.len());
            }
        }

        if let Some(site) = cfg.sites.iter().find(|s| s.supported() && s.searchable()) {
            println!("\n在「{}」搜索「{word}」…", site.name);
            match tvbox::search(&client, site, &word).await {
                Ok(hits) => {
                    println!("命中 {} 条", hits.len());
                    for h in hits.iter().take(5) {
                        println!("  · {} {} {}", h.name, h.remarks.clone().unwrap_or_default(), h.id);
                    }
                    if let Some(first) = hits.first() {
                        let d = tvbox::detail(&client, site, &first.id).await?;
                        println!(
                            "详情：{}（{} 个播放源）",
                            d.name,
                            d.sources.len()
                        );
                        if let Some(src) = d.sources.first() {
                            println!(
                                "  线路 {} 共 {} 集，首集 {}",
                                src.from,
                                src.episodes.len(),
                                src.episodes[0].url
                            );
                            match tvbox::resolve(&client, site, &cfg.parses, &src.from, &src.episodes[0].url).await {
                                Ok(r) => println!("  解析结果：{}", r.url),
                                Err(e) => println!("  解析失败：{e:#}"),
                            }
                        }
                    }
                }
                Err(e) => println!("搜索失败：{e:#}"),
            }
        }
    } else {
        let groups = live::parse_auto(&fetched.text);
        let n: usize = groups.iter().map(|g| g.channels.len()).sum();
        println!("直播清单：{} 组 / {} 频道", groups.len(), n);
        for g in groups.iter().take(6) {
            println!(
                "  · {}（{}）示例：{}",
                g.name,
                g.channels.len(),
                g.channels.first().map(|c| c.name.clone()).unwrap_or_default()
            );
        }
    }
    Ok(())
}
