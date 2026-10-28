//! API の鍵。repo の外の file に 1 行 1 個で置き、`Authorization: Bearer <鍵>`（OpenAI 形式）か
//! `x-api-key: <鍵>`（Anthropic 形式）で受ける。

/// 鍵の file を読む。空行と `#` で始まる行は飛ばす。
#[must_use]
pub fn parse_keys(content: &str) -> Vec<String> {
    content
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_owned)
        .collect()
}

/// `Authorization: Bearer` か `x-api-key` の値が、登録済みの鍵のどれかと一致するか。
#[must_use]
pub fn authorized(authorization: Option<&str>, x_api_key: Option<&str>, keys: &[String]) -> bool {
    let bearer = authorization.and_then(|h| h.strip_prefix("Bearer "));
    [bearer, x_api_key]
        .into_iter()
        .flatten()
        .any(|given| !given.is_empty() && keys.iter().any(|k| same(k.as_bytes(), given.as_bytes())))
}

/// 一致する長さまで時間を変えない比較。
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_one_key_per_line() {
        let keys = parse_keys("# cli\n  key-one  \n\nkey-two\n");
        assert_eq!(
            keys,
            ["key-one", "key-two"],
            "空行と注釈を飛ばし、前後の空白を落とす"
        );
    }

    #[test]
    fn accepts_registered_bearer() {
        let keys = vec!["key-one".to_owned(), "key-two".to_owned()];
        assert!(
            authorized(Some("Bearer key-two"), None, &keys),
            "登録済みの鍵は通す"
        );
    }

    #[test]
    fn accepts_registered_x_api_key() {
        let keys = vec!["key-one".to_owned()];
        assert!(
            authorized(None, Some("key-one"), &keys),
            "Anthropic 形式の x-api-key でも通す"
        );
        assert!(
            authorized(Some("Bearer wrong"), Some("key-one"), &keys),
            "どちらかが登録済みなら通す"
        );
    }

    #[test]
    fn rejects_missing_wrong_or_bare() {
        let keys = vec!["key-one".to_owned()];
        assert!(!authorized(None, None, &keys), "鍵の無い要求は拒む");
        assert!(
            !authorized(Some("Bearer key-two"), None, &keys),
            "違う鍵は拒む"
        );
        assert!(
            !authorized(None, Some("key-two"), &keys),
            "違う x-api-key は拒む"
        );
        assert!(
            !authorized(Some("key-one"), None, &keys),
            "Bearer の無い値は拒む"
        );
        assert!(
            !authorized(None, Some("Bearer key-one"), &keys),
            "x-api-key に Bearer は付けない"
        );
        assert!(
            !authorized(Some("Bearer key-on"), None, &keys),
            "前方一致では通さない"
        );
        assert!(
            !authorized(Some("Bearer "), Some(""), &[String::new()]),
            "空の鍵は通さない"
        );
    }
}
