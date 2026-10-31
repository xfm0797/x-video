//! Spider 探针：拉取引擎/站点脚本并调用指定方法，用于调试站点兼容性
//!
//! 用法：cargo run -p xv-core --example spider_probe -- <engine_url> <rule_url> <method> [args]

use anyhow::{anyhow, Result};

fn fetch(u: &str) -> Result<String> {
    // 支持本地文件，便于调试自写脚本
    if !u.starts_with("http") {
        return Ok(std::fs::read_to_string(u)?);
    }
    let c = reqwest::blocking::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/126.0")
        .timeout(std::time::Duration::from_secs(60))
        .build()?;
    let r = c.get(u).send()?;
    if !r.status().is_success() {
        return Err(anyhow!("HTTP {}", r.status()));
    }
    Ok(r.text()?)
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        eprintln!("用法: spider_probe <engine_url|-> <rule_url> <method> [args]");
        std::process::exit(2);
    }
    let engine = if args[1] == "-" {
        String::new()
    } else {
        fetch(&args[1])?
    };
    let rule = fetch(&args[2])?;
    let method = &args[3];
    let call_args = args.get(4).cloned().unwrap_or_else(|| "false".to_string());

    println!("引擎 {} 字节，规则 {} 字节", engine.len(), rule.len());
    let js = format!("{engine}\n{rule}");
    let base = args[2].clone();

    let start = std::time::Instant::now();
    match xv_core::sources::spider::call(&js, &base, method, &call_args) {
        Ok(out) => {
            println!("耗时 {:.2}s", start.elapsed().as_secs_f32());
            let head: String = out.chars().take(1200).collect();
            println!("{head}");
        }
        Err(e) => println!("失败（{:.2}s）: {e:#}", start.elapsed().as_secs_f32()),
    }
    Ok(())
}
