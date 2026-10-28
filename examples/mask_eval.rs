//! 実機の llama-server を相手に評価の文で `/mask` の区分ごとの再現率を出す。
//!
//! `LLAMA_SERVER_URL=http://127.0.0.1:5012 cargo run --release --example mask_eval`

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Instant;

use kotonoha_server::eval::{self, Outcome};
use kotonoha_server::llm::LlamaServer;
use kotonoha_server::mask;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let url = std::env::var("LLAMA_SERVER_URL").unwrap_or_else(|_| "http://127.0.0.1:5012".into());
    let llm = LlamaServer::new(&url)?;
    let cases = eval::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("eval/mask"))?;
    let mut total: BTreeMap<&str, (usize, usize, usize)> = BTreeMap::new();
    for case in &cases {
        let started = Instant::now();
        let got = mask::mask(&llm, &case.text, Some(&case.filename)).await?;
        let outcomes = eval::score(&case.text, &case.expected, &got);
        println!("## {} ({:.1}s)", case.name, started.elapsed().as_secs_f64());
        for (want, outcome) in case.expected.iter().zip(&outcomes) {
            match outcome {
                Outcome::Exact => {}
                Outcome::WrongType(k) => println!(
                    "- 区分違い: {:?} {} → {}",
                    want.text,
                    want.kind.name(),
                    k.name()
                ),
                Outcome::Covered(by) => println!("- 包含: {:?} は {by:?} に含まれた", want.text),
                Outcome::Missed => println!("- 取りこぼし: {:?} ({})", want.text, want.kind.name()),
            }
        }
        let extra: Vec<_> = got
            .iter()
            .filter(|g| {
                !case
                    .expected
                    .iter()
                    .any(|w| w.text.contains(&g.text) || g.text.contains(&w.text))
            })
            .map(|g| format!("{:?}({})", g.text, g.kind.name()))
            .collect();
        if !extra.is_empty() {
            println!("- 正解に無い検出: {}", extra.join(", "));
        }
        for (kind, (masked, exact, all)) in eval::recall_by_kind(&case.expected, &outcomes) {
            let t = total.entry(kind).or_default();
            t.0 += masked;
            t.1 += exact;
            t.2 += all;
        }
    }
    println!("\n| 区分 | 伏せた | 区分も一致 | 正解 | 再現率 |\n|---|---|---|---|---|");
    for (kind, (masked, exact, all)) in total {
        #[allow(clippy::cast_precision_loss)]
        let rate = masked as f64 / all as f64 * 100.0;
        println!("| {kind} | {masked} | {exact} | {all} | {rate:.0}% |");
    }
    Ok(())
}
