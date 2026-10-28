//! Gemma の候補を入力と照らし、重なる候補は長い方を残す。

use std::cmp::Reverse;
use std::collections::{BTreeMap, HashSet};

use super::{Candidate, Item};

/// 候補の文字列が入力に現れる位置を全て探す。現れない物と 1 文字以下の物は捨てる。
#[must_use]
pub fn locate(text: &str, items: &[Item]) -> Vec<Candidate> {
    let mut found = Vec::new();
    for item in items {
        let needle = item.text.trim();
        if needle.chars().count() < 2 {
            continue;
        }
        found.extend(text.match_indices(needle).map(|(start, _)| Candidate {
            start,
            end: start + needle.len(),
            kind: item.kind,
        }));
    }
    found
}

/// 重なる候補は長い方 (同じ長さなら正規表現の区分) を残し、
/// 文字列ごとに一つの区分にして初出の順に返す。
#[must_use]
pub fn resolve(text: &str, mut candidates: Vec<Candidate>) -> Vec<Item> {
    candidates.sort_by_key(|c| (Reverse(c.end - c.start), !c.kind.by_rules(), c.start));
    // 採用した範囲は互いに重ならないので、開始位置で引けば隣だけ見れば足りる。
    let mut taken: BTreeMap<usize, Candidate> = BTreeMap::new();
    for c in candidates {
        let clashes = taken
            .range(..c.end)
            .next_back()
            .is_some_and(|(_, t)| t.end > c.start);
        if !clashes {
            taken.insert(c.start, c);
        }
    }
    let mut seen = HashSet::new();
    taken
        .values()
        .filter_map(|c| {
            let text = &text[c.start..c.end];
            seen.insert(text).then(|| Item {
                text: text.to_owned(),
                kind: c.kind,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::Kind;
    use super::*;

    fn item(text: &str, kind: Kind) -> Item {
        Item {
            text: text.into(),
            kind,
        }
    }

    fn at(text: &str, needle: &str, kind: Kind) -> Candidate {
        let start = text.find(needle).unwrap();
        Candidate {
            start,
            end: start + needle.len(),
            kind,
        }
    }

    #[test]
    fn locate_finds_every_occurrence() {
        let text = "田中と田中";
        let got = locate(text, &[item("田中", Kind::Person)]);
        assert_eq!(
            got.iter().map(|c| c.start).collect::<Vec<_>>(),
            [0, 9],
            "全ての出現を拾う"
        );
    }

    #[test]
    fn locate_drops_unseen_and_trivial() {
        let text = "株式会社アクメ の 林";
        let got = locate(
            text,
            &[
                item("アクメ商事", Kind::Company),
                item("林", Kind::Person),
                item("  ", Kind::Person),
                item(" 株式会社アクメ ", Kind::Company),
            ],
        );
        assert_eq!(
            got,
            vec![at(text, "株式会社アクメ", Kind::Company)],
            "入力に無い物・1 文字・空白だけは捨て、前後の空白は落とす"
        );
    }

    #[test]
    fn resolve_keeps_longer_on_overlap() {
        let text = "http://db01.acme.local/x と db01.acme.local";
        let url = at(text, "http://db01.acme.local/x", Kind::Url);
        let inner = Candidate {
            start: 7,
            end: 22,
            kind: Kind::Host,
        };
        let last = text.rfind("db01").unwrap();
        let alone = Candidate {
            start: last,
            end: last + 15,
            kind: Kind::Host,
        };
        assert_eq!(
            &text[alone.start..alone.end],
            "db01.acme.local",
            "単独の出現の位置"
        );
        let got = resolve(text, vec![inner, url, alone]);
        assert_eq!(
            got,
            vec![
                item("http://db01.acme.local/x", Kind::Url),
                item("db01.acme.local", Kind::Host),
            ],
            "URL の中の host は URL に含め、単独の出現は残す"
        );
    }

    #[test]
    fn resolve_drops_text_always_inside_longer() {
        let text = "株式会社アクメの件";
        let got = resolve(
            text,
            vec![
                at(text, "アクメ", Kind::Company),
                at(text, "株式会社アクメ", Kind::Company),
            ],
        );
        assert_eq!(
            got,
            vec![item("株式会社アクメ", Kind::Company)],
            "長い方を残す"
        );
    }

    #[test]
    fn resolve_prefers_rules_on_same_span() {
        let text = "x a@b.co y";
        let got = resolve(
            text,
            vec![
                at(text, "a@b.co", Kind::Person),
                at(text, "a@b.co", Kind::Mail),
            ],
        );
        assert_eq!(
            got,
            vec![item("a@b.co", Kind::Mail)],
            "同じ範囲なら正規表現の区分"
        );
    }

    #[test]
    fn resolve_partial_overlap_keeps_longer() {
        let text = "abcdefgh";
        let left = Candidate {
            start: 0,
            end: 5,
            kind: Kind::Project,
        };
        let right = Candidate {
            start: 3,
            end: 6,
            kind: Kind::Host,
        };
        let got = resolve(text, vec![right, left]);
        assert_eq!(got, vec![item("abcde", Kind::Project)], "交差も長い方");
    }

    #[test]
    fn resolve_orders_by_first_occurrence_and_dedups() {
        let text = "B A B";
        let got = resolve(
            text,
            vec![
                Candidate {
                    start: 4,
                    end: 5,
                    kind: Kind::Person,
                },
                Candidate {
                    start: 2,
                    end: 3,
                    kind: Kind::Person,
                },
                Candidate {
                    start: 0,
                    end: 1,
                    kind: Kind::Person,
                },
            ],
        );
        assert_eq!(
            got,
            vec![item("B", Kind::Person), item("A", Kind::Person)],
            "初出の順で一度ずつ"
        );
    }
}
