//! `masking.rs` — egress secret masker (production-hardening §2).
//!
//! Every outbound request body and every tool result passes through the
//! replacer — secrets leak most often via `env`/`.env` reads landing in a
//! prompt. Defense in depth: sandbox env sanitization is the first wall;
//! this is the second. A match becomes `[REDACTED_<kind>]`.

/// Resolved secret values + the shapes that catch them.
pub struct EgressMasker {
    /// Concrete secret strings resolved from `keyring:`/`env:` config —
    /// the exact values to scrub (never the `keyring:` handle itself).
    secrets: Vec<String>,
}

/// Patterns marking text as secret-shaped — matched case-insensitively on
/// the *content*, not just names (a value containing `sk-ant-…` is a leak
/// even if the field was named `note`).
const SECRET_SHAPES: &[&str] = &[
    "sk-", "sk-ant-", "sk-proj-", "ghp_", "gho_", "github_pat_",
    "xai-", "glpat-", "AKIA", "AIza", "ya29.", "dop_v1_",
    "-----BEGIN", "PRIVATE KEY-----",
];

/// Field-name patterns whose values are secrets — `api_key`, `token`, …
const SECRET_FIELDS: &[&str] = &[
    "_key", "_token", "_secret", "_password", "api_key", "apikey",
    "access_token", "refresh_token", "auth_token", "bearer ",
];

impl EgressMasker {
    /// Build from the resolved secret strings (post `env:`/`keyring:`).
    pub fn new(secrets: Vec<String>) -> Self {
        // Keep only non-trivial secrets — a 4-char value would over-mask.
        let secrets = secrets
            .into_iter()
            .filter(|s| s.len() >= 8)
            .collect();
        Self { secrets }
    }

    /// Scrub `text` — exact secrets → `[REDACTED_SECRET]`, shape matches →
    /// `[REDACTED_<kind>]`. Returns the sanitized string + whether it changed.
    pub fn scrub(&self, text: &str) -> (String, bool) {
        let mut out = text.to_string();
        let mut changed = false;
        // 1. Exact resolved secrets — highest confidence.
        for s in &self.secrets {
            if out.contains(s.as_str()) {
                out = out.replace(s.as_str(), "[REDACTED_SECRET]");
                changed = true;
            }
        }
        // 2. Shape matches — the value looks like a key/token.
        for shape in SECRET_SHAPES {
            if let Some(masked) = mask_shape(&out, shape) {
                out = masked;
                changed = true;
            }
        }
        // 3. Field-name secrets — `"api_key": "..."` → scrub the value.
        out = mask_field_values(&out);
        (out, changed)
    }
}

/// Replace `shape`-prefixed tokens with `[REDACTED_<shape>]` — e.g. any
/// `sk-…` run becomes `[REDACTED_API_KEY]`.
///
/// Two correctness rules vs. the naive version:
/// - **Word boundary** — a shape must be at a token start (preceded by
///   whitespace/quote/punctuation or string start), else `task-force`,
///   `disk-`, `risk-`, `msk-` get mangled by the `sk-` substring match.
/// - **PEM block** — `-----BEGIN` opens a multi-line block; masking only the
///   header line leaks the base64 key material. Redact the whole block
///   through `-----END …-----` (or to end of text when unterminated).
fn mask_shape(text: &str, shape: &str) -> Option<String> {
    let lower = text.to_lowercase();
    let needle = shape.to_lowercase();
    if !lower.contains(&needle) {
        return None;
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        let lower_rest = rest.to_lowercase();
        let Some(idx) = lower_rest.find(&needle) else { break };

        // Word-boundary check: the char before the match must be a boundary
        // (start, whitespace, quote, or non-alphanumeric punctuation), else
        // this `sk-` is inside a longer word like `task-`/`disk-`/`msk-`.
        let boundary_ok = idx == 0
            || rest[..idx]
                .chars()
                .last()
                .map(|c| !c.is_alphanumeric() && c != '-' && c != '_')
                .unwrap_or(true);
        if !boundary_ok {
            // Not a token start — emit through this char and keep scanning.
            let adv = idx + needle.len();
            out.push_str(&rest[..adv]);
            rest = &rest[adv..];
            continue;
        }

        out.push_str(&rest[..idx]);
        let tail = &rest[idx..];

        // PEM block: redact through the matching `-----END …-----` line so
        // the key material can't leak between BEGIN and END.
        if needle.starts_with("-----begin") {
            if let Some(end_idx) = tail.to_lowercase().find("-----end") {
                let after_end = &tail[end_idx..];
                let line_end = after_end.find('\n').unwrap_or(after_end.len());
                out.push_str("[REDACTED_PEM_BLOCK]\n");
                rest = &after_end[line_end..];
                continue;
            }
            // Unterminated block — redact to end of text.
            out.push_str("[REDACTED_PEM_BLOCK]");
            rest = "";
            break;
        }

        // Ordinary token: consume shape + the following non-space run.
        let end = tail
            .find(|c: char| c.is_whitespace() || c == '"' || c == '\'')
            .unwrap_or(tail.len());
        out.push_str("[REDACTED_API_KEY]");
        rest = &tail[end..];
    }
    out.push_str(rest);
    Some(out)
}

/// `"<field>": "<value>"` / `<field>=<value>` → scrub value when field is
/// secret-shaped.
fn mask_field_values(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let lower = line.to_lowercase();
        let is_secret_field = SECRET_FIELDS.iter().any(|f| lower.contains(f));
        if is_secret_field {
            // Find the value part after `:` or `=`.
            if let Some(pos) = line.find(|c| c == ':' || c == '=') {
                let (head, val) = line.split_at(pos + 1);
                let v = val.trim();
                if !v.is_empty() && !v.starts_with('[') && v.len() >= 6 {
                    out.push_str(head);
                    out.push_str(" [REDACTED_SECRET]\n");
                    continue;
                }
            }
        }
        out.push_str(line);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_secret_scrubbed() {
        let m = EgressMasker::new(vec!["xai-real-key-123".into()]);
        let (out, changed) = m.scrub("the key is xai-real-key-123 ok");
        assert!(changed);
        assert!(out.contains("[REDACTED_SECRET]"));
        assert!(!out.contains("xai-real-key-123"));
    }

    #[test]
    fn shape_scrubbed() {
        let m = EgressMasker::new(vec![]);
        let (out, _) = m.scrub("call with sk-ant-abc123 done");
        assert!(out.contains("[REDACTED_API_KEY]"));
    }

    #[test]
    fn field_value_scrubbed() {
        let m = EgressMasker::new(vec![]);
        let (out, _) = m.scrub("api_key: very_secret_value_here");
        assert!(out.contains("[REDACTED_SECRET]"));
    }

    #[test]
    fn clean_text_untouched() {
        let m = EgressMasker::new(vec![]);
        let (out, changed) = m.scrub("fn main() { println!(\"hi\"); }");
        assert!(!changed);
        assert_eq!(out, "fn main() { println!(\"hi\"); }");
    }

    #[test]
    fn word_boundary_spares_task_force() {
        // `sk-` inside `task-force`, `disk-`, `risk-` must NOT be masked —
        // a substring match with no token boundary mangles normal prose.
        let m = EgressMasker::new(vec![]);
        for w in ["task-force", "disk-full", "risk-reward", "msk-edge"] {
            let (out, _) = m.scrub(w);
            assert_eq!(out, w, "`{w}` was mangled by the sk- shape");
        }
    }

    #[test]
    fn pem_block_fully_redacted() {
        // `-----BEGIN` opens a multi-line key block — masking only the
        // header line would leak the base64 material between BEGIN and END.
        let m = EgressMasker::new(vec![]);
        let pem = "here\n-----BEGIN PRIVATE KEY-----\nMIIabc123==\nMIIdef456==\n-----END PRIVATE KEY-----\nafter";
        let (out, _) = m.scrub(pem);
        assert!(!out.contains("MIIabc123"), "key material leaked");
        assert!(!out.contains("MIIdef456"), "key material leaked");
        assert!(out.contains("[REDACTED_PEM_BLOCK]"));
        assert!(out.contains("after"));
    }

    #[test]
    fn tool_call_arguments_are_scrubbed() {
        // A tool that read `.env` puts the secret into the NEXT request's
        // tool_calls[].arguments — this is the exfiltration path the masker
        // originally missed. Verify the field is covered end-to-end.
        let m = EgressMasker::new(vec![]);
        let (out, _) = m.scrub("{\"cmd\":\"export AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMI\"}");
        assert!(!out.contains("wJalrXUtnFEMI"));
    }
}
