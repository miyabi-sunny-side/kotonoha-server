//! `/mask`: 伏せるべき文字列を区分つきで探す。置き換えはしない (CLI の仕事)。

mod resolve;
mod rules;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::chunk;
use crate::llm::{Chat, Llm, LlmError};

pub use resolve::{locate, resolve};
pub use rules::find as find_by_rules;

/// 伏せる文字列の区分。前半は正規表現、後半は Gemma が拾う。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Mail,
    Ip,
    Ipv6,
    Url,
    Phone,
    Secret,
    Person,
    Company,
    Host,
    Project,
    InternalId,
}

impl Kind {
    pub const LLM: [Kind; 5] = [
        Kind::Person,
        Kind::Company,
        Kind::Host,
        Kind::Project,
        Kind::InternalId,
    ];

    #[must_use]
    pub fn by_rules(self) -> bool {
        !Self::LLM.contains(&self)
    }

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Kind::Mail => "mail",
            Kind::Ip => "ip",
            Kind::Ipv6 => "ipv6",
            Kind::Url => "url",
            Kind::Phone => "phone",
            Kind::Secret => "secret",
            Kind::Person => "person",
            Kind::Company => "company",
            Kind::Host => "host",
            Kind::Project => "project",
            Kind::InternalId => "internal_id",
        }
    }
}

/// 入力の `start..end` に現れる候補。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub start: usize,
    pub end: usize,
    pub kind: Kind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    pub text: String,
    #[serde(rename = "type")]
    pub kind: Kind,
}

/// 1 回の Gemma 呼び出しに渡す入力の上限 (推定 token)。出力と指示の分を 32K から残す。
pub const CHUNK_TOKENS: usize = 12_000;

const SYSTEM: &str = "あなたは機密情報の検出器です。外部へ渡す前のテキスト（ソースコードや文書）から、伏せるべき文字列を抜き出します。
対象は次の区分だけです。
- person: 人名（氏名の一部、ローマ字表記、コメントやメールの署名に書かれた名前を含む）
- company: 会社名・組織名・部署名（取引先や顧客を含む。OSS・言語・クラウド・一般的な製品の名前は除く）
- host: 社内のホスト名・サーバー名・内部のドメイン名
- project: 案件名・プロジェクト名・コードネーム・社内システムの名前
- internal_id: 社員番号・顧客番号・契約番号・チケット番号など、社内で割り振られた ID
text には入力にそのまま現れる文字列を一字一句変えずに書きます。翻訳・正規化・要約をしません。
メールアドレス・IP アドレス・URL・電話番号・パスワードや API キーは別の仕組みで拾うので、含めなくてかまいません。
プログラミング言語の予約語、ライブラリ名、一般的な変数名・関数名・型名は含めません。
同じ文字列は一度だけ書きます。該当が無ければ空の配列を返します。";

fn schema() -> Value {
    let kinds: Vec<&str> = Kind::LLM.iter().map(|k| k.name()).collect();
    json!({
        "type": "object",
        "properties": {
            "items": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "text": { "type": "string" },
                        "type": { "type": "string", "enum": kinds }
                    },
                    "required": ["text", "type"]
                }
            }
        },
        "required": ["items"]
    })
}

fn chat(part: &str, filename: Option<&str>) -> Chat {
    let user = match filename {
        Some(name) => format!("file: {name}\n\n{part}"),
        None => part.to_owned(),
    };
    Chat {
        messages: vec![
            json!({ "role": "system", "content": SYSTEM }),
            json!({ "role": "user", "content": user }),
        ],
        schema: schema(),
        thinking: false,
        max_tokens: 8192,
    }
}

#[derive(Deserialize)]
struct Answer {
    items: Vec<Item>,
}

/// 正規表現と Gemma の候補を合わせ、入力に現れない物を捨て、重なりは長い方を残す。
///
/// # Errors
/// Gemma の呼び出しに失敗したとき、または答えが schema に合わないとき。
pub async fn mask<L: Llm>(
    llm: &L,
    text: &str,
    filename: Option<&str>,
) -> Result<Vec<Item>, LlmError> {
    let mut candidates = find_by_rules(text);
    let mut proposed = Vec::new();
    for part in chunk::split(text, CHUNK_TOKENS) {
        let content = llm.complete(chat(part, filename)).await?;
        let answer: Answer = serde_json::from_str(&content)
            .map_err(|e| LlmError::Malformed(format!("{e}: {content}")))?;
        proposed.extend(answer.items.into_iter().filter(|i| !i.kind.by_rules()));
    }
    let parts = name_parts(&proposed);
    candidates.extend(locate(text, &proposed));
    candidates.extend(locate(text, &parts));
    Ok(resolve(text, candidates))
}

/// 空白で区切られた氏名の姓・名。単独で書かれた「小林」も伏せるために使う。
/// ASCII の 2 文字以下の部分は別の語に紛れやすいので使わない。
fn name_parts(items: &[Item]) -> Vec<Item> {
    items
        .iter()
        .filter(|i| i.kind == Kind::Person)
        .flat_map(|i| {
            let parts: Vec<&str> = i.text.split_whitespace().collect();
            let whole = parts.len() > 1;
            parts
                .into_iter()
                .filter(move |p| whole && (!p.is_ascii() || p.len() >= 3))
        })
        .map(|p| Item {
            text: p.to_owned(),
            kind: Kind::Person,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::tests::Stub;

    #[tokio::test]
    async fn merges_rules_and_llm_and_drops_unseen() {
        let llm = Stub::new(vec![
            r#"{"items":[{"text":"山田太郎","type":"person"},{"text":"存在しない","type":"company"}]}"#,
        ]);
        let text = "// 担当: 山田太郎 <taro@acme.co.jp>";
        let items = mask(&llm, text, Some("a.rs")).await.unwrap();
        assert_eq!(
            items,
            vec![
                Item {
                    text: "山田太郎".into(),
                    kind: Kind::Person
                },
                Item {
                    text: "taro@acme.co.jp".into(),
                    kind: Kind::Mail
                },
            ],
            "Gemma の候補のうち入力に無い物は捨てる"
        );
        let sent = llm.sent();
        assert!(
            sent[0].messages[1]["content"]
                .as_str()
                .unwrap()
                .starts_with("file: a.rs\n\n"),
            "file 名を Gemma に渡す"
        );
        assert!(!sent[0].thinking, "mask は thinking しない");
    }

    #[tokio::test]
    async fn splits_long_input_and_merges_answers() {
        let line = format!("{}\n", "あ".repeat(1000));
        let mut text = line.repeat(CHUNK_TOKENS / 1000 + 1);
        text.push_str("鈴木一郎\n");
        text.insert_str(0, "佐藤花子\n");
        let llm = Stub::new(vec![
            r#"{"items":[{"text":"佐藤花子","type":"person"}]}"#,
            r#"{"items":[{"text":"鈴木一郎","type":"person"}]}"#,
        ]);
        let items = mask(&llm, &text, None).await.unwrap();
        assert_eq!(llm.sent().len(), 2, "上限を超える入力は分けて渡す");
        let names: Vec<&str> = items.iter().map(|i| i.text.as_str()).collect();
        assert_eq!(names, ["佐藤花子", "鈴木一郎"], "分けた結果を合わせる");
    }

    #[tokio::test]
    async fn also_masks_parts_of_a_full_name() {
        let llm = Stub::new(vec![
            r#"{"items":[{"text":"小林 直樹","type":"person"},{"text":"Li Wei","type":"person"}]}"#,
        ]);
        let text = "前任: 小林 直樹 / 小林: 確認済み / 直樹 / Li Wei, Wei / Linux";
        let items = mask(&llm, text, None).await.unwrap();
        let names: Vec<&str> = items.iter().map(|i| i.text.as_str()).collect();
        assert_eq!(
            names,
            ["小林 直樹", "小林", "直樹", "Li Wei", "Wei"],
            "氏名の姓・名が単独で現れても伏せる。短すぎる ASCII の部分は使わない"
        );
    }

    #[tokio::test]
    async fn rejects_answer_outside_schema() {
        let llm = Stub::new(vec!["not json"]);
        let err = mask(&llm, "x", None).await.unwrap_err();
        assert!(
            matches!(err, LlmError::Malformed(_)),
            "壊れた答えは誤りにする"
        );
    }
}
