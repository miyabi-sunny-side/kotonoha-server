//! `/transcript`: 日本語の文字起こしを整え、話者ごとの発言と要点にする。

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::chunk;
use crate::llm::{Chat, Llm, LlmError};

/// 出力が入力と同じほど長くなるので、mask より小さく分ける。
pub const CHUNK_TOKENS: usize = 6_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Segment {
    pub speaker: String,
    pub text: String,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct Transcript {
    /// 整えた本文。
    pub text: String,
    pub segments: Vec<Segment>,
    pub points: Vec<String>,
}

#[derive(Deserialize)]
struct Answer {
    segments: Vec<Segment>,
    points: Vec<String>,
}

const SYSTEM: &str = "あなたは日本語の文字起こしを整える編集者です。
入力は音声認識の生の文です。次のとおり整えます。
- つなぎ言葉（えー、あの、えっと、まあ、なんか 等）と言い直しを除く。
- 誤変換を文脈から正しい語に直す。
- 句読点を補い、読める文にする。
- 意味・事実・数値・固有名詞は変えない。要約せず、話された内容は全て残す。
segments は発言の順に、話者が替わるごとに分けます。speaker には入力から分かる名前やラベルを書き、分からなければ「不明」とします。
points は話の要点を短い文で、重要な順に書きます。";

fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "segments": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "speaker": { "type": "string" },
                        "text": { "type": "string" }
                    },
                    "required": ["speaker", "text"]
                }
            },
            "points": { "type": "array", "items": { "type": "string" } }
        },
        "required": ["segments", "points"]
    })
}

/// 本文を発言から組む。話者が 2 人以上なら行頭に話者を付ける。
#[must_use]
pub fn compose(segments: &[Segment]) -> String {
    let labelled = segments.windows(2).any(|w| w[0].speaker != w[1].speaker);
    segments
        .iter()
        .map(|s| {
            if labelled {
                format!("{}: {}", s.speaker, s.text)
            } else {
                s.text.clone()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// # Errors
/// Gemma の呼び出しに失敗したとき、または答えが schema に合わないとき。
pub async fn transcript<L: Llm>(llm: &L, text: &str) -> Result<Transcript, LlmError> {
    let mut segments = Vec::new();
    let mut points = Vec::new();
    for part in chunk::split(text, CHUNK_TOKENS) {
        let chat = Chat {
            messages: vec![
                json!({ "role": "system", "content": SYSTEM }),
                json!({ "role": "user", "content": part }),
            ],
            schema: schema(),
            thinking: false,
            max_tokens: 16_384,
        };
        let content = llm.complete(chat).await?;
        let answer: Answer = serde_json::from_str(&content)
            .map_err(|e| LlmError::Malformed(format!("{e}: {content}")))?;
        segments.extend(answer.segments);
        points.extend(answer.points);
    }
    Ok(Transcript {
        text: compose(&segments),
        segments,
        points,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::tests::Stub;

    fn seg(speaker: &str, text: &str) -> Segment {
        Segment {
            speaker: speaker.into(),
            text: text.into(),
        }
    }

    #[test]
    fn compose_single_speaker_without_labels() {
        let text = compose(&[
            seg("不明", "今日は晴れです。"),
            seg("不明", "明日は雨です。"),
        ]);
        assert_eq!(
            text, "今日は晴れです。\n明日は雨です。",
            "話者が 1 人なら発言だけを並べる"
        );
    }

    #[test]
    fn compose_many_speakers_with_labels() {
        let text = compose(&[seg("田中", "始めます。"), seg("佐藤", "はい。")]);
        assert_eq!(
            text, "田中: 始めます。\n佐藤: はい。",
            "話者が複数なら行頭に話者"
        );
    }

    #[test]
    fn compose_nothing() {
        assert_eq!(compose(&[]), "", "発言が無ければ空");
    }

    #[tokio::test]
    async fn merges_chunks() {
        let llm = Stub::new(vec![
            r#"{"segments":[{"speaker":"A","text":"一。"}],"points":["p1"]}"#,
            r#"{"segments":[{"speaker":"B","text":"二。"}],"points":["p2"]}"#,
        ]);
        let raw = format!("{}\n{}\n", "え".repeat(CHUNK_TOKENS - 1), "あ".repeat(10));
        let t = transcript(&llm, &raw).await.unwrap();
        assert_eq!(llm.sent().len(), 2, "長い文字起こしは分けて渡す");
        assert_eq!(
            t.segments,
            vec![seg("A", "一。"), seg("B", "二。")],
            "発言を順に合わせる"
        );
        assert_eq!(t.points, ["p1", "p2"], "要点を合わせる");
        assert_eq!(t.text, "A: 一。\nB: 二。", "本文は発言から組む");
    }
}
