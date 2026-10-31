//! TVBox `type=3` JS Spider 沙箱
//!
//! 用纯 Rust 的 Boa 引擎执行站点脚本，暴露 TVBox 约定的宿主 API：
//! `req` / `joinUrl` / `pdfh` / `pdfa` / `base64*` / `md5` / `log` / `local`。
//!
//! 脚本在独立阻塞线程里跑（调用方用 `spawn_blocking`），引擎实例每次调用新建，
//! 脚本报错不会拖垮主进程。

use anyhow::{anyhow, Result};
use base64::Engine as _;
use boa_engine::{js_string, Context, JsValue, NativeFunction, Source};
use scraper::{Html, Selector};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// 单次脚本调用的网络超时
const REQ_TIMEOUT_SECS: u64 = 20;

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Safari/537.36";

/// 宿主上下文：脚本内所有宿主 API 共用
struct Host {
    client: reqwest::blocking::Client,
    local: RefCell<HashMap<String, String>>,
    logs: RefCell<Vec<String>>,
}

thread_local! {
    static HOST: RefCell<Option<Rc<Host>>> = const { RefCell::new(None) };
}

fn with_host<T>(f: impl FnOnce(&Host) -> T) -> Option<T> {
    HOST.with(|h| h.borrow().as_ref().map(|host| f(host)))
}

/// 执行站点脚本中的一个方法，返回 JSON 字符串
///
/// - `js`：站点脚本（含引擎脚本）拼接后的源码
/// - `method`：`homeContent` / `categoryContent` / `detailContent` / `searchContent` / `playerContent`
/// - `args`：调用参数的 JS 字面量，如 `false`、`"1",1,false,{}`、`["123"]`
pub fn call(js: &str, base_url: &str, method: &str, args: &str) -> Result<String> {
    call_with_env(js, base_url, "", method, args)
}

/// 带环境变量（`ext` / `key`）的调用
pub fn call_with_env(
    js: &str,
    base_url: &str,
    env_js: &str,
    method: &str,
    args: &str,
) -> Result<String> {
    let client = reqwest::blocking::Client::builder()
        .user_agent(UA)
        .timeout(std::time::Duration::from_secs(REQ_TIMEOUT_SECS))
        .redirect(reqwest::redirect::Policy::limited(8))
        .build()
        .map_err(|e| anyhow!("构建 HTTP 客户端失败: {e}"))?;

    let host = Rc::new(Host {
        client,
        local: RefCell::new(HashMap::new()),
        logs: RefCell::new(Vec::new()),
    });

    HOST.with(|h| *h.borrow_mut() = Some(host));

    // 把站点地址暴露给脚本（TVBox 的 HOST 约定）
    let env = format!(
        "var HOST = {};\n{env_js}",
        serde_json::to_string(base_url).unwrap_or_else(|_| "\"\"".into())
    );
    let result = run(js, &env, method, args);

    HOST.with(|h| *h.borrow_mut() = None);

    result
}

fn run(js: &str, env_js: &str, method: &str, args: &str) -> Result<String> {
    let mut ctx = Context::default();
    register_host_api(&mut ctx)?;

    if !env_js.trim().is_empty() {
        ctx.eval(Source::from_bytes(env_js))
            .map_err(|e| anyhow!("站点环境变量注入失败: {e}"))?;
    }

    ctx.eval(Source::from_bytes(js))
        .map_err(|e| anyhow!("站点脚本执行失败: {e}"))?;

    let probe = format!("typeof {method} === 'function'");
    let has_fn = ctx
        .eval(Source::from_bytes(probe.as_str()))
        .map_err(|e| anyhow!("探测方法失败: {e}"))?;
    if !has_fn.to_boolean() {
        return Err(anyhow!("站点脚本未实现 {method}"));
    }

    let expr = format!("JSON.stringify((function(){{ return {method}({args}); }})())");
    let val = ctx
        .eval(Source::from_bytes(expr.as_str()))
        .map_err(|e| anyhow!("{method} 调用失败: {e}"))?;

    if val.is_undefined() || val.is_null() {
        return Err(anyhow!("{method} 返回空结果"));
    }
    let text = val
        .to_string(&mut ctx)
        .map_err(|e| anyhow!("结果序列化失败: {e}"))?
        .to_std_string_escaped();
    Ok(text)
}

/// 取第 i 个参数并转成字符串
fn arg_str(args: &[JsValue], i: usize, ctx: &mut Context) -> String {
    match args.get(i) {
        Some(v) => v
            .to_string(ctx)
            .map(|s| s.to_std_string_escaped())
            .unwrap_or_default(),
        None => String::new(),
    }
}

fn register_host_api(ctx: &mut Context) -> Result<()> {
    // req(url, options) -> string
    let req_fn = NativeFunction::from_fn_ptr(|_this, args, ctx| {
        let url = arg_str(args, 0, ctx);
        let opts = args.get(1).cloned().unwrap_or(JsValue::undefined());
        let out = js_req(&url, &opts, ctx);
        Ok(js_string!(out).into())
    });
    define(ctx, "req", req_fn)?;

    // fetch 与 req 语义一致
    let fetch_fn = NativeFunction::from_fn_ptr(|_this, args, ctx| {
        let url = arg_str(args, 0, ctx);
        let opts = args.get(1).cloned().unwrap_or(JsValue::undefined());
        let out = js_req(&url, &opts, ctx);
        Ok(js_string!(out).into())
    });
    define(ctx, "fetch", fetch_fn)?;

    let join = NativeFunction::from_fn_ptr(|_this, args, ctx| {
        let a = arg_str(args, 0, ctx);
        let b = arg_str(args, 1, ctx);
        Ok(js_string!(join_url(&a, &b)).into())
    });
    define(ctx, "joinUrl", join)?;

    let pdfh = NativeFunction::from_fn_ptr(|_this, args, ctx| {
        let html = arg_str(args, 0, ctx);
        let rule = arg_str(args, 1, ctx);
        Ok(js_string!(pdfh_impl(&html, &rule)).into())
    });
    define(ctx, "pdfh", pdfh)?;

    let pdfa = NativeFunction::from_fn_ptr(|_this, args, ctx| {
        let html = arg_str(args, 0, ctx);
        let rule = arg_str(args, 1, ctx);
        let items = pdfa_impl(&html, &rule);
        let arr = boa_engine::object::builtins::JsArray::new(ctx);
        for it in items {
            let _ = arr.push(js_string!(it), ctx);
        }
        Ok(arr.into())
    });
    define(ctx, "pdfa", pdfa)?;

    let b64e = NativeFunction::from_fn_ptr(|_this, args, ctx| {
        let s = arg_str(args, 0, ctx);
        let out = base64::engine::general_purpose::STANDARD.encode(s.as_bytes());
        Ok(js_string!(out).into())
    });
    define(ctx, "base64Encode", b64e)?;

    let b64d = NativeFunction::from_fn_ptr(|_this, args, ctx| {
        let s = arg_str(args, 0, ctx);
        let out = base64::engine::general_purpose::STANDARD
            .decode(s.as_bytes())
            .ok()
            .and_then(|b| String::from_utf8(b).ok())
            .unwrap_or_default();
        Ok(js_string!(out).into())
    });
    define(ctx, "base64Decode", b64d)?;

    let md5_fn = NativeFunction::from_fn_ptr(|_this, args, ctx| {
        use md5::{Digest, Md5};
        let s = arg_str(args, 0, ctx);
        let mut h = Md5::new();
        h.update(s.as_bytes());
        Ok(js_string!(format!("{:x}", h.finalize())).into())
    });
    define(ctx, "md5", md5_fn)?;

    let log_fn = NativeFunction::from_fn_ptr(|_this, args, ctx| {
        let msg = arg_str(args, 0, ctx);
        with_host(|h| {
            let mut logs = h.logs.borrow_mut();
            if logs.len() < 200 {
                logs.push(msg);
            }
        });
        Ok(JsValue::undefined())
    });
    define(ctx, "log", log_fn.clone())?;
    define(ctx, "print", log_fn)?;

    let get_item = NativeFunction::from_fn_ptr(|_this, args, ctx| {
        let key = arg_str(args, 0, ctx);
        let dflt = arg_str(args, 1, ctx);
        let got = with_host(|h| h.local.borrow().get(&key).cloned()).flatten();
        Ok(js_string!(got.unwrap_or(dflt)).into())
    });
    define(ctx, "getItem", get_item)?;

    let set_item = NativeFunction::from_fn_ptr(|_this, args, ctx| {
        let key = arg_str(args, 0, ctx);
        let val = arg_str(args, 1, ctx);
        with_host(|h| {
            h.local.borrow_mut().insert(key, val);
        });
        Ok(JsValue::undefined())
    });
    define(ctx, "setItem", set_item)?;

    let noop_true = NativeFunction::from_fn_ptr(|_this, _args, _ctx| Ok(JsValue::from(true)));
    define(ctx, "getProxy", noop_true)?;

    let prelude = r#"
        var console = {
            log: function(){ try{ log.apply(null, arguments) }catch(e){} },
            error: function(){}, warn: function(){}, info: function(){}, debug: function(){}
        };
        var local = {
            get: function(k,d){ return getItem(k, d===undefined?'':d) },
            set: function(k,v){ return setItem(k, v===undefined?'':v) }
        };
        var MOBILE_UA = 'Mozilla/5.0 (Linux; Android 12; Pixel 5) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Mobile Safari/537.36';
        var PC_UA = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Safari/537.36';
        var UC_UA = MOBILE_UA;
    "#;
    ctx.eval(Source::from_bytes(prelude))
        .map_err(|e| anyhow!("宿主预置脚本失败: {e}"))?;

    Ok(())
}

fn define(ctx: &mut Context, name: &str, f: NativeFunction) -> Result<()> {
    let realm = ctx.realm().clone();
    let obj = f.to_js_function(&realm);
    ctx.global_object()
        .set(js_string!(name), obj, true, ctx)
        .map_err(|e| anyhow!("注册宿主函数 {name} 失败: {e}"))?;
    Ok(())
}

// ---------- 宿主函数实现 ----------

fn js_req(url: &str, opts: &JsValue, ctx: &mut Context) -> String {
    if url.is_empty() {
        return String::new();
    }
    let (method, body, headers) = parse_req_opts(opts, ctx);
    let client = match with_host(|h| h.client.clone()) {
        Some(c) => c,
        None => return String::new(),
    };
    let mut rb = if method.eq_ignore_ascii_case("post") {
        client.post(url)
    } else {
        client.get(url)
    };
    for (k, v) in headers {
        rb = rb.header(k, v);
    }
    if !body.is_empty() {
        rb = rb.body(body);
    }
    match rb.send() {
        Ok(resp) => match resp.text() {
            Ok(t) => t,
            Err(e) => {
                with_host(|h| h.logs.borrow_mut().push(format!("req 读取失败: {e}")));
                String::new()
            }
        },
        Err(e) => {
            with_host(|h| h.logs.borrow_mut().push(format!("req 失败 {url}: {e}")));
            String::new()
        }
    }
}

fn parse_req_opts(opts: &JsValue, ctx: &mut Context) -> (String, String, Vec<(String, String)>) {
    let mut method = "GET".to_string();
    let mut body = String::new();
    let mut headers = Vec::new();

    let Some(obj) = opts.as_object() else {
        return (method, body, headers);
    };
    let get_prop = |obj: &boa_engine::JsObject, key: &str, ctx: &mut Context| -> Option<String> {
        let v = obj.get(js_string!(key), ctx).ok()?;
        if v.is_undefined() || v.is_null() {
            return None;
        }
        v.to_string(ctx).ok().map(|s| s.to_std_string_escaped())
    };

    if let Some(m) = get_prop(obj, "method", ctx) {
        method = m.to_uppercase();
    }
    if let Some(b) = get_prop(obj, "body", ctx) {
        body = b;
    }
    if let Ok(hv) = obj.get(js_string!("headers"), ctx) {
        if let Some(hobj) = hv.as_object() {
            if let Ok(keys) = hobj.own_property_keys(ctx) {
                for k in keys {
                    let key = match k {
                        boa_engine::property::PropertyKey::String(s) => s.to_std_string_escaped(),
                        other => other.to_string().to_string(),
                    };
                    if key.is_empty() {
                        continue;
                    }
                    if let Some(val) = get_prop(hobj, &key, ctx) {
                        headers.push((key, val));
                    }
                }
            }
        }
    }
    (method, body, headers)
}

/// 相对地址拼接（TVBox 的 joinUrl 语义：base 为站点地址）
pub fn join_url(base: &str, rel: &str) -> String {
    if rel.starts_with("http://") || rel.starts_with("https://") {
        return rel.to_string();
    }
    if base.is_empty() {
        return rel.to_string();
    }
    if rel.starts_with('/') {
        if let Some(scheme_end) = base.find("://") {
            let rest = &base[scheme_end + 3..];
            let host_end = rest.find('/').unwrap_or(rest.len());
            return format!("{}{}", &base[..scheme_end + 3 + host_end], rel);
        }
        return rel.to_string();
    }
    let base_no_query = base.split(['?', '#']).next().unwrap_or(base);
    match base_no_query.rfind('/') {
        Some(i) => format!("{}/{}", &base_no_query[..i], rel),
        None => format!("{base_no_query}/{rel}"),
    }
}

// ---------- pdfh / pdfa ----------

fn select_all(html: &str, selector: &str) -> Vec<String> {
    let (sel, idx) = split_eq(selector);
    let doc = Html::parse_document(html);
    let sel = match Selector::parse(&sel) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let mut out: Vec<String> = doc.select(&sel).map(|e| e.html()).collect();
    if let Some(i) = idx {
        if i < out.len() {
            return vec![out.remove(i)];
        }
        return Vec::new();
    }
    out
}

/// 处理 jQuery 风格的 `:eq(n)`
fn split_eq(selector: &str) -> (String, Option<usize>) {
    if let Some(pos) = selector.rfind(":eq(") {
        let rest = &selector[pos + 4..];
        if let Some(end) = rest.find(')') {
            if let Ok(n) = rest[..end].trim().parse::<usize>() {
                return (selector[..pos].to_string(), Some(n));
            }
        }
    }
    (selector.to_string(), None)
}

/// `pdfa`：返回所有匹配元素的 outer HTML
pub fn pdfa_impl(html: &str, rule: &str) -> Vec<String> {
    let rule = rule.trim();
    if rule.is_empty() || html.trim().is_empty() {
        return Vec::new();
    }
    let parts: Vec<&str> = rule.split("&&").map(|s| s.trim()).collect();
    select_all(html, parts[0])
}

/// `pdfh`：按 `selector&&selector&&Text|Html|attr` 链式取值
pub fn pdfh_impl(html: &str, rule: &str) -> String {
    let rule = rule.trim();
    if rule.is_empty() || html.trim().is_empty() {
        return String::new();
    }
    if rule.starts_with("$.") {
        return json_path(html, rule);
    }

    let parts: Vec<&str> = rule.split("&&").map(|s| s.trim()).collect();
    let mut current: Vec<String> = select_all(html, parts[0]);
    if current.is_empty() {
        return String::new();
    }

    for (i, part) in parts.iter().enumerate().skip(1) {
        let last = i == parts.len() - 1;
        if last {
            return extract(current.first().map(|s| s.as_str()).unwrap_or(""), part);
        }
        let mut next = Vec::new();
        for c in &current {
            next.extend(select_all(c, part));
        }
        if next.is_empty() {
            return String::new();
        }
        current = next;
    }

    current.first().cloned().unwrap_or_default()
}

fn extract(html: &str, part: &str) -> String {
    let doc = Html::parse_fragment(html);
    let root = doc.root_element();
    match part {
        "Text" => root.text().collect::<String>().trim().to_string(),
        "Html" => root.inner_html(),
        attr => {
            // 片段解析会套一层 html/body，属性要从内层元素取
            let sel = Selector::parse("*").expect("通配选择器");
            let el = root.select(&sel).next().unwrap_or(root);
            el.value().attr(attr).unwrap_or("").to_string()
        }
    }
}

fn json_path(html: &str, rule: &str) -> String {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(html) else {
        return String::new();
    };
    let mut cur = &v;
    for seg in rule.trim_start_matches("$.").split('.') {
        if seg.is_empty() {
            continue;
        }
        match cur.get(seg) {
            Some(n) => cur = n,
            None => return String::new(),
        }
    }
    match cur {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_join_url() {
        assert_eq!(join_url("https://a.com/x/y", "/p/q"), "https://a.com/p/q");
        assert_eq!(join_url("https://a.com/x/y", "z"), "https://a.com/x/z");
        assert_eq!(
            join_url("https://a.com/x/", "https://b.com/1"),
            "https://b.com/1"
        );
    }

    #[test]
    fn test_pdfh_pdfa() {
        let html = r#"<div class="list">
            <a class="item" href="/v/1">影片一</a>
            <a class="item" href="/v/2">影片二</a>
        </div>"#;
        assert_eq!(pdfa_impl(html, "a.item").len(), 2);
        assert_eq!(pdfh_impl(html, "a.item&&Text"), "影片一");
        assert_eq!(pdfh_impl(html, "a.item:eq(1)&&Text"), "影片二");
        assert_eq!(pdfh_impl(html, "a.item&&href"), "/v/1");
    }

    #[test]
    fn test_spider_call_home() {
        let js = r#"
            var rule = { title: '测试站' };
            function homeContent(filter) {
                return { class: [{ type_id: '1', type_name: '电影' }], list: [] };
            }
            function categoryContent(tid, pg, filter, extend) {
                return { list: [{ vod_id: tid + '-' + pg, vod_name: '片' + pg }], page: pg, pagecount: 9 };
            }
        "#;
        let out = call(js, "https://demo.test/", "homeContent", "false").unwrap();
        assert!(out.contains("电影"), "实际: {out}");
        let out2 = call(js, "https://demo.test/", "categoryContent", r#""7",2,false,{}"#).unwrap();
        assert!(out2.contains("7-2"), "实际: {out2}");
    }

    #[test]
    fn test_spider_missing_method() {
        let js = "var a = 1;";
        let err = call(js, "https://demo.test/", "homeContent", "false").unwrap_err();
        assert!(err.to_string().contains("未实现"), "实际: {err}");
    }
}
