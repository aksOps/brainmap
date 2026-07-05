use regex::Regex;
use std::sync::OnceLock;

fn patterns() -> &'static [Regex] {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        [
            r"(?i)\b(api[_-]?key|token|secret|password)\s*[:=]\s*['\x22]?[A-Za-z0-9_\-./+=]{12,}",
            r"(?i)\bBearer\s+[A-Za-z0-9_\-./+=]{12,}",
            r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----",
            r"(?i)\b(cookie|authorization)\s*:\s*[^\n\r]{8,}",
            r"\bsk-[A-Za-z0-9_\-]{16,}",
            r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b",
        ]
        .into_iter()
        .map(|p| Regex::new(p).unwrap())
        .collect()
    })
}

pub fn redact(text: &str) -> String {
    let mut out = text.to_string();
    for re in patterns() {
        out = re.replace_all(&out, "[REDACTED]").to_string();
    }
    out
}

pub fn contains_secret(text: &str) -> bool {
    patterns().iter().any(|re| re.is_match(text))
}

pub fn sensitivity(text: &str) -> &'static str {
    let lower = text.to_lowercase();
    if contains_secret(text) {
        "secret"
    } else if lower.contains("private") || lower.contains("credential") || lower.contains("payment")
    {
        "private"
    } else {
        "personal"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_api_keys_and_bearer() {
        let out = redact("api_key=abcdef1234567890\nAuthorization: Bearer abcdef1234567890");
        assert!(out.contains("[REDACTED]"));
        assert!(!out.contains("abcdef1234567890"));
    }

    #[test]
    fn redacts_private_key() {
        let out = redact("-----BEGIN PRIVATE KEY-----\nabc\n-----END PRIVATE KEY-----");
        assert_eq!(out, "[REDACTED]");
    }
}
