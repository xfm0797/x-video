//! 验证 JS spider（drpy 规则）链路：
//!   cargo run -p xv-source --example spider -- <TVBox配置地址> [站点关键词] [搜索词]

use xv_source::{http, spider::Spy, tvbox};

fn abs_maybe(base: &str, value: &str) -> String {
    let v = value.trim();
    if v.is_empty() || v.contains("://") {
        return v.to_string();
    }
    if v.starts_with("csp_") || v.starts_with("js:") || v.starts_with("py:") {
        return v.to_string();
    }
    if v.starts_with('.') || v.starts_with('/') || v.contains(".js") || v.contains(".json") {
        return http::join(base, v);
    }
    v.to_string()
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let cfg_url = args.next().expect("用法：spider <配置地址> [站点关键词] [搜索词]");
    let want = args.next().unwrap_or_else(|| "磁力".to_string());
    let word = args.next().unwrap_or_else(|| "复仇".to_string());

    let client = http::client();
    let fetched = http::fetch_text(&client, &cfg_url, None).await?;
    let mut cfg = tvbox::parse_config(&fetched.text)?;
    for site in &mut cfg.sites {
        site.api = abs_maybe(&cfg_url, &site.api);
        if let Some(s) = site.ext.as_str() {
            site.ext = serde_json::Value::String(abs_maybe(&cfg_url, s));
        }
    }

    let js: Vec<&tvbox::Site> = cfg.sites.iter().filter(|s| s.script_url().is_some()).collect();
    let java: Vec<&tvbox::Site> = cfg.sites.iter().filter(|s| s.needs_java_spider()).collect();
    println!(
        "站点 {} 个：JS 规则 {}，Java 内核 {}",
        cfg.sites.len(),
        js.len(),
        java.len()
    );

    let Some(site) = js.iter().find(|s| s.name.contains(&want)).or_else(|| js.first()) else {
        println!("没有找到 JS 规则站点");
        return Ok(());
    };
    println!("\n站点：{}（{}）", site.name, site.key);
    let script_url = site.script_url().unwrap();
    println!("脚本：{script_url}");

    let fetched = http::fetch_text(&client, &script_url, Some(&script_url)).await?;
    let spy = match Spy::load(&fetched.text, &fetched.final_url, &site.headers()) {
        Ok(s) => s,
        Err(e) => {
            println!("规则加载失败：{e:#}");
            return Ok(());
        }
    };
    println!(
        "规则：title={:?} host={:?}\n  分类模板={:?}\n  搜索模板={:?}",
        spy.rule.title, spy.rule.host, spy.rule.url, spy.rule.search_url
    );

    let site_for_detail = (*site).clone();
    let spy_for_search = spy.clone();
    let w = word.clone();
    let hits = tokio::task::spawn_blocking(move || spy_for_search.search(&w, 1)).await??;
    println!("\n搜索「{word}」命中 {} 条", hits.len());
    for h in hits.iter().take(5) {
        println!(
            "  · {} | {} | {}",
            h.name,
            h.remarks.clone().unwrap_or_default(),
            h.id.chars().take(70).collect::<String>()
        );
    }
    let Some(first) = hits.first().cloned() else {
        println!("没有命中，收工");
        return Ok(());
    };

    let spy2 = spy.clone();
    let id = first.id.clone();
    let d = tokio::task::spawn_blocking(move || spy2.detail(&id)).await??;
    println!(
        "\n详情：{}（{} 个线路）{}",
        d.name,
        d.sources.len(),
        d.year.unwrap_or_default()
    );
    for s in d.sources.iter().take(3) {
        println!(
            "  线路 {}：{} 集，首集 {}",
            s.from,
            s.episodes.len(),
            s.episodes.first().map(|e| e.url.clone()).unwrap_or_default()
        );
    }
    if let Some(ep) = d.sources.first().and_then(|s| s.episodes.first()) {
        match tvbox::resolve(&client, &site_for_detail, &cfg.parses, "", &ep.url).await {
            Ok(r) => println!("\n解析结果：{}", r.url),
            Err(e) => println!("\n解析失败：{e:#}"),
        }
    }
    Ok(())
}
