//! Env sanitization — applied to **every** spawn, sandboxed or not.
//!
//! Contract (sandbox-model.md §4): never inherit host env wholesale.
//! Allowlist wins on name, but the denylist (matched on uppercased name
//! **and** value) always wins. All matching is case-insensitive.

/// Names kept from the host env (exact match, case-insensitive).
const ALLOWLIST: &[&str] = &[
    "PATH", "LANG", "TERM", "HOME", "TMPDIR", "USER", "SHELL",
    "COLORTERM", "EDITOR", "VISUAL", "TZ",
    // toolchain vars the workspace needs
    "CARGO_HOME", "GOPATH", "GOCACHE", "NVM_DIR", "NODE_ENV",
    "PYTHONPATH", "VIRTUAL_ENV", "RUSTUP_HOME", "JAVA_HOME",
];

/// Prefixes kept (e.g. `LC_*` locale vars).
const ALLOW_PREFIXES: &[&str] = &["LC_"];

/// Name/value patterns that always strip — matched against the
/// **uppercased** name and value, so `database_url` and `AWS_*` die the same
/// as their uppercase forms (Windows env is case-insensitive).
const DENY_PATTERNS: &[&str] = &[
    "_KEY", "_TOKEN", "_SECRET", "_PASSWORD", "_URL", "_URI", "_DSN",
    "PRIVATE", "AWS_", "GITHUB_", "OPENAI_", "ANTHROPIC_",
    "DATABASE_URL", "MONGODB_URI", "REDIS_AUTH", "SENTRY_DSN", "COOKIE",
];

/// Filter the host env to a sanitized map. `extra` entries are appended
/// post-sanitization (they're trusted — the caller built them).
pub fn sanitize_env(
    extra: &[(String, String)],
) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (name, value) in std::env::vars() {
        let uname = name.to_uppercase();
        let uvalue = value.to_uppercase();
        // Denylist wins — check name AND value.
        if DENY_PATTERNS.iter().any(|p| uname.contains(p) || uvalue.contains(p)) {
            continue;
        }
        // Allowlist by exact name or prefix.
        let allowed = ALLOWLIST.iter().any(|a| uname == *a)
            || ALLOW_PREFIXES.iter().any(|p| uname.starts_with(p));
        if allowed {
            out.push((name, value));
        }
    }
    // Caller-supplied vars ride in post-sanitization.
    out.extend(extra.iter().cloned());
    out
}

/// Is a name denied by the denylist? Exposed for `bash` tool pre-flight.
pub fn is_denied(name: &str, value: &str) -> bool {
    let uname = name.to_uppercase();
    let uvalue = value.to_uppercase();
    DENY_PATTERNS.iter().any(|p| uname.contains(p) || uvalue.contains(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_secrets_case_insensitive() {
        let out = sanitize_env(&[]);
        let names: Vec<&str> = out.iter().map(|(n, _)| n.as_str()).collect();
        // Common secrets must never survive.
        for forbidden in ["OPENAI_API_KEY", "AWS_SECRET_ACCESS_KEY", "DATABASE_URL"] {
            assert!(!names.contains(&forbidden), "{forbidden} leaked");
        }
    }

    #[test]
    fn lowercase_database_url_stripped() {
        // is_denied matches on uppercased name+value.
        assert!(is_denied("database_url", "postgres://u:p@h/db"));
        assert!(is_denied("my_secret", "x"));
        assert!(is_denied("anything", "contains_AWS_thing"));
        assert!(!is_denied("PATH", "/usr/bin"));
        assert!(!is_denied("CARGO_HOME", "/home/u/.cargo"));
    }

    #[test]
    fn keeps_toolchain_and_locale() {
        std::env::set_var("TEST_AGENT_CARGO_HOME_X", "v");
        let out = sanitize_env(&[("FORCED".into(), "1".into())]);
        assert!(out.iter().any(|(n, _)| n == "FORCED"));
    }
}
