//! 評価の文の正解のうち、正規表現の区分は Gemma なしで全件を拾う。

use std::path::Path;

use kotonoha_server::eval::{self, Outcome};
use kotonoha_server::mask::{find_by_rules, resolve};

#[test]
fn rules_catch_every_typed_secret_in_eval_set() {
    let cases = eval::load(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("eval/mask")
            .as_path(),
    )
    .expect("eval set loads");
    assert!(cases.len() >= 5, "評価の文が揃っている");
    let mut checked = 0;
    for case in &cases {
        for want in &case.expected {
            assert!(
                case.text.contains(&want.text),
                "{}: 正解 {:?} が入力に在る",
                case.name,
                want.text
            );
        }
        let typed: Vec<_> = case
            .expected
            .iter()
            .filter(|i| i.kind.by_rules())
            .cloned()
            .collect();
        let got = resolve(&case.text, find_by_rules(&case.text));
        for (want, outcome) in typed.iter().zip(eval::score(&case.text, &typed, &got)) {
            assert_eq!(
                outcome,
                Outcome::Exact,
                "{}: {:?} を {:?} で拾う",
                case.name,
                want.text,
                want.kind
            );
            checked += 1;
        }
    }
    assert!(checked >= 20, "正規表現の区分の正解が十分ある: {checked}");
}
