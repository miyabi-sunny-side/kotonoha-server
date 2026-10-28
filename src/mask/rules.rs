//! 型の決まった機密 (mail・IP・URL・電話番号・鍵やパスワード) を正規表現で拾う。

use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::LazyLock;

use regex::Regex;

use super::{Candidate, Kind};

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("built-in pattern compiles")
}

static MAIL: LazyLock<Regex> =
    LazyLock::new(|| re(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)*\.[A-Za-z]{2,}"));
static IPV4: LazyLock<Regex> = LazyLock::new(|| re(r"(?:\d{1,3}\.){3}\d{1,3}"));
static IPV6: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?:[0-9A-Fa-f]{0,4}:){2,7}[0-9A-Fa-f]{0,4}(?:(?:\.\d{1,3}){3})?"));
static URL: LazyLock<Regex> =
    LazyLock::new(|| re(r#"[A-Za-z][A-Za-z0-9+.-]*://[^\s"'<>`「」『』（）【】、。，．]+"#));
static PHONE_SPLIT: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?:\+81[-‐－ ]?|0)\d{1,4}[-‐－(（ ]\d{1,4}[-‐－)） ]\d{3,4}"));
static PHONE_PLAIN: LazyLock<Regex> = LazyLock::new(|| re(r"\+81\d{9,10}|0\d{9,10}"));
static TOKEN: LazyLock<Regex> = LazyLock::new(|| {
    re(concat!(
        r"sk-[A-Za-z0-9_-]{20,}",
        r"|(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{36,}|github_pat_[A-Za-z0-9_]{22,}",
        r"|(?:AKIA|ASIA)[0-9A-Z]{16}",
        r"|xox[abposr]-[A-Za-z0-9-]{10,}",
        r"|AIza[0-9A-Za-z_-]{35}",
        r"|eyJ[A-Za-z0-9_-]{5,}\.eyJ[A-Za-z0-9_-]{5,}\.[A-Za-z0-9_-]+",
        r"|(?s)-----BEGIN [A-Z ]*PRIVATE KEY-----.*?-----END [A-Z ]*PRIVATE KEY-----",
    ))
});
static BEARER: LazyLock<Regex> = LazyLock::new(|| re(r"Bearer\s+([A-Za-z0-9._~+/-]{16,}=*)"));
static ASSIGNED: LazyLock<Regex> = LazyLock::new(|| {
    re(concat!(
        r#"(?i)(?:pass(?:word|wd|phrase)?|pwd|secret|token|api[_-]?key|access[_-]?key"#,
        r#"|private[_-]?key|credential|auth[_-]?key|パスワード|暗証番号|トークン)"#,
        r#"[A-Za-z0-9_-]*["']?\s*[:=：]\s*"#,
        r#"(?:"([^"\n]*)"|'([^'\n]*)'|([^\s"'`,;()\[\]{}]+))"#,
    ))
});
/// 右辺がコードの参照に見えるもの: `config.token` のような経路と `API_SECRET` のような定数名。
static CODE_REF: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"^(?:[A-Za-z_$][A-Za-z0-9_$]*(?:\.[A-Za-z_$][A-Za-z0-9_$]*)+|[A-Z][A-Z0-9]*(?:_[A-Z0-9]+)+)$",
    )
});

/// 型の決まった候補を全て探す。重なりの解消は呼び出し側がする。
///
/// # Panics
/// 組み込みの正規表現が壊れているときだけ。
#[must_use]
pub fn find(text: &str) -> Vec<Candidate> {
    let mut found = Vec::new();
    let mut push = |start: usize, end: usize, kind: Kind| {
        found.push(Candidate { start, end, kind });
    };
    for m in MAIL.find_iter(text) {
        push(m.start(), m.end(), Kind::Mail);
    }
    for m in IPV4.find_iter(text) {
        let before = text[..m.start()].chars().next_back();
        let after = text[m.end()..].chars().take(2).collect::<Vec<_>>();
        let joined = before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '.')
            || after.first().is_some_and(char::is_ascii_alphanumeric)
            || (after.first() == Some(&'.') && after.get(1).is_some_and(char::is_ascii_digit));
        if !joined && m.as_str().parse::<Ipv4Addr>().is_ok() {
            push(m.start(), m.end(), Kind::Ip);
        }
    }
    for m in IPV6.find_iter(text) {
        let before = text[..m.start()].chars().next_back();
        let after = text[m.end()..].chars().next();
        let joined = before.is_some_and(|c| c.is_ascii_alphanumeric() || "_:.".contains(c))
            || after.is_some_and(|c| c.is_ascii_alphanumeric() || "_:".contains(c));
        let s = m.as_str();
        if !joined && s.bytes().any(|b| b.is_ascii_digit()) && s.parse::<Ipv6Addr>().is_ok() {
            push(m.start(), m.end(), Kind::Ipv6);
        }
    }
    for m in URL.find_iter(text) {
        let url = m.as_str().trim_end_matches(|c| ".,;:!?)]}'".contains(c));
        push(m.start(), m.start() + url.len(), Kind::Url);
    }
    for m in PHONE_SPLIT
        .find_iter(text)
        .chain(PHONE_PLAIN.find_iter(text))
    {
        let before = text[..m.start()].chars().next_back();
        let after = text[m.end()..].chars().next();
        let joined = before.is_some_and(|c| c.is_ascii_alphanumeric())
            || after.is_some_and(|c| c.is_ascii_alphanumeric());
        if !joined && phone_digits_fit(m.as_str()) {
            push(m.start(), m.end(), Kind::Phone);
        }
    }
    for m in TOKEN.find_iter(text) {
        let before = text[..m.start()].chars().next_back();
        if !before.is_some_and(|c| c.is_ascii_alphanumeric()) {
            push(m.start(), m.end(), Kind::Secret);
        }
    }
    for c in BEARER.captures_iter(text) {
        let m = c.get(1).expect("group 1 always takes part");
        push(m.start(), m.end(), Kind::Secret);
    }
    for c in ASSIGNED.captures_iter(text) {
        let (m, quoted) = match (c.get(1).or(c.get(2)), c.get(3)) {
            (Some(m), _) => (m, true),
            (None, Some(m)) => (m, false),
            (None, None) => continue,
        };
        let followed_by_call = text[m.end()..].starts_with('(');
        if looks_like_value(m.as_str(), quoted) && !followed_by_call {
            push(m.start(), m.end(), Kind::Secret);
        }
    }
    found
}

/// 国内は 0 から 10・11 桁、+81 は続く 9・10 桁。
fn phone_digits_fit(s: &str) -> bool {
    let digits: String = s.chars().filter(char::is_ascii_digit).collect();
    match s.strip_prefix("+81") {
        Some(_) => (11..=12).contains(&digits.len()),
        None => digits.starts_with('0') && (10..=11).contains(&digits.len()),
    }
}

/// 代入の右辺が値に見えるか。変数の参照・null・placeholder は値としない。
fn looks_like_value(value: &str, quoted: bool) -> bool {
    let v = value.trim();
    if v.chars().count() < 3 {
        return false;
    }
    if v.chars().all(|c| c == '*' || c == 'x' || c == 'X') {
        return false;
    }
    if ["${", "{{", "<", "%("].iter().any(|p| v.starts_with(p)) {
        return false;
    }
    if quoted {
        return true;
    }
    let keyword = ["null", "nil", "none", "true", "false", "undefined"]
        .iter()
        .any(|k| v.eq_ignore_ascii_case(k));
    !keyword && !CODE_REF.is_match(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(text: &str) -> Vec<(&str, Kind)> {
        find(text)
            .into_iter()
            .map(|c| (&text[c.start..c.end], c.kind))
            .collect()
    }

    fn has(text: &str, want: &str, kind: Kind) {
        let got = found(text);
        assert!(
            got.contains(&(want, kind)),
            "{want:?} を {kind:?} で拾う: {got:?}"
        );
    }

    fn none_of(text: &str, kind: Kind) {
        let got = found(text);
        assert!(
            got.iter().all(|(_, k)| *k != kind),
            "{text:?} に {kind:?} は無い: {got:?}"
        );
    }

    #[test]
    fn mail() {
        has(
            "連絡は miyabi.ooh@gmail.com まで",
            "miyabi.ooh@gmail.com",
            Kind::Mail,
        );
        has(
            "<taro.yamada+dev@sub.acme.co.jp>",
            "taro.yamada+dev@sub.acme.co.jp",
            Kind::Mail,
        );
        has(
            "宛先：hanako@example.org。",
            "hanako@example.org",
            Kind::Mail,
        );
    }

    #[test]
    fn ipv4() {
        has("host = \"10.20.30.40\"", "10.20.30.40", Kind::Ip);
        has("192.168.1.101:5012", "192.168.1.101", Kind::Ip);
        has("(127.0.0.1)", "127.0.0.1", Kind::Ip);
        none_of("version 1.2.3", Kind::Ip);
        none_of("999.1.1.1", Kind::Ip);
        none_of("1.2.3.4.5", Kind::Ip);
    }

    #[test]
    fn ipv6() {
        has(
            "addr fe80::1ff:fe23:4567:890a%eth0",
            "fe80::1ff:fe23:4567:890a",
            Kind::Ipv6,
        );
        has(
            "[2001:db8:85a3::8a2e:370:7334]:443",
            "2001:db8:85a3::8a2e:370:7334",
            Kind::Ipv6,
        );
        has("bind ::1 only", "::1", Kind::Ipv6);
        none_of("use std::fs::read; Vec::new(); add::<u8>", Kind::Ipv6);
        none_of("12:34:56", Kind::Ipv6);
        none_of("abc::def", Kind::Ipv6);
    }

    #[test]
    fn url() {
        has(
            "see https://wiki.acme.local/pages/123?x=1.",
            "https://wiki.acme.local/pages/123?x=1",
            Kind::Url,
        );
        has(
            "「http://10.0.0.5:8080/api」",
            "http://10.0.0.5:8080/api",
            Kind::Url,
        );
        has(
            "jdbc:postgresql://db01.corp:5432/sales",
            "postgresql://db01.corp:5432/sales",
            Kind::Url,
        );
        has(
            "(ssh://git@git.acme.local/repo)",
            "ssh://git@git.acme.local/repo",
            Kind::Url,
        );
    }

    #[test]
    fn phone() {
        has("TEL: 03-1234-5678", "03-1234-5678", Kind::Phone);
        has("携帯 090-1234-5678 まで", "090-1234-5678", Kind::Phone);
        has("09012345678", "09012345678", Kind::Phone);
        has("+81-90-1234-5678", "+81-90-1234-5678", Kind::Phone);
        has("03(1234)5678", "03(1234)5678", Kind::Phone);
        has("0120-123-456", "0120-123-456", Kind::Phone);
        none_of("id 123-4567", Kind::Phone);
        none_of("2026-10-28", Kind::Phone);
        none_of("0123456789012345", Kind::Phone);
    }

    #[test]
    fn secret_by_prefix() {
        has(
            "key=sk-proj-AbCdEf0123456789XyZ0123",
            "sk-proj-AbCdEf0123456789XyZ0123",
            Kind::Secret,
        );
        has(
            "ghp_0123456789abcdefABCDEF0123456789abcd",
            "ghp_0123456789abcdefABCDEF0123456789abcd",
            Kind::Secret,
        );
        has("AKIAIOSFODNN7EXAMPLE", "AKIAIOSFODNN7EXAMPLE", Kind::Secret);
        has(
            "xoxb-123456789012-abcdefABCDEF",
            "xoxb-123456789012-abcdefABCDEF",
            Kind::Secret,
        );
        has(
            "AIzaSyA-0123456789abcdefghijklmnopqrstu",
            "AIzaSyA-0123456789abcdefghijklmnopqrstu",
            Kind::Secret,
        );
        has(
            "Bearer eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.c2lnbmF0dXJl",
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.c2lnbmF0dXJl",
            Kind::Secret,
        );
        let pem = "-----BEGIN RSA PRIVATE KEY-----\nMIIBOgIBAAJBAK\n-----END RSA PRIVATE KEY-----";
        has(&format!("x\n{pem}\n"), pem, Kind::Secret);
    }

    #[test]
    fn secret_by_assignment() {
        has("DB_PASSWORD=hunter2", "hunter2", Kind::Secret);
        has("password: \"p@ss w0rd!\"", "p@ss w0rd!", Kind::Secret);
        has(
            "const apiKey = 'abc123def456';",
            "abc123def456",
            Kind::Secret,
        );
        has(
            "\"client_secret\": \"s3cr3t-value\"",
            "s3cr3t-value",
            Kind::Secret,
        );
        has("パスワード：Tokyo2026!", "Tokyo2026!", Kind::Secret);
        has("token=abcd1234 next", "abcd1234", Kind::Secret);
        none_of("password = get_password()", Kind::Secret);
        none_of("token = config.token", Kind::Secret);
        none_of("password: null", Kind::Secret);
        none_of("api_key = os.environ[\"API_KEY\"]", Kind::Secret);
        none_of("passwords are stored hashed", Kind::Secret);
        none_of("{ \"X-Api-Key\": STRIPE_SECRET }", Kind::Secret);
        has("PASSWORD=HUNTER2", "HUNTER2", Kind::Secret);
    }
}
