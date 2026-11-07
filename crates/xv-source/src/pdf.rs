//! drpy 里的 `pdfh` / `pdfa`：用 `&&` 串起来的简易选择器。
//!
//! 语法沿用 TVBox 的习惯：
//! - `body&&.col`        先定位 `body`，再在其中找 `.col`
//! - `.card&&h2&&Text`   逐层下钻，最后一段是取值动作
//! - 取值动作：`Text` / `Html` / `tag` / 具体属性名（`href`、`style`、`data-id`…）
//! - 支持 `:eq(n)` `:first` `:last` `:lt(n)` `:gt(n)` 这几个 drpy 常用的下标伪类

use scraper::{ElementRef, Html, Selector};

/// 选择器里的下标伪类。
#[derive(Debug, Clone, Copy, PartialEq)]
enum Pick {
    Eq(usize),
    Lt(usize),
    Gt(usize),
    First,
    Last,
}

/// 取值动作。
#[derive(Debug, Clone, PartialEq)]
enum Prop {
    Text,
    Html,
    Tag,
    Attr(String),
}

fn parse_prop(token: &str) -> Option<Prop> {
    let t = token.trim();
    match t {
        "" => None,
        "Text" | "text" | "textContent" => Some(Prop::Text),
        "Html" | "html" | "*" => Some(Prop::Html),
        "tag" | "tagName" => Some(Prop::Tag),
        _ => {
            let lower = t.to_ascii_lowercase();
            let known = matches!(
                lower.as_str(),
                "href" | "src" | "style" | "title" | "alt" | "class" | "id" | "value" | "onclick"
            ) || lower.starts_with("data-")
                || lower.starts_with("aria-");
            known.then_some(Prop::Attr(lower))
        }
    }
}

/// 把结尾的 `:eq(2)` / `:first` / `:last` 摘出来，剩下的交给 scraper 的 CSS 解析。
fn split_pick(sel: &str) -> (String, Option<Pick>) {
    let s = sel.trim();
    let lower = s.to_ascii_lowercase();
    for (tag, pick) in [(":first", Pick::First), (":last", Pick::Last)] {
        if lower.ends_with(tag) {
            return (s[..s.len() - tag.len()].trim_end().to_string(), Some(pick));
        }
    }
    if lower.ends_with(')') {
        if let Some(open) = s.rfind('(') {
            let head = &lower[..open];
            let n: usize = s[open + 1..s.len() - 1].trim().parse().unwrap_or(0);
            let pick = if head.ends_with(":eq") {
                Some(Pick::Eq(n))
            } else if head.ends_with(":lt") {
                Some(Pick::Lt(n))
            } else if head.ends_with(":gt") {
                Some(Pick::Gt(n))
            } else {
                None
            };
            if let Some(p) = pick {
                return (s[..open - 3].trim_end().to_string(), Some(p));
            }
        }
    }
    (s.to_string(), None)
}

fn apply_pick(nodes: Vec<ElementRef<'_>>, pick: Option<Pick>) -> Vec<ElementRef<'_>> {
    match pick {
        None => nodes,
        Some(Pick::Eq(n)) => nodes.into_iter().nth(n).into_iter().collect(),
        Some(Pick::First) => nodes.into_iter().take(1).collect(),
        Some(Pick::Last) => nodes.into_iter().last().into_iter().collect(),
        Some(Pick::Lt(n)) => nodes.into_iter().take(n).collect(),
        Some(Pick::Gt(n)) => nodes.into_iter().skip(n + 1).collect(),
    }
}

fn select_all<'a>(doc: &'a Html, sel: &str) -> Vec<ElementRef<'a>> {
    let (clean, pick) = split_pick(sel);
    let clean = clean.trim();
    if clean.is_empty() {
        return Vec::new();
    }
    let Ok(selector) = Selector::parse(clean) else {
        return Vec::new();
    };
    apply_pick(doc.select(&selector).collect(), pick)
}

fn element_text(el: &ElementRef) -> String {
    el.text().collect::<String>().trim().to_string()
}

fn prop_of(el: &ElementRef, prop: &Prop) -> String {
    match prop {
        Prop::Text => element_text(el),
        Prop::Html => el.html(),
        Prop::Tag => el.value().name().to_string(),
        Prop::Attr(name) => {
            // 属性名大小写不敏感（HTML 里全小写，规则里可能写 Href）
            let lower = name.to_ascii_lowercase();
            if let Some(v) = el.value().attr(&lower) {
                return v.trim().to_string();
            }
            el.value()
                .attrs()
                .find(|(k, _)| k.eq_ignore_ascii_case(&lower))
                .map(|(_, v)| v.trim().to_string())
                .unwrap_or_default()
        }
    }
}

/// 拆 `a&&b&&Text`：返回（选择器链, 取值动作）。
fn split_chain(selector: &str) -> (Vec<String>, Option<Prop>) {
    let mut tokens: Vec<String> = selector
        .split("&&")
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty())
        .collect();
    let mut prop = None;
    if let Some(last) = tokens.last().cloned() {
        if let Some(p) = parse_prop(&last) {
            prop = Some(p);
            tokens.pop();
        }
    }
    (tokens, prop)
}

/// `pdfa(html, selector)`：返回所有命中节点的外层 HTML。
pub fn pdfa(html: &str, selector: &str) -> Vec<String> {
    let (tokens, prop) = split_chain(selector);
    if tokens.is_empty() {
        return Vec::new();
    }
    let mut scope: Vec<String> = vec![html.to_string()];
    for (i, token) in tokens.iter().enumerate() {
        let last = i + 1 == tokens.len();
        let mut next: Vec<String> = Vec::new();
        for h in &scope {
            // 顶层按完整文档解析（否则 `<body>` 这类标签在片段模式下会被丢掉）
            let doc = if i == 0 {
                Html::parse_document(h)
            } else {
                Html::parse_fragment(h)
            };
            if last {
                for el in select_all(&doc, token) {
                    next.push(match &prop {
                        Some(p) => prop_of(&el, p),
                        None => el.html(),
                    });
                }
            } else if let Some(el) = select_all(&doc, token).first() {
                next.push(el.html());
            }
        }
        scope = next;
        if scope.is_empty() {
            return Vec::new();
        }
    }
    scope
}

/// `pdfh(html, selector)`：返回第一个命中节点按取值动作得到的字符串。
pub fn pdfh(html: &str, selector: &str) -> String {
    let (tokens, prop) = split_chain(selector);
    if tokens.is_empty() {
        return String::new();
    }
    let mut current = html.to_string();
    for (i, token) in tokens.iter().enumerate() {
        let last = i + 1 == tokens.len();
        let doc = if i == 0 {
            Html::parse_document(&current)
        } else {
            Html::parse_fragment(&current)
        };
        let Some(el) = select_all(&doc, token).into_iter().next() else {
            return String::new();
        };
        if last {
            return match &prop {
                Some(p) => prop_of(&el, p),
                None => element_text(&el),
            };
        }
        current = el.html();
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    const HTML: &str = r#"<html><body>
      <div class="col"><a href="/p/1.html"><h2>片名一</h2></a><span class="me-auto">8.1分</span></div>
      <div class="col"><a href="/p/2.html"><h2>片名二</h2></a><span class="me-auto">7.2分</span></div>
      <div class="col"><a href="/p/3.html"><h2>片名三</h2></a></div>
    </body></html>"#;

    #[test]
    fn pdfa_returns_all_matches() {
        let list = pdfa(HTML, "body&&.col");
        assert_eq!(list.len(), 3);
        assert!(list[0].contains("片名一"));
    }

    #[test]
    fn pdfh_drills_and_reads_text() {
        assert_eq!(pdfh(HTML, ".col&&h2&&Text"), "片名一");
        assert_eq!(pdfh(HTML, ".col&&a&&href"), "/p/1.html");
        assert_eq!(pdfh(HTML, ".col&&.me-auto&&Text"), "8.1分");
    }

    #[test]
    fn pdfh_supports_index_pseudo() {
        assert_eq!(pdfh(HTML, ".col:eq(1)&&h2&&Text"), "片名二");
        assert_eq!(pdfh(HTML, ".col:last&&h2&&Text"), "片名三");
    }

    #[test]
    fn missing_selector_is_empty() {
        assert_eq!(pdfh(HTML, ".nothing&&Text"), "");
        assert!(pdfa(HTML, ".nothing").is_empty());
        assert_eq!(pdfh("", ".col&&Text"), "");
    }
}
