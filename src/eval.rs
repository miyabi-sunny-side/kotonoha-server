//! `/mask` の取りこぼしを測る。`eval/mask/<case>/` に入力の file と正解の `expected.json` を置く。

use std::collections::BTreeMap;
use std::path::Path;

use crate::mask::{Item, Kind};

pub struct Case {
    pub name: String,
    pub filename: String,
    pub text: String,
    pub expected: Vec<Item>,
}

/// 正解の 1 件が結果でどう扱われたか。
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// 同じ文字列・同じ区分で返った。
    Exact,
    /// 区分は違うが同じ文字列が返った (伏せられはする)。
    WrongType(Kind),
    /// 全ての出現がより長い返った文字列の中にある (伏せられはする)。
    Covered(String),
    Missed,
}

impl Outcome {
    #[must_use]
    pub fn masked(&self) -> bool {
        *self != Outcome::Missed
    }
}

/// 正解の各件を結果と照らす。
#[must_use]
pub fn score(text: &str, expected: &[Item], got: &[Item]) -> Vec<Outcome> {
    let spans: Vec<(usize, usize, &str)> = got
        .iter()
        .flat_map(|g| {
            text.match_indices(g.text.as_str())
                .map(|(start, s)| (start, start + s.len(), g.text.as_str()))
        })
        .collect();
    // 全ての出現を覆う、より長い返った文字列。
    let covering = |want: &str| -> Option<String> {
        let mut by = None;
        for (start, _) in text.match_indices(want) {
            let end = start + want.len();
            let (_, _, g) = spans.iter().find(|(s, e, _)| *s <= start && end <= *e)?;
            by.get_or_insert_with(|| (*g).to_owned());
        }
        by
    };
    expected
        .iter()
        .map(|want| {
            if let Some(same) = got.iter().find(|g| g.text == want.text) {
                if same.kind == want.kind {
                    Outcome::Exact
                } else {
                    Outcome::WrongType(same.kind)
                }
            } else if let Some(by) = covering(&want.text) {
                Outcome::Covered(by)
            } else {
                Outcome::Missed
            }
        })
        .collect()
}

/// 区分ごとの (伏せられた数, 区分も合った数, 正解の数)。
#[must_use]
pub fn recall_by_kind(
    expected: &[Item],
    outcomes: &[Outcome],
) -> BTreeMap<&'static str, (usize, usize, usize)> {
    let mut tally = BTreeMap::new();
    for (want, outcome) in expected.iter().zip(outcomes) {
        let entry = tally.entry(want.kind.name()).or_insert((0, 0, 0));
        entry.0 += usize::from(outcome.masked());
        entry.1 += usize::from(*outcome == Outcome::Exact);
        entry.2 += 1;
    }
    tally
}

/// `dir` の下の case を名前順に読む。
///
/// # Errors
/// file を読めない、または `expected.json` が壊れているとき。
pub fn load(dir: &Path) -> anyhow::Result<Vec<Case>> {
    let mut dirs: Vec<_> = std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    let mut cases = Vec::new();
    for case in dirs {
        let expected: Vec<Item> =
            serde_json::from_str(&std::fs::read_to_string(case.join("expected.json"))?)?;
        let input = std::fs::read_dir(&case)?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .find(|p| p.file_name().is_some_and(|n| n != "expected.json"))
            .ok_or_else(|| anyhow::anyhow!("{}: no input file", case.display()))?;
        cases.push(Case {
            name: file_name(&case),
            filename: file_name(&input),
            text: std::fs::read_to_string(&input)?,
            expected,
        });
    }
    Ok(cases)
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(text: &str, kind: Kind) -> Item {
        Item {
            text: text.into(),
            kind,
        }
    }

    #[test]
    fn scores_each_expected_item() {
        let expected = [
            item("山田", Kind::Person),
            item("アクメ", Kind::Company),
            item("db01", Kind::Host),
            item("PRJ-1", Kind::InternalId),
            item("小林", Kind::Person),
        ];
        let got = [
            item("山田", Kind::Person),
            item("株式会社アクメ", Kind::Company),
            item("db01", Kind::Project),
            item("小林 直樹", Kind::Person),
        ];
        let text = "山田 株式会社アクメ db01 PRJ-1 小林 直樹 / 小林:";
        assert_eq!(
            score(text, &expected, &got),
            [
                Outcome::Exact,
                Outcome::Covered("株式会社アクメ".into()),
                Outcome::WrongType(Kind::Project),
                Outcome::Missed,
                Outcome::Missed,
            ],
            "一致・包含・区分違い・取りこぼし。単独の出現が残るなら包含でも取りこぼし"
        );
    }

    #[test]
    fn tallies_recall_per_kind() {
        let expected = [
            item("a", Kind::Person),
            item("b", Kind::Person),
            item("c", Kind::Host),
        ];
        let outcomes = [
            Outcome::Exact,
            Outcome::Missed,
            Outcome::WrongType(Kind::Project),
        ];
        let tally = recall_by_kind(&expected, &outcomes);
        assert_eq!(tally["person"], (1, 1, 2), "person は 2 件中 1 件");
        assert_eq!(tally["host"], (1, 0, 1), "区分違いは伏せられたが区分は外れ");
    }
}
