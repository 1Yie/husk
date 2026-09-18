//! `web_fetch` — fetch web pages and convert to clean markdown/text.
//!
//! Read-only network tool for browsing documentation, API docs, GitHub issues,
//! and online references.

use std::sync::Arc;
use std::time::Duration;

use futures::FutureExt;
use serde::Deserialize;

use super::registry::{schema_for, Args, ToolCtx, ToolError, ToolResult, ToolSpec};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct WebFetchArgs {
    /// The HTTP or HTTPS URL to fetch.
    url: String,

    /// Output format: "markdown" (default, converts HTML to readable markdown),
    /// "text" (plain text extraction), or "html" (raw HTML).
    format: Option<String>,

    /// Maximum characters to return. Default: 50,000.
    max_length: Option<usize>,
}

pub fn spec() -> ToolSpec {
    ToolSpec {
        name: "web_fetch",
        schema: schema_for::<WebFetchArgs>(
            "Fetch content from a web URL (HTTP/HTTPS) and return it as clean markdown or text.\n\
             Useful for reading online documentation, library APIs, GitHub issues/discussions,\n\
             specifications, and technical articles.",
        ),
        readonly: true,
        exec: Arc::new(|args, ctx| exec(args, ctx).boxed()),
    }
}

/// Alias tool spec for `webfetch` (without underscore)
pub fn spec_alias() -> ToolSpec {
    let mut s = spec();
    s.name = "webfetch";
    s
}

async fn exec(args: Args, _ctx: Arc<ToolCtx>) -> Result<ToolResult, ToolError> {
    let parsed: WebFetchArgs =
        serde_json::from_value(args).map_err(|e| ToolError::Args(e.to_string()))?;

    let url_str = parsed.url.trim();
    if !url_str.starts_with("http://") && !url_str.starts_with("https://") {
        return Err(ToolError::Args(
            "URL must start with http:// or https://".into(),
        ));
    }

    let max_len = parsed.max_length.unwrap_or(50_000);
    let format_mode = parsed.format.unwrap_or_else(|| "markdown".into());

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36 agent-rs/0.1")
        .build()
        .map_err(|e| ToolError::Failed(format!("Failed to build HTTP client: {e}")))?;

    let resp = client
        .get(url_str)
        .header("Accept", "text/html,application/xhtml+xml,application/json,text/plain;q=0.9,*/*;q=0.8")
        .send()
        .await
        .map_err(|e| ToolError::Failed(format!("Failed to fetch URL '{url_str}': {e}")))?;

    let status = resp.status();
    if !status.is_success() {
        return Err(ToolError::Failed(format!(
            "HTTP request to '{url_str}' returned status {status}"
        )));
    }

    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_lowercase();

    let raw_body = resp
        .text()
        .await
        .map_err(|e| ToolError::Failed(format!("Failed to read response body: {e}")))?;

    let mut result_text = if format_mode == "html" {
        raw_body
    } else if content_type.contains("application/json") {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw_body) {
            serde_json::to_string_pretty(&val).unwrap_or(raw_body)
        } else {
            raw_body
        }
    } else if content_type.contains("text/html") || raw_body.contains("<html") || raw_body.contains("<!DOCTYPE") {
        html_to_markdown(&raw_body, format_mode == "markdown")
    } else {
        raw_body
    };

    let total_len = result_text.chars().count();
    if total_len > max_len {
        let truncated: String = result_text.chars().take(max_len).collect();
        result_text = format!(
            "{}\n\n[Content truncated at {} characters. Total length was {} characters.]",
            truncated, max_len, total_len
        );
    }

    Ok(ToolResult::text(result_text))
}

/// Convert an HTML string into readable markdown or text.
fn html_to_markdown(html: &str, is_markdown: bool) -> String {
    // 1. Remove scripts, styles, svg, and HTML comments
    let s = strip_tag_contents(html, "script");
    let s = strip_tag_contents(&s, "style");
    let s = strip_tag_contents(&s, "svg");
    let s = strip_tag_contents(&s, "noscript");
    let s = strip_tag_contents(&s, "head");
    let s = strip_comments(&s);

    // 2. Format common HTML blocks
    let mut out = s;

    if is_markdown {
        // Headers
        out = replace_tag_with_prefix(&out, "h1", "\n\n# ");
        out = replace_tag_with_prefix(&out, "h2", "\n\n## ");
        out = replace_tag_with_prefix(&out, "h3", "\n\n### ");
        out = replace_tag_with_prefix(&out, "h4", "\n\n#### ");
        out = replace_tag_with_prefix(&out, "h5", "\n\n##### ");
        out = replace_tag_with_prefix(&out, "h6", "\n\n###### ");

        // Code blocks: <pre><code>...</code></pre>
        out = replace_code_blocks(&out);

        // Inline code: <code>...</code>
        out = replace_inline_code(&out);

        // Lists
        out = replace_tag_with_prefix(&out, "li", "\n- ");

        // Links: <a href="url">text</a> -> [text](url)
        out = replace_links(&out);

        // Bold & Italics
        out = replace_simple_pair(&out, "strong", "**");
        out = replace_simple_pair(&out, "b", "**");
        out = replace_simple_pair(&out, "em", "*");
        out = replace_simple_pair(&out, "i", "*");
    }

    // Line breaks and paragraphs
    out = out.replace("<br>", "\n");
    out = out.replace("<br/>", "\n");
    out = out.replace("<br />", "\n");
    out = out.replace("<p>", "\n\n");
    out = out.replace("</p>", "\n\n");
    out = out.replace("<hr>", "\n---\n");
    out = out.replace("<hr/>", "\n---\n");
    out = out.replace("<hr />", "\n---\n");
    out = out.replace("<div>", "\n");
    out = out.replace("</div>", "\n");
    out = out.replace("<tr>", "\n");
    out = out.replace("</tr>", "\n");
    out = out.replace("</td>", " | ");
    out = out.replace("</th>", " | ");

    // 3. Strip all remaining HTML tags
    let mut in_tag = false;
    let mut clean = String::with_capacity(out.len());
    for c in out.chars() {
        if c == '<' {
            in_tag = true;
        } else if c == '>' {
            in_tag = false;
        } else if !in_tag {
            clean.push(c);
        }
    }

    // 4. Decode HTML entities
    let decoded = decode_entities(&clean);

    // 5. Clean up redundant whitespace and consecutive newlines
    clean_whitespace(&decoded)
}

fn strip_tag_contents(input: &str, tag: &str) -> String {
    let open_pattern = format!("<{tag}");
    let close_pattern = format!("</{tag}>");
    let mut res = String::with_capacity(input.len());
    let mut remaining = input;

    while let Some(start_idx) = find_case_insensitive(remaining, &open_pattern) {
        res.push_str(&remaining[..start_idx]);
        let after_open = &remaining[start_idx..];
        if let Some(close_rel) = find_case_insensitive(after_open, &close_pattern) {
            remaining = &after_open[close_rel + close_pattern.len()..];
        } else {
            // Unclosed tag: drop remainder
            remaining = "";
            break;
        }
    }
    res.push_str(remaining);
    res
}

fn strip_comments(input: &str) -> String {
    let mut res = String::with_capacity(input.len());
    let mut remaining = input;
    while let Some(start) = remaining.find("<!--") {
        res.push_str(&remaining[..start]);
        if let Some(end) = remaining[start..].find("-->") {
            remaining = &remaining[start + end + 3..];
        } else {
            remaining = "";
            break;
        }
    }
    res.push_str(remaining);
    res
}

fn find_case_insensitive(haystack: &str, needle: &str) -> Option<usize> {
    let needle_lower = needle.to_lowercase();
    let haystack_lower = haystack.to_lowercase();
    haystack_lower.find(&needle_lower)
}

fn replace_tag_with_prefix(input: &str, tag: &str, prefix: &str) -> String {
    let open_prefix = format!("<{tag}");
    let close_tag = format!("</{tag}>");
    let mut res = String::with_capacity(input.len());
    let mut remaining = input;

    while let Some(start) = find_case_insensitive(remaining, &open_prefix) {
        res.push_str(&remaining[..start]);
        let after = &remaining[start..];
        if let Some(tag_end) = after.find('>') {
            let inner_start = &after[tag_end + 1..];
            if let Some(close_pos) = find_case_insensitive(inner_start, &close_tag) {
                let inner_text = &inner_start[..close_pos];
                res.push_str(prefix);
                res.push_str(inner_text.trim());
                res.push('\n');
                remaining = &inner_start[close_pos + close_tag.len()..];
            } else {
                res.push_str(&after[..tag_end + 1]);
                remaining = &after[tag_end + 1..];
            }
        } else {
            res.push_str(after);
            remaining = "";
            break;
        }
    }
    res.push_str(remaining);
    res
}

fn replace_simple_pair(input: &str, tag: &str, md_wrapper: &str) -> String {
    let open_prefix = format!("<{tag}");
    let close_tag = format!("</{tag}>");
    let mut res = String::with_capacity(input.len());
    let mut remaining = input;

    while let Some(start) = find_case_insensitive(remaining, &open_prefix) {
        res.push_str(&remaining[..start]);
        let after = &remaining[start..];
        if let Some(tag_end) = after.find('>') {
            let inner_start = &after[tag_end + 1..];
            if let Some(close_pos) = find_case_insensitive(inner_start, &close_tag) {
                let inner_text = &inner_start[..close_pos];
                res.push_str(md_wrapper);
                res.push_str(inner_text);
                res.push_str(md_wrapper);
                remaining = &inner_start[close_pos + close_tag.len()..];
            } else {
                res.push_str(&after[..tag_end + 1]);
                remaining = &after[tag_end + 1..];
            }
        } else {
            res.push_str(after);
            remaining = "";
            break;
        }
    }
    res.push_str(remaining);
    res
}

fn replace_code_blocks(input: &str) -> String {
    let mut res = String::with_capacity(input.len());
    let mut remaining = input;
    while let Some(start) = find_case_insensitive(remaining, "<pre>") {
        res.push_str(&remaining[..start]);
        let after = &remaining[start + 5..];
        if let Some(close_pos) = find_case_insensitive(after, "</pre>") {
            let inner = &after[..close_pos];
            let code_clean = inner.replace("<code>", "").replace("</code>", "");
            res.push_str("\n```\n");
            res.push_str(code_clean.trim());
            res.push_str("\n```\n");
            remaining = &after[close_pos + 6..];
        } else {
            res.push_str(&remaining[start..]);
            remaining = "";
            break;
        }
    }
    res.push_str(remaining);
    res
}

fn replace_inline_code(input: &str) -> String {
    let mut res = String::with_capacity(input.len());
    let mut remaining = input;
    while let Some(start) = find_case_insensitive(remaining, "<code>") {
        res.push_str(&remaining[..start]);
        let after = &remaining[start + 6..];
        if let Some(close_pos) = find_case_insensitive(after, "</code>") {
            let inner = &after[..close_pos];
            res.push('`');
            res.push_str(inner);
            res.push('`');
            remaining = &after[close_pos + 7..];
        } else {
            res.push_str(&remaining[start..]);
            remaining = "";
            break;
        }
    }
    res.push_str(remaining);
    res
}

fn replace_links(input: &str) -> String {
    let mut res = String::with_capacity(input.len());
    let mut remaining = input;

    while let Some(start) = find_case_insensitive(remaining, "<a ") {
        res.push_str(&remaining[..start]);
        let after = &remaining[start..];
        if let Some(tag_end) = after.find('>') {
            let tag_attrs = &after[..tag_end];
            let href = extract_attribute(tag_attrs, "href");
            let inner_start = &after[tag_end + 1..];
            if let Some(close_pos) = find_case_insensitive(inner_start, "</a>") {
                let inner_text = &inner_start[..close_pos];
                if let Some(href_val) = href {
                    if !href_val.is_empty() && !inner_text.trim().is_empty() {
                        res.push_str(&format!("[{}]({})", inner_text.trim(), href_val));
                    } else {
                        res.push_str(inner_text);
                    }
                } else {
                    res.push_str(inner_text);
                }
                remaining = &inner_start[close_pos + 4..];
            } else {
                res.push_str(&after[..tag_end + 1]);
                remaining = &after[tag_end + 1..];
            }
        } else {
            res.push_str(after);
            remaining = "";
            break;
        }
    }
    res.push_str(remaining);
    res
}

fn extract_attribute(tag: &str, attr: &str) -> Option<String> {
    let pattern = format!("{attr}=\"");
    if let Some(start) = find_case_insensitive(tag, &pattern) {
        let val_start = &tag[start + pattern.len()..];
        if let Some(end) = val_start.find('"') {
            return Some(val_start[..end].to_string());
        }
    }
    let pattern_single = format!("{attr}='");
    if let Some(start) = find_case_insensitive(tag, &pattern_single) {
        let val_start = &tag[start + pattern_single.len()..];
        if let Some(end) = val_start.find('\'') {
            return Some(val_start[..end].to_string());
        }
    }
    None
}

fn decode_entities(input: &str) -> String {
    let out = input
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&mdash;", "—")
        .replace("&ndash;", "–")
        .replace("&hellip;", "…");

    // Handle &#123; and &#x7b; numeric entities
    let mut res = String::with_capacity(out.len());
    let mut remaining = out.as_str();

    while let Some(amp_pos) = remaining.find("&#") {
        res.push_str(&remaining[..amp_pos]);
        let after = &remaining[amp_pos + 2..];
        if let Some(semi_pos) = after.find(';') {
            let code_str = &after[..semi_pos];
            let decoded_char = if let Some(hex_str) = code_str.strip_prefix('x').or_else(|| code_str.strip_prefix('X')) {
                u32::from_str_radix(hex_str, 16).ok().and_then(char::from_u32)
            } else {
                code_str.parse::<u32>().ok().and_then(char::from_u32)
            };

            if let Some(ch) = decoded_char {
                res.push(ch);
                remaining = &after[semi_pos + 1..];
            } else {
                res.push_str("&#");
                remaining = after;
            }
        } else {
            res.push_str("&#");
            remaining = after;
        }
    }
    res.push_str(remaining);
    res
}

fn clean_whitespace(input: &str) -> String {
    let mut lines = Vec::new();
    let mut consecutive_empty = 0;

    for line in input.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            consecutive_empty += 1;
            if consecutive_empty <= 1 {
                lines.push("");
            }
        } else {
            consecutive_empty = 0;
            lines.push(trimmed);
        }
    }

    lines.join("\n").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_html_to_markdown_basic() {
        let html = r#"
            <!DOCTYPE html>
            <html>
            <head><title>Test Page</title><style>.foo { color: red; }</style></head>
            <body>
                <script>alert("bad");</script>
                <h1>Main Heading</h1>
                <p>Hello, this is a <strong>bold</strong> and <em>italic</em> test.</p>
                <p>Here is a link: <a href="https://example.com">Example</a>.</p>
                <ul>
                    <li>First item</li>
                    <li>Second item</li>
                </ul>
                <pre><code>let x = 42;</code></pre>
            </body>
            </html>
        "#;
        let md = html_to_markdown(html, true);
        assert!(!md.contains("alert"));
        assert!(!md.contains("color: red"));
        assert!(md.contains("# Main Heading"));
        assert!(md.contains("**bold**"));
        assert!(md.contains("*italic*"));
        assert!(md.contains("[Example](https://example.com)"));
        assert!(md.contains("- First item"));
        assert!(md.contains("```\nlet x = 42;\n```"));
    }

    #[test]
    fn test_entity_decoding() {
        let text = "Tom &amp; Jerry &lt;3 cheese &mdash; &quot;quote&#39; &#x41;&#66;";
        let decoded = decode_entities(text);
        assert_eq!(decoded, "Tom & Jerry <3 cheese — \"quote' AB");
    }
}
