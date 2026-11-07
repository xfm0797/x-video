//! TVBox `type=3` 的 spider：在进程内跑 JS 规则脚本。
//!
//! 一条 JS 规则（`ext` 指向 `js/xxx.js`，内容通常是 `var rule = {...}`）里有两类写法：
//!
//! 1. **选择器 DSL**：`一级`/`搜索` 写成 `容器;标题;图片;简介;链接`，
//!    `二级` 写成一个字段对象，每个字段是选择器（`;` 分隔多个候选）。
//! 2. **代码片段**：以 `js:` 开头，里面可以调 `request`/`pdfh`/`pdfa`/`jsp`。
//!
//! 这里两种都实现：选择器走本地的 [`crate::pdf`]，代码片段用 rquickjs 执行，
//! 并补齐 drpy 常用的宿主函数（`request`/`post`/`pdfh`/`pdfa`/`jsp`/`urlencode`…）。
//!
//! `csp_xxx` 这类 Java spider（需要 `pg.jar`）跑不了，会明确标成不支持。

use std::time::{Duration, Instant};

use anyhow::{anyhow, Result};
use rquickjs::{Context, Function, Runtime};
use serde_json::{json, Value};

use crate::http;
use crate::pdf;
use crate::tvbox::{Episode, Hit, PlaySource, VodDetail};

/// 规则脚本里能读到的字段。`*_raw` 保留原始写法（可能带 `js:` 前缀）。
#[derive(Debug, Clone, Default)]
pub struct Rule {
    pub title: String,
    pub host: String,
    /// 分类页模板，含 `fyclass` / `fypage`
    pub url: String,
    /// 搜索页模板，含 `**`（关键词）与 `fypage`
    pub search_url: String,
    pub class_name: String,
    pub class_url: String,
    pub headers: Vec<(String, String)>,
    /// `一级`（列表页）
    pub home_raw: String,
    /// `二级`（详情页）：可能是对象，也可能是代码
    pub detail_raw: Value,
    /// `搜索`
    pub search_raw: String,
    /// `lazy`（播放地址解析）
    pub lazy_raw: String,
    /// `tabs`（线路名，一般是 `js:` 代码）
    pub tabs_raw: String,
}

impl Rule {
    /// 该字段是不是 `js:` 代码（否则按选择器 DSL 处理）。
    fn is_js(raw: &str) -> bool {
        let t = raw.trim_start();
        t.starts_with("js:") || t.starts_with("JS:")
    }

    fn code(raw: &str) -> &str {
        let t = raw.trim_start();
        for p in ["js:", "JS:"] {
            if let Some(rest) = t.strip_prefix(p) {
                return rest.trim_start();
            }
        }
        t
    }
}

/// 一个已经加载好规则的 spider 站点。
#[derive(Debug, Clone)]
pub struct Spy {
    pub rule: Rule,
    pub script: String,
    pub script_url: String,
}

/// 宿主脚本：补齐 drpy 规则里直接用到的全局函数与常量。
const PRELUDE: &str = r#"
var __RESULT = null;
var TABS = [];
var LISTS = [];
var VERSION = "2.0.0";
var RKEY = "";
var MOBILE_UA = "Mozilla/5.0 (Linux; Android 10; Mobile) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Mobile Safari/537.36";
var PC_UA = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Safari/537.36";
function setResult(x){ __RESULT = x; }
function setHomeResult(x){ __RESULT = x; }
function log(s){ try{ __host("log", String(s)); }catch(e){} }
function print(s){ log(s); }
function urlencode(s){ return encodeURIComponent(String(s)); }
function urldecode(s){ try { return decodeURIComponent(String(s)); } catch(e){ return String(s); } }
function request(url, opt){
  opt = opt || {};
  url = String(url);
  // drpy 用 `地址;post&body` 表示这个请求要走 POST
  var at = url.indexOf(";post");
  if (at >= 0) {
    var tail = url.slice(at + 5);
    if (tail.charAt(0) === "&") tail = tail.slice(1);
    url = url.slice(0, at);
    opt.method = "POST";
    opt.body = tail;
    opt.headers = opt.headers || {};
    opt.headers["Content-Type"] = opt.headers["Content-Type"] || "application/x-www-form-urlencoded";
  }
  var r = JSON.parse(__host("req", JSON.stringify({url: url, opt: opt})));
  if (!r || r.error) throw new Error((r && r.error) || "请求失败");
  if (opt.withHeaders) return r;
  return r.content;
}
function req(url, opt){ return request(url, opt || {}); }
function post(url, params){
  params = params || {};
  params.method = "POST";
  if (!params.body && params.data) params.body = params.data;
  return request(url, params);
}
function postJson(url, params){
  params = params || {};
  params.headers = params.headers || {};
  params.headers["Content-Type"] = params.headers["Content-Type"] || "application/json";
  return post(url, params);
}
function pdfh(html, sel, opt){
  return __host("pdfh", JSON.stringify([html == null ? "" : String(html), sel == null ? "" : String(sel)]));
}
function pdfa(html, sel, opt){
  var s = __host("pdfa", JSON.stringify([html == null ? "" : String(html), sel == null ? "" : String(sel)]));
  try { return JSON.parse(s); } catch(e){ return []; }
}
function pdfx(html, sel, opt){ return pdfh(html, sel, opt); }
function pd(html, sel, base){
  var v = !sel ? String(html == null ? "" : html) : pdfh(html, sel);
  if (base && v && !/^(https?:)?\/\//i.test(v) && v.charAt(0) === "/") return joinUrl(base, v);
  return v;
}
function getProxyUrl(){ return ""; }
function getProxy(){ return false; }
// 有些规则用 fetch(...).then(...) 或 await fetch()，这里用同步「可等待对象」模拟，
// 宿主请求本身是阻塞的，语义上等价于已经完成。
function __awaitable(value){
  return {
    then: function(onOk, onErr){
      try {
        var r = onOk ? onOk(value) : value;
        return __awaitable(r === undefined ? value : r);
      } catch(e) {
        if (onErr) return __awaitable(onErr(e));
        throw e;
      }
    },
    catch: function(onErr){ return this.then(null, onErr); },
    finally: function(fn){ if (fn) fn(); return this; }
  };
}
function fetch(url, opt){
  var r = JSON.parse(__host("req", JSON.stringify({url: String(url), opt: opt || {}})));
  var code = r && r.code || 0;
  var body = (r && r.content) || "";
  return __awaitable({
    ok: code >= 200 && code < 300,
    status: code,
    url: (r && r.url) || String(url),
    text: function(){ return __awaitable(body); },
    json: function(){ try { return __awaitable(JSON.parse(body)); } catch(e){ return __awaitable(null); } }
  });
}
function joinUrl(base, url){
  if (!url) return base || "";
  if (/^https?:/i.test(url)) return url;
  base = String(base || "");
  if (url.charAt(0) === "/") {
    var m = base.match(/^https?:\/\/[^\/]+/i);
    return (m ? m[0] : base) + url;
  }
  if (base.charAt(base.length - 1) !== "/") base += "/";
  return base + url;
}
function __jsonpath(data, path){
  if (!path) return data;
  var p = String(path).replace(/^\$\.?/, "");
  if (!p) return data;
  var parts = p.split(".").filter(function(x){ return x.length; });
  var cur = data;
  for (var i = 0; i < parts.length; i++) {
    if (cur == null) return null;
    var key = parts[i];
    var m = key.match(/^([^\[\]]+)\[(\d+)\]$/);
    if (m) { cur = cur[m[1]]; if (cur != null) cur = cur[Number(m[2])]; }
    else { cur = cur[key]; }
  }
  return cur == null ? null : cur;
}
// drpy 里 jsp 是个命名空间对象，规则里常写 pdfh = jsp.pdfh;
var jsp = {
  pdfh: function(h, s, o){ return pdfh(h, s, o); },
  pdfa: function(h, s, o){ return pdfa(h, s, o); },
  pd: function(h, s, b){ return pd(h, s, b); },
  jsp: __jsonpath,
  joinUrl: joinUrl
};
var __STORE = {};
function setItem(k, v){ __STORE[String(k)] = String(v); }
function getItem(k){ return __STORE[String(k)] || ""; }
function clearItem(k){ delete __STORE[String(k)]; }
var RULE_CK = "rule_cookie";
var rule_fetch_params = { headers: { Cookie: "" }, method: "GET" };
var fetch_params = { headers: { Cookie: "" } };
"#;

/// 规则脚本里读 `rule` 并转成 JSON 的表达式。
const RULE_JSON: &str = r#"
(function(){
  var r = (typeof rule !== "undefined" && rule) || (typeof RULE !== "undefined" && RULE) || {};
  function pick(){
    for (var i = 0; i < arguments.length; i++) {
      var v = r[arguments[i]];
      if (typeof v === "string" && v.trim()) return v;
    }
    return "";
  }
  function raw(){
    for (var i = 0; i < arguments.length; i++) {
      var v = r[arguments[i]];
      if (v !== undefined && v !== null && v !== "") return v;
    }
    return "";
  }
  return JSON.stringify({
    title: pick("title"),
    host: pick("host"),
    url: pick("url"),
    searchUrl: pick("searchUrl", "search_url"),
    className: pick("class_name"),
    classUrl: pick("class_url"),
    headers: r.headers || r.header || null,
    home: raw("一级", "推荐"),
    detail: raw("二级", "详情"),
    search: raw("搜索"),
    lazy: raw("lazy"),
    tabs: raw("tabs")
  });
})()
"#;

fn js_str(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string())
}

fn header_json(pairs: &[(String, String)]) -> String {
    let mut map = serde_json::Map::new();
    for (k, v) in pairs {
        map.insert(k.clone(), Value::String(v.clone()));
    }
    Value::Object(map).to_string()
}

/// eval 一段 JS 并把异常信息（message + 首行栈）带出来，
/// 否则 rquickjs 只会给一句 "Exception generated by QuickJS"。
fn eval_detail(ctx: &rquickjs::Ctx, code: &str, what: &str) -> Result<String> {
    match ctx.eval::<String, _>(code) {
        Ok(s) => Ok(s),
        Err(e) => {
            let pending = ctx.catch();
            let detail = pending
                .as_exception()
                .map(|ex| {
                    let msg = ex.message().unwrap_or_default();
                    let stack = ex.stack().unwrap_or_default();
                    let line = stack.lines().next().unwrap_or("").trim().to_string();
                    if line.is_empty() {
                        msg
                    } else {
                        format!("{msg}（{line}）")
                    }
                })
                .unwrap_or_else(|| format!("{e}"));
            Err(anyhow!("{what}：{detail}"))
        }
    }
}

fn eval_void(ctx: &rquickjs::Ctx, code: &str, what: &str) -> Result<()> {
    match ctx.eval::<() , _>(code) {
        Ok(()) => Ok(()),
        Err(e) => {
            let pending = ctx.catch();
            let detail = pending
                .as_exception()
                .map(|ex| ex.message().unwrap_or_default())
                .unwrap_or_else(|| format!("{e}"));
            Err(anyhow!("{what}：{detail}"))
        }
    }
}

fn block_on_js<T>(deadline: Duration, f: impl FnOnce(&rquickjs::Ctx) -> Result<T>) -> Result<T> {
    let rt = Runtime::new().map_err(|e| anyhow!("JS 引擎初始化失败：{e}"))?;
    let until = Instant::now() + deadline;
    rt.set_interrupt_handler(Some(Box::new(move || Instant::now() > until)));
    let ctx = Context::full(&rt).map_err(|e| anyhow!("JS 上下文创建失败：{e}"))?;
    ctx.with(|ctx| f(&ctx))
}

// ---------- 网络 ----------

struct Net {
    headers: Vec<(String, String)>,
    referer: String,
}

/// 全局只建一个 blocking 客户端：既不重复建连接池，
/// 也避免它在异步上下文里析构（reqwest::blocking 析构时会建运行时，会 panic）。
fn net_client() -> &'static reqwest::blocking::Client {
    static CLIENT: std::sync::OnceLock<reqwest::blocking::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(20))
            .gzip(true)
            .build()
            .unwrap_or_else(|_| reqwest::blocking::Client::new())
    })
}

impl Net {
    fn new(headers: Vec<(String, String)>, referer: String) -> Self {
        Self { headers, referer }
    }

    fn fetch(&self, payload: &str) -> String {
        let client = net_client();
        let v: Value = serde_json::from_str(payload).unwrap_or(Value::Null);
        let url = v.get("url").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let opt = v.get("opt").cloned().unwrap_or(Value::Null);
        if url.is_empty() {
            return json!({"error": "地址为空"}).to_string();
        }
        let method = opt
            .get("method")
            .and_then(|x| x.as_str())
            .unwrap_or("GET")
            .to_ascii_uppercase();

        let mut req = if method == "POST" {
            client.post(&url)
        } else {
            client.get(&url)
        };
        req = req.header("User-Agent", crate::http::UA);
        for (k, val) in &self.headers {
            // 规则里常写 'MOBILE_UA' 这种占位，交给 JS 侧替换不了，这里按默认 UA 处理
            if val.contains("_UA") {
                continue;
            }
            req = req.header(k.as_str(), val.as_str());
        }
        if let Some(h) = opt.get("headers").and_then(|x| x.as_object()) {
            for (k, val) in h {
                if let Some(s) = val.as_str() {
                    req = req.header(k.as_str(), s);
                }
            }
        }
        let has_ref = self.headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("referer"))
            || opt.get("headers").and_then(|x| x.get("Referer")).is_some();
        if !has_ref && !self.referer.is_empty() {
            req = req.header("Referer", self.referer.as_str());
        }

        let body = opt
            .get("body")
            .or_else(|| opt.get("data"))
            .and_then(|x| x.as_str())
            .map(|x| x.to_string());
        if method == "POST" {
            if let Some(b) = body {
                if opt.get("form").and_then(|x| x.as_bool()).unwrap_or(false) {
                    req = req.form(&b);
                } else {
                    req = req.body(b);
                }
            }
        }

        match req.send() {
            Ok(resp) => {
                let code = resp.status().as_u16();
                let charset = resp
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|ct| {
                        ct.to_ascii_lowercase()
                            .split("charset=")
                            .nth(1)
                            .map(|x| x.split(';').next().unwrap_or("").trim().to_string())
                    });
                let final_url = resp.url().to_string();
                let bytes = resp.bytes().unwrap_or_default().to_vec();
                let text = http::decode(&bytes, charset.as_deref());
                json!({ "code": code, "url": final_url, "content": text }).to_string()
            }
            Err(e) => json!({"error": format!("{e}")}).to_string(),
        }
    }

    fn get(&self, url: &str) -> Result<String> {
        let payload = json!({ "url": url, "opt": {} }).to_string();
        let raw = self.fetch(&payload);
        let v: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
        if let Some(err) = v.get("error").and_then(|x| x.as_str()) {
            return Err(anyhow!("{err}"));
        }
        let code = v.get("code").and_then(|x| x.as_u64()).unwrap_or(0);
        if !(200..400).contains(&code) {
            return Err(anyhow!("HTTP {code}"));
        }
        Ok(v.get("content").and_then(|x| x.as_str()).unwrap_or("").to_string())
    }
}

// ---------- 选择器 DSL ----------

/// drpy 的列表规则：`容器;标题;图片;简介;链接`
#[derive(Debug, Clone, Default)]
struct SelRule {
    container: String,
    title: String,
    pic: String,
    desc: String,
    url: String,
}

fn parse_sel_rule(raw: &str) -> SelRule {
    let parts: Vec<String> = raw.split(';').map(|x| x.trim().to_string()).collect();
    let mut rule = SelRule {
        container: parts.first().cloned().unwrap_or_default(),
        ..Default::default()
    };
    let fields: Vec<&String> = parts.iter().skip(1).filter(|_| true).collect();
    match fields.len() {
        0 => {}
        1 => rule.title = parts[1].clone(),
        2 => {
            rule.title = parts[1].clone();
            rule.url = parts[2].clone();
        }
        3 => {
            rule.title = parts[1].clone();
            rule.pic = parts[2].clone();
            rule.url = parts[3].clone();
        }
        _ => {
            rule.title = parts[1].clone();
            rule.pic = parts[2].clone();
            rule.desc = parts[3].clone();
            rule.url = parts[4].clone();
        }
    }
    rule
}

/// 字段值：`;` 分隔多个候选，取第一个非空。
fn field_value(html: &str, expr: &str) -> String {
    for alt in expr.split(';') {
        let alt = alt.trim();
        if alt.is_empty() {
            continue;
        }
        let v = pdf::pdfh(html, alt);
        if !v.is_empty() {
            return v;
        }
    }
    String::new()
}

/// 图片字段常是 `background-image:url(...)`，把地址抠出来。
fn norm_pic(raw: &str, base: &str) -> Option<String> {
    let v = raw.trim();
    if v.is_empty() {
        return None;
    }
    let inner = if let Some(i) = v.find("url(") {
        let rest = &v[i + 4..];
        let rest = rest.trim_start_matches(['"', '\'']);
        let end = rest
            .find([')', '"', '\''])
            .unwrap_or(rest.len());
        rest[..end].trim().to_string()
    } else {
        v.to_string()
    };
    if inner.is_empty() {
        return None;
    }
    Some(http::join(base, &inner))
}

fn apply_sel_rule(html: &str, rule: &SelRule, base: &str) -> Vec<Hit> {
    let mut out = Vec::new();
    for item in pdf::pdfa(html, &rule.container) {
        let name = field_value(&item, &rule.title);
        if name.is_empty() {
            continue;
        }
        let url = field_value(&item, &rule.url);
        if url.is_empty() {
            continue;
        }
        out.push(Hit {
            id: http::join(base, &url),
            name,
            pic: norm_pic(&field_value(&item, &rule.pic), base),
            remarks: {
                let d = field_value(&item, &rule.desc);
                (!d.is_empty()).then_some(d)
            },
            year: None,
        });
    }
    out
}

/// 百分号解码（剧集地址里常见 `urlencode` 过的 `url|input|desc`）。
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
            if let Ok(v) = u8::from_str_radix(hex, 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

impl Spy {
    /// 读脚本、取出 `rule`。规则脚本本身不依赖 drpy 运行时，直接 eval 即可。
    pub fn load(script: &str, script_url: &str, headers: &[(String, String)]) -> Result<Self> {
        if script.trim_start().contains("import ") && script.trim_start().contains(" from \"") {
            return Err(anyhow!("该规则依赖 drpy2 模块加载，暂不支持"));
        }
        let src = script.to_string();
        let base_headers = headers.to_vec();
        let raw = block_on_js(Duration::from_secs(10), move |ctx| {
            install_host(ctx, Net::new(base_headers, String::new()))?;
            eval_void(ctx, PRELUDE, "宿主脚本初始化失败")?;
            eval_void(ctx, src.as_str(), "规则脚本执行失败")?;
            eval_detail(ctx, RULE_JSON, "读取规则失败")
        })?;
        let v: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        let headers = match v.get("headers") {
            Some(Value::String(text)) => http::parse_json(text)
                .map(|x| http::header_pairs(&x))
                .unwrap_or_default(),
            Some(other) => http::header_pairs(other),
            None => Vec::new(),
        };
        let rule = Rule {
            title: s("title"),
            host: s("host"),
            url: s("url"),
            search_url: s("searchUrl"),
            class_name: s("className"),
            class_url: s("classUrl"),
            headers,
            home_raw: s("home"),
            detail_raw: v.get("detail").cloned().unwrap_or(Value::String(String::new())),
            search_raw: s("search"),
            lazy_raw: s("lazy"),
            tabs_raw: s("tabs"),
        };
        if rule.host.is_empty() && rule.url.is_empty() {
            return Err(anyhow!("脚本里没有找到 rule.host"));
        }
        Ok(Self {
            rule,
            script: script.to_string(),
            script_url: script_url.to_string(),
        })
    }

    fn net(&self) -> Net {
        Net::new(self.rule.headers.clone(), self.rule.host.clone())
    }

    fn page_url(&self, cate: &str, page: u32) -> String {
        let mut tpl = self.rule.url.clone();
        if tpl.is_empty() {
            tpl = "/".to_string();
        }
        tpl = tpl
            .replace("fyclass", cate)
            .replace("fypage", &page.to_string())
            .replace("fyfilter", "");
        // 过滤器占位符 `{{fl.xxx or "0"}}` 统一取默认值
        let re = regex::Regex::new(r"\{\{fl\.[^}]*\}\}").unwrap();
        let tpl = re.replace_all(&tpl, "0").to_string();
        http::join(&self.rule.host, &tpl)
    }

    fn search_url(&self, wd: &str, page: u32) -> String {
        let tpl = self.rule.search_url.clone();
        if tpl.is_empty() {
            return String::new();
        }
        let out = tpl
            .replace("**", &crate::tvbox::enc(wd))
            .replace("fypage", &page.to_string());
        http::join(&self.rule.host, &out)
    }

    fn detail_url(&self, id: &str) -> String {
        http::join(&self.rule.host, id)
    }

    /// 分类列表。
    pub fn list(&self, cate: &str, page: u32) -> Result<Vec<Hit>> {
        let raw = self.rule.home_raw.clone();
        if raw.trim().is_empty() {
            return Ok(Vec::new());
        }
        let input = self.page_url(cate, page);
        self.hits_from(&raw, input, cate.to_string(), String::new(), page)
    }

    /// 搜索。
    pub fn search(&self, wd: &str, page: u32) -> Result<Vec<Hit>> {
        let raw = if self.rule.search_raw.trim().is_empty() {
            self.rule.home_raw.clone()
        } else {
            self.rule.search_raw.clone()
        };
        if raw.trim().is_empty() {
            return Err(anyhow!("该站点没有搜索规则"));
        }
        let input = self.search_url(wd, page);
        if input.is_empty() {
            return Err(anyhow!("该站点没有搜索地址"));
        }
        self.hits_from(&raw, input, String::new(), wd.to_string(), page)
    }

    /// 列表/搜索的统一入口：选择器规则直接抓页面，`js:` 交给引擎。
    fn hits_from(
        &self,
        raw: &str,
        input: String,
        cate: String,
        wd: String,
        page: u32,
    ) -> Result<Vec<Hit>> {
        if Rule::is_js(raw) {
            let v = self.run_code(
                Rule::code(raw),
                Vars {
                    page,
                    cate,
                    wd,
                    input,
                    html: String::new(),
                },
                None,
            )?;
            return Ok(to_hits(&v));
        }
        let html = self.net().get(&input)?;
        let rule = parse_sel_rule(raw);
        Ok(apply_sel_rule(&html, &rule, &self.rule.host))
    }

    /// 详情 + 播放列表。
    pub fn detail(&self, id: &str) -> Result<VodDetail> {
        let raw = self.rule.detail_raw.clone();
        let input = self.detail_url(id);
        match raw {
            Value::Object(fields) => {
                let html = self.net().get(&input)?;
                self.detail_from_fields(&fields, &html, &input, id)
            }
            Value::String(code) if !code.trim().is_empty() => {
                let html = self.net().get(&input).unwrap_or_default();
                let v = self.run_code(
                    Rule::code(&code),
                    Vars {
                        page: 1,
                        cate: String::new(),
                        wd: String::new(),
                        input,
                        html,
                    },
                    None,
                )?;
                let mut d = to_detail(&v, &self.rule.host);
                if d.id.is_empty() {
                    d.id = id.to_string();
                }
                Ok(d)
            }
            _ => Err(anyhow!("该站点没有详情规则")),
        }
    }

    fn detail_from_fields(
        &self,
        fields: &serde_json::Map<String, Value>,
        html: &str,
        input: &str,
        id: &str,
    ) -> Result<VodDetail> {
        let get = |key: &str| -> String {
            match fields.get(key) {
                Some(Value::String(expr)) if !expr.trim().is_empty() => {
                    if Rule::is_js(expr) {
                        String::new()
                    } else {
                        field_value(html, expr)
                    }
                }
                _ => String::new(),
            }
        };

        let mut sources = Vec::new();
        // 线路名：`tabs` 跑出来是数组；没有就按「线路1」
        let tabs_expr = match fields.get("tabs") {
            Some(Value::String(x)) if !x.trim().is_empty() => x.clone(),
            _ => self.rule.tabs_raw.clone(),
        };
        let tabs = self.tabs_list(&tabs_expr, html, input);
        if let Some(Value::String(lists)) = fields.get("lists") {
            let v = self.run_code(
                Rule::code(lists),
                Vars {
                    page: 1,
                    cate: String::new(),
                    wd: String::new(),
                    input: input.to_string(),
                    html: html.to_string(),
                },
                None,
            )?;
            let groups: Vec<Vec<String>> = json_to_groups(&v);
            for (i, group) in groups.iter().enumerate() {
                let from = tabs
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| format!("线路{}", i + 1));
                let mut episodes = Vec::new();
                for line in group {
                    if let Some(ep) = episode_of(line, &self.rule.host) {
                        episodes.push(ep);
                    }
                }
                if !episodes.is_empty() {
                    sources.push(PlaySource { from, episodes });
                }
            }
        }

        let content = {
            let c = get("content");
            if c.is_empty() {
                get("desc")
            } else {
                c
            }
        };
        Ok(VodDetail {
            id: id.to_string(),
            name: {
                let n = get("title");
                if n.is_empty() {
                    get("name")
                } else {
                    n
                }
            },
            pic: norm_pic(&get("img"), &self.rule.host),
            year: {
                let y = get("year");
                (!y.is_empty()).then_some(y)
            },
            area: {
                let a = get("area");
                (!a.is_empty()).then_some(a)
            },
            kind: {
                let k = get("type");
                (!k.is_empty()).then_some(k)
            },
            actor: {
                let a = get("actor");
                (!a.is_empty()).then_some(a)
            },
            director: {
                let d = get("director");
                (!d.is_empty()).then_some(d)
            },
            content: (!content.is_empty()).then_some(content),
            sources,
        })
    }

    fn tabs_list(&self, expr: &str, html: &str, input: &str) -> Vec<String> {
        if !Rule::is_js(expr) {
            return Vec::new();
        }
        let v = self.run_code(
            Rule::code(expr),
            Vars {
                page: 1,
                cate: String::new(),
                wd: String::new(),
                input: input.to_string(),
                html: html.to_string(),
            },
            Some("TABS"),
        );
        match v {
            Ok(Value::Array(items)) => items
                .iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect(),
            _ => Vec::new(),
        }
    }

    /// 播放地址解析（`lazy`）。返回 (地址, 请求头JSON)。
    pub fn play(&self, url: &str) -> Result<(String, Option<String>)> {
        if !Rule::is_js(&self.rule.lazy_raw) {
            return Err(anyhow!("该站点没有 lazy 规则"));
        }
        let v = self.run_code(
            Rule::code(&self.rule.lazy_raw),
            Vars {
                page: 1,
                cate: String::new(),
                wd: String::new(),
                input: url.to_string(),
                html: String::new(),
            },
            Some("input"),
        )?;
        match v {
            Value::String(s) => {
                if s.trim().is_empty() {
                    Err(anyhow!("lazy 没有返回地址"))
                } else {
                    Ok((http::join(&self.rule.host, s.trim()), None))
                }
            }
            Value::Object(map) => {
                let u = map
                    .get("url")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                if u.is_empty() {
                    return Err(anyhow!("lazy 没有返回地址"));
                }
                let headers = map.get("header").and_then(|x| x.as_str()).map(|s| s.to_string());
                Ok((http::join(&self.rule.host, &u), headers))
            }
            _ => Err(anyhow!("lazy 返回格式无法识别")),
        }
    }

    fn run_code(&self, code: &str, vars: Vars, ret: Option<&str>) -> Result<Value> {
        let script = self.script.clone();
        // drpy 的约定：tabs 的结果在 TABS，lazy 的结果在改写后的 input
        let out_expr = ret
            .unwrap_or("(__RESULT !== null && __RESULT !== undefined) ? __RESULT : (LISTS && LISTS.length ? LISTS : d)");
        let wrapped = format!(
            r#"
(function(){{
  var rule = (typeof __RULE__ !== "undefined" && __RULE__) || {{}};
  var HOST = rule.host || {host};
  var MY_PAGE = {page};
  var MY_CATE = {cate};
  var MY_FLAG = "";
  var MY_FILTER = false;
  var MY_CATE_NAME = {cate};
  var WD = {wd};
  var KEY = WD;
  var input = {input};
  var MY_URL = input;
  var html = {html};
  var d = [];
  __RESULT = null;
  TABS = [];
  LISTS = [];
  try {{
    var __h = {hdrs};
    if (__h.Cookie === undefined) __h.Cookie = "";
    rule_fetch_params.headers = __h;
    fetch_params.headers = __h;
  }} catch(e) {{}}
  (function(){{
{code}
  }})();
  var out = {out_expr};
  return JSON.stringify(out === undefined ? null : out);
}})()
"#,
            host = js_str(&self.rule.host),
            page = vars.page,
            cate = js_str(&vars.cate),
            wd = js_str(&vars.wd),
            input = js_str(&vars.input),
            html = js_str(&vars.html),
            hdrs = header_json(&self.rule.headers),
            out_expr = out_expr,
            code = code,
        );
        let headers = self.rule.headers.clone();
        let referer = self.rule.host.clone();
        let raw = block_on_js(Duration::from_secs(25), move |ctx| {
            install_host(ctx, Net::new(headers, referer))?;
            eval_void(ctx, PRELUDE, "宿主脚本初始化失败")?;
            eval_void(ctx, script.as_str(), "规则脚本执行失败")?;
            eval_void(
                ctx,
                "var __RULE__ = (typeof rule !== 'undefined' && rule) || (typeof RULE !== 'undefined' && RULE) || {};",
                "规则读取失败",
            )?;
            eval_detail(ctx, wrapped.as_str(), "规则执行失败")
        })?;
        let v: Value = serde_json::from_str(&raw).map_err(|e| anyhow!("规则返回了非 JSON：{e}"))?;
        if let Some(err) = v.get("__error").and_then(|x| x.as_str()) {
            return Err(anyhow!("{err}"));
        }
        Ok(v)
    }
}

struct Vars {
    page: u32,
    cate: String,
    wd: String,
    input: String,
    html: String,
}

fn install_host(ctx: &rquickjs::Ctx, net: Net) -> Result<()> {
    let host = Function::new(ctx.clone(), move |op: String, payload: String| -> String {
        match op.as_str() {
            "req" => net.fetch(&payload),
            "pdfh" => {
                let v: Value = serde_json::from_str(&payload).unwrap_or(Value::Null);
                let html = v.get(0).and_then(|x| x.as_str()).unwrap_or("");
                let sel = v.get(1).and_then(|x| x.as_str()).unwrap_or("");
                pdf::pdfh(html, sel)
            }
            "pdfa" => {
                let v: Value = serde_json::from_str(&payload).unwrap_or(Value::Null);
                let html = v.get(0).and_then(|x| x.as_str()).unwrap_or("");
                let sel = v.get(1).and_then(|x| x.as_str()).unwrap_or("");
                serde_json::to_string(&pdf::pdfa(html, sel)).unwrap_or_else(|_| "[]".into())
            }
            "log" => String::new(),
            _ => String::new(),
        }
    })
    .map_err(|e| anyhow!("宿主函数注册失败：{e}"))?;
    ctx.globals()
        .set("__host", host)
        .map_err(|e| anyhow!("宿主函数注册失败：{e}"))?;
    Ok(())
}

fn text_of(v: &Value, keys: &[&str]) -> String {
    for k in keys {
        match v.get(*k) {
            Some(Value::String(s)) if !s.trim().is_empty() => return s.trim().to_string(),
            Some(Value::Number(n)) => return n.to_string(),
            _ => {}
        }
    }
    String::new()
}

fn to_hits(v: &Value) -> Vec<Hit> {
    let arr = v.as_array().cloned().unwrap_or_default();
    let mut out = Vec::new();
    for item in arr {
        if !item.is_object() {
            continue;
        }
        let name = text_of(&item, &["title", "name", "vod_name"]);
        let url = text_of(&item, &["url", "vod_id", "id", "link"]);
        if name.is_empty() || url.is_empty() {
            continue;
        }
        let pic = text_of(&item, &["pic_url", "pic", "img", "vod_pic"]);
        let desc = text_of(&item, &["desc", "remarks", "vod_remarks", "note"]);
        out.push(Hit {
            id: url,
            name,
            pic: (!pic.is_empty()).then_some(pic),
            remarks: (!desc.is_empty()).then_some(desc),
            year: None,
        });
    }
    out
}

/// `lists` 返回的一般是「数组的数组」，也兼容一维数组。
fn json_to_groups(v: &Value) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    let Some(arr) = v.as_array() else { return out };
    let nested = arr.iter().any(|x| x.is_array());
    if !nested {
        let flat: Vec<String> = arr
            .iter()
            .filter_map(|x| x.as_str().map(|s| s.to_string()))
            .collect();
        if !flat.is_empty() {
            out.push(flat);
        }
        return out;
    }
    for group in arr {
        let Some(items) = group.as_array() else { continue };
        let lines: Vec<String> = items
            .iter()
            .filter_map(|x| x.as_str().map(|s| s.to_string()))
            .collect();
        out.push(lines);
    }
    out
}

/// `第1集$urlencode(url|input|desc)` → Episode
fn episode_of(line: &str, base: &str) -> Option<Episode> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let (name, rest) = match line.split_once('$') {
        Some((n, r)) => (n.trim().to_string(), r.trim()),
        None => (String::new(), line),
    };
    let decoded = percent_decode(rest);
    let url = decoded.split('|').next().unwrap_or("").trim().to_string();
    if url.is_empty() {
        return None;
    }
    Some(Episode {
        name: if name.is_empty() {
            "播放".to_string()
        } else {
            name
        },
        url: http::join(base, &url),
    })
}

/// 详情：drpy 的 `{tabs:[{name,list:[{name,url}]}]}` 与 CMS 的 `vod_play_*` 都认。
fn to_detail(v: &Value, base: &str) -> VodDetail {
    let mut sources = Vec::new();
    if let Some(tabs) = v.get("tabs").and_then(|x| x.as_array()) {
        for tab in tabs {
            let from = text_of(tab, &["name", "title", "from"]);
            let mut episodes = Vec::new();
            if let Some(list) = tab.get("list").and_then(|x| x.as_array()) {
                for ep in list {
                    let name = text_of(ep, &["name", "title"]);
                    let url = text_of(ep, &["url", "link"]);
                    if url.is_empty() {
                        continue;
                    }
                    episodes.push(Episode {
                        name: if name.is_empty() {
                            format!("第{}集", episodes.len() + 1)
                        } else {
                            name
                        },
                        url: http::join(base, &url),
                    });
                }
            }
            if !episodes.is_empty() {
                sources.push(PlaySource {
                    from: if from.is_empty() {
                        format!("线路{}", sources.len() + 1)
                    } else {
                        from
                    },
                    episodes,
                });
            }
        }
    }
    if sources.is_empty() {
        if let Some(cms) = crate::tvbox::detail_from_value(v) {
            return cms;
        }
    }

    VodDetail {
        id: text_of(v, &["vod_id", "id", "url"]),
        name: text_of(v, &["vod_name", "title", "name"]),
        pic: {
            let p = text_of(v, &["vod_pic", "img", "pic"]);
            (!p.is_empty()).then_some(p)
        },
        year: {
            let y = text_of(v, &["vod_year", "year"]);
            (!y.is_empty()).then_some(y)
        },
        area: {
            let a = text_of(v, &["vod_area", "area"]);
            (!a.is_empty()).then_some(a)
        },
        kind: {
            let k = text_of(v, &["vod_type", "type_name", "cate"]);
            (!k.is_empty()).then_some(k)
        },
        actor: {
            let a = text_of(v, &["vod_actor", "actor"]);
            (!a.is_empty()).then_some(a)
        },
        director: {
            let d = text_of(v, &["vod_director", "director"]);
            (!d.is_empty()).then_some(d)
        },
        content: {
            let c = text_of(v, &["vod_content", "content", "desc", "des"]);
            (!c.is_empty()).then_some(c)
        },
        sources,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCRIPT: &str = r#"
var rule = {
  title: '测试站',
  host: 'https://a.example',
  url: '/list/fyclass/fypage.html',
  searchUrl: '/search?wd=**&page=fypage',
  '一级': '.post-box-list&&article;a:eq(0)&&Text;.cover&&style;a:eq(0)&&Text;a&&href',
  '二级': {
    'title': '.post-title&&Text;.cat-links&&Text',
    'img': '.poster&&img&&data-src',
    'content': '.abstract&&Text',
    'tabs': "js:TABS=['线路一','线路二']",
    'lists': "js:LISTS=[['第1集$' + urlencode('https://a.example/1.m3u8')], ['第1集$' + urlencode('https://a.example/2.m3u8')]]"
  },
  '搜索': '#main&&article;.post-title&&Text;;.published&&Text;a&&href',
  lazy: "js:let u = input.split('|')[0]; input = {url: u, header: JSON.stringify({Referer: HOST})};"
};
"#;

    const LIST_HTML: &str = r#"<html><body>
      <div class="post-box-list">
        <article><a href="/p/1.html"><span class="post-title">片名一</span></a>
          <div class="cover" style="background-image:url('/img/1.jpg')"></div></article>
        <article><a href="/p/2.html"><span class="post-title">片名二</span></a>
          <div class="cover" style="background-image:url('/img/2.jpg')"></div></article>
      </div>
    </body></html>"#;

    const DETAIL_HTML: &str = r#"<html><body>
      <h1 class="post-title">片名一</h1>
      <div class="abstract">简介内容</div>
      <div class="poster"><img data-src="/img/1.jpg"></div>
    </body></html>"#;

    #[test]
    fn reads_rule_fields() {
        let spy = Spy::load(SCRIPT, "https://a.example/js/t.js", &[]).unwrap();
        assert_eq!(spy.rule.title, "测试站");
        assert_eq!(spy.page_url("1", 2), "https://a.example/list/1/2.html");
        assert_eq!(
            spy.search_url("庆余年", 1),
            "https://a.example/search?wd=%E5%BA%86%E4%BD%99%E5%B9%B4&page=1"
        );
        assert!(Rule::is_js(&spy.rule.search_raw) == false);
        assert!(spy.rule.detail_raw.is_object());
    }

    #[test]
    fn selector_rule_parses_list() {
        let rule = parse_sel_rule(".post-box-list&&article;a:eq(0)&&Text;.cover&&style;a:eq(0)&&Text;a&&href");
        assert_eq!(rule.container, ".post-box-list&&article");
        assert_eq!(rule.title, "a:eq(0)&&Text");
        assert_eq!(rule.pic, ".cover&&style");
        assert_eq!(rule.desc, "a:eq(0)&&Text");
        assert_eq!(rule.url, "a&&href");

        let hits = apply_sel_rule(LIST_HTML, &rule, "https://a.example");
        assert_eq!(hits.len(), 2, "应解析出两条");
        assert_eq!(hits[0].name, "片名一");
        assert_eq!(hits[0].id, "https://a.example/p/1.html");
        assert_eq!(hits[0].pic.as_deref(), Some("https://a.example/img/1.jpg"));
    }

    #[test]
    fn search_rule_with_empty_pic_field() {
        let rule = parse_sel_rule("#main&&article;.post-title&&Text;;.published&&Text;a&&href");
        assert_eq!(rule.container, "#main&&article");
        assert_eq!(rule.title, ".post-title&&Text");
        assert_eq!(rule.pic, "");
    }

    #[test]
    fn field_value_tries_alternatives() {
        let html = r#"<div class="cat-links">分类</div>"#;
        assert_eq!(field_value(html, ".post-title&&Text;.cat-links&&Text"), "分类");
        assert_eq!(field_value(html, ".nothing&&Text"), "");
    }

    #[test]
    fn episode_line_decodes_urlencode() {
        let line = "第01集$https%3A%2F%2Fv.qq.com%2F1.m3u8%7Chttps%3A%2F%2Fddys.pro%2Fp%2F1";
        let ep = episode_of(line, "https://ddys.pro").unwrap();
        assert_eq!(ep.name, "第01集");
        assert_eq!(ep.url, "https://v.qq.com/1.m3u8");
    }

    #[test]
    fn detail_fields_and_lists() {
        let spy = Spy::load(SCRIPT, "https://a.example/js/t.js", &[]).unwrap();
        let fields = spy.rule.detail_raw.as_object().cloned().unwrap();
        let d = spy
            .detail_from_fields(&fields, DETAIL_HTML, "https://a.example/p/1.html", "/p/1.html")
            .unwrap();
        assert_eq!(d.name, "片名一");
        assert_eq!(d.content.as_deref(), Some("简介内容"));
        assert_eq!(d.pic.as_deref(), Some("https://a.example/img/1.jpg"));
        assert_eq!(d.sources.len(), 2);
        assert_eq!(d.sources[0].from, "线路一");
        assert_eq!(d.sources[1].episodes[0].url, "https://a.example/2.m3u8");
    }

    #[test]
    fn lazy_returns_url_and_header() {
        let spy = Spy::load(SCRIPT, "https://a.example/js/t.js", &[]).unwrap();
        let (url, header) = spy.play("https://a.example/1.m3u8|https://a.example/p/1").unwrap();
        assert_eq!(url, "https://a.example/1.m3u8");
        assert!(header.unwrap_or_default().contains("Referer"));
    }

    #[test]
    fn rejects_drpy_module_script() {
        let src = r#"import cheerio from "assets://js/lib/cheerio.min.js";"#;
        assert!(Spy::load(src, "", &[]).is_err());
    }

    #[test]
    fn maps_list_items_to_hits() {
        let v = json!([
            {"title": "片名", "desc": "更新到 10 集", "pic_url": "https://a/1.jpg", "url": "https://a/p/1.html"}
        ]);
        let hits = to_hits(&v);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "片名");
        assert_eq!(hits[0].remarks.as_deref(), Some("更新到 10 集"));
    }

    #[test]
    fn normalizes_pic_from_css() {
        assert_eq!(
            norm_pic("background-image:url(\"/img/a.jpg\")", "https://x").as_deref(),
            Some("https://x/img/a.jpg")
        );
        assert_eq!(
            norm_pic("https://cdn/a.jpg", "https://x").as_deref(),
            Some("https://cdn/a.jpg")
        );
    }
}
