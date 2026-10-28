//! API の鍵。repo の外の file に 1 行 1 個で置き、`Authorization: Bearer <鍵>` で受ける。

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

/// `Authorization` の値が登録済みの鍵のどれかと一致するか。
#[must_use]
pub fn authorized(header: Option<&str>, keys: &[String]) -> bool {
    let Some(given) = header.and_then(|h| h.strip_prefix("Bearer ")) else {
        return false;
    };
    !given.is_empty() && keys.iter().any(|k| same(k.as_bytes(), given.as_bytes()))
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
            authorized(Some("Bearer key-two"), &keys),
            "登録済みの鍵は通す"
        );
    }

    #[test]
    fn rejects_missing_wrong_or_bare() {
        let keys = vec!["key-one".to_owned()];
        assert!(!authorized(None, &keys), "鍵の無い要求は拒む");
        assert!(!authorized(Some("Bearer key-two"), &keys), "違う鍵は拒む");
        assert!(!authorized(Some("key-one"), &keys), "Bearer の無い値は拒む");
        assert!(
            !authorized(Some("Bearer key-on"), &keys),
            "前方一致では通さない"
        );
        assert!(
            !authorized(Some("Bearer "), &[String::new()]),
            "空の鍵は通さない"
        );
    }
}
