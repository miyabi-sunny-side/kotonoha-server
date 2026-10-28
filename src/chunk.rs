//! 長い入力を、Gemma の文脈に収まる大きさへ行の境目で分ける。

/// token 数の控えめな見積もり。非 ASCII の文字は 1 文字 1 token、ASCII は 3 byte で 1 token。
#[must_use]
pub fn estimate_tokens(text: &str) -> usize {
    Count::of(text).tokens()
}

/// 見積もりの元になる数。片を伸ばすたびに数え直さず足していく。
#[derive(Clone, Copy, Default)]
struct Count {
    ascii: usize,
    other: usize,
    bytes: usize,
}

impl Count {
    fn of(text: &str) -> Self {
        let ascii = text.bytes().filter(u8::is_ascii).count();
        let other = text.chars().filter(|c| !c.is_ascii()).count();
        Self {
            ascii,
            other,
            bytes: text.len(),
        }
    }

    fn plus(self, more: Self) -> Self {
        Self {
            ascii: self.ascii + more.ascii,
            other: self.other + more.other,
            bytes: self.bytes + more.bytes,
        }
    }

    fn tokens(self) -> usize {
        self.other + self.ascii.div_ceil(3)
    }
}

/// 各片の見積もりが `budget` 以下になるよう、なるべく行の境目で分ける。
/// 片を全て繋ぐと元の文字列に戻る。
#[must_use]
pub fn split(text: &str, budget: usize) -> Vec<&str> {
    let mut pieces = Vec::new();
    let mut start = 0;
    let mut count = Count::default();
    for line in text.split_inclusive('\n') {
        let size = Count::of(line);
        if count.plus(size).tokens() > budget && count.tokens() > 0 {
            let end = start + count.bytes;
            pieces.push(&text[start..end]);
            start = end;
            count = Count::default();
        }
        if size.tokens() <= budget {
            count = count.plus(size);
            continue;
        }
        pieces.extend(split_line(line, budget));
        start += line.len();
    }
    if start < text.len() {
        pieces.push(&text[start..]);
    }
    pieces
}

/// 1 行が上限を超えるときは文字の境目で分ける。
fn split_line(line: &str, budget: usize) -> Vec<&str> {
    let mut pieces = Vec::new();
    let mut start = 0;
    let mut count = Count::default();
    for (i, c) in line.char_indices() {
        let size = Count::of(&line[i..i + c.len_utf8()]);
        if count.tokens() > 0 && count.plus(size).tokens() > budget {
            pieces.push(&line[start..i]);
            start = i;
            count = Count::default();
        }
        count = count.plus(size);
    }
    if start < line.len() {
        pieces.push(&line[start..]);
    }
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimates_japanese_and_ascii() {
        assert_eq!(estimate_tokens("日本語"), 3, "非 ASCII は 1 文字 1 token");
        assert_eq!(estimate_tokens("abcdef"), 2, "ASCII は 3 byte で 1 token");
        assert_eq!(estimate_tokens("ab日"), 2, "端数は切り上げる");
        assert_eq!(estimate_tokens(""), 0, "空は 0");
    }

    #[test]
    fn short_text_is_one_piece() {
        assert_eq!(split("a\nb\n", 100), ["a\nb\n"], "上限内なら分けない");
        assert!(split("", 100).is_empty(), "空なら片は無い");
    }

    #[test]
    fn splits_on_line_boundaries() {
        let text = "aaa\nbbb\nccc\n";
        let pieces = split(text, 3);
        assert_eq!(pieces, ["aaa\nbbb\n", "ccc\n"], "行の境目で分ける");
        assert_eq!(pieces.concat(), text, "繋ぐと元に戻る");
    }

    #[test]
    fn splits_a_long_line_on_char_boundaries() {
        let text = "あいうえおかきくけこ";
        let pieces = split(text, 4);
        assert_eq!(
            pieces,
            ["あいうえ", "おかきく", "けこ"],
            "長い行は文字の境目で分ける"
        );
    }

    #[test]
    fn every_piece_fits_budget() {
        let text = "短い行\n".to_owned() + &"x".repeat(50) + "\n日本語の行です\n";
        for piece in split(&text, 6) {
            assert!(estimate_tokens(piece) <= 6, "{piece:?} が上限を超える");
        }
        assert_eq!(split(&text, 6).concat(), text, "繋ぐと元に戻る");
    }
}
