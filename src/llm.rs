//! 上流の `llama-server` への呼び出し (`/v1/chat/completions` の互換 API)。

use std::future::Future;
use std::time::Duration;

use serde_json::{Value, json};

/// 1 回の問い合わせ。答えは `schema` に従う JSON の文字列で返る。
#[derive(Clone, Debug)]
pub struct Chat {
    pub messages: Vec<Value>,
    pub schema: Value,
    pub thinking: bool,
    pub max_tokens: u32,
}

#[derive(Debug)]
pub enum LlmError {
    /// 上流に届かない、または上流が誤りを返した。
    Upstream(String),
    /// 答えが途中で切れた、または期待した形でない。
    Malformed(String),
}

impl std::fmt::Display for LlmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LlmError::Upstream(m) => write!(f, "upstream: {m}"),
            LlmError::Malformed(m) => write!(f, "malformed answer: {m}"),
        }
    }
}

impl std::error::Error for LlmError {}

pub trait Llm: Send + Sync + 'static {
    fn complete(&self, chat: Chat) -> impl Future<Output = Result<String, LlmError>> + Send;
}

pub struct LlamaServer {
    base: String,
    http: reqwest::Client,
}

impl LlamaServer {
    /// # Errors
    /// HTTP client を作れないとき。
    pub fn new(base: &str) -> anyhow::Result<Self> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_mins(15))
            .build()?;
        Ok(Self {
            base: base.trim_end_matches('/').to_owned(),
            http,
        })
    }
}

impl Llm for LlamaServer {
    async fn complete(&self, chat: Chat) -> Result<String, LlmError> {
        let url = format!("{}/v1/chat/completions", self.base);
        let res = self
            .http
            .post(&url)
            .json(&request_body(&chat))
            .send()
            .await
            .map_err(|e| LlmError::Upstream(format!("{url}: {e}")))?;
        let status = res.status();
        let body: Value = res
            .json()
            .await
            .map_err(|e| LlmError::Upstream(format!("{url}: {status}: {e}")))?;
        if !status.is_success() {
            return Err(LlmError::Upstream(format!("{url}: {status}: {body}")));
        }
        answer(&body)
    }
}

#[must_use]
pub fn request_body(chat: &Chat) -> Value {
    json!({
        "messages": chat.messages,
        "response_format": {
            "type": "json_schema",
            "json_schema": { "name": "output", "strict": true, "schema": chat.schema }
        },
        "chat_template_kwargs": { "enable_thinking": chat.thinking },
        "temperature": 0,
        "max_tokens": chat.max_tokens,
        "stream": false
    })
}

/// 上流の応答から答えの本文を取り出す。途中で切れた答えは誤りにする。
///
/// # Errors
/// 本文が無い、または `max_tokens` で切れたとき。
pub fn answer(body: &Value) -> Result<String, LlmError> {
    let choice = &body["choices"][0];
    if choice["finish_reason"] == "length" {
        return Err(LlmError::Malformed("answer hit max_tokens".into()));
    }
    choice["message"]["content"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| LlmError::Malformed(format!("no content: {body}")))
}

#[cfg(test)]
pub mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use super::*;

    /// 決まった答えを順に返し、受けた問い合わせを記録する。
    pub struct Stub {
        answers: Mutex<VecDeque<String>>,
        sent: Mutex<Vec<Chat>>,
    }

    impl Stub {
        pub fn new(answers: Vec<&str>) -> Self {
            Self {
                answers: Mutex::new(answers.into_iter().map(str::to_owned).collect()),
                sent: Mutex::new(Vec::new()),
            }
        }

        /// # Panics
        /// 別の test が lock を握ったまま落ちたとき。
        pub fn sent(&self) -> Vec<Chat> {
            self.sent.lock().unwrap().clone()
        }
    }

    impl Llm for Stub {
        async fn complete(&self, chat: Chat) -> Result<String, LlmError> {
            self.sent.lock().unwrap().push(chat);
            self.answers
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| LlmError::Upstream("stub has no more answers".into()))
        }
    }

    fn chat(thinking: bool) -> Chat {
        Chat {
            messages: vec![json!({"role": "user", "content": "hi"})],
            schema: json!({"type": "object"}),
            thinking,
            max_tokens: 10,
        }
    }

    #[test]
    fn request_forces_schema_and_thinking_switch() {
        let body = request_body(&chat(false));
        assert_eq!(
            body["response_format"]["json_schema"]["schema"],
            json!({"type": "object"}),
            "schema で形を強制する"
        );
        assert_eq!(
            body["chat_template_kwargs"]["enable_thinking"], false,
            "thinking を窓口ごとに切り替える"
        );
        assert_eq!(
            request_body(&chat(true))["chat_template_kwargs"]["enable_thinking"],
            true,
            "thinking を有効にできる"
        );
    }

    #[test]
    fn answer_takes_content() {
        let body =
            json!({"choices": [{"finish_reason": "stop", "message": {"content": "{\"a\":1}"}}]});
        assert_eq!(answer(&body).unwrap(), "{\"a\":1}", "本文を取り出す");
    }

    #[test]
    fn answer_rejects_truncated() {
        let body =
            json!({"choices": [{"finish_reason": "length", "message": {"content": "{\"a\""}}]});
        assert!(
            matches!(answer(&body), Err(LlmError::Malformed(_))),
            "切れた答えは誤り"
        );
        assert!(
            matches!(answer(&json!({})), Err(LlmError::Malformed(_))),
            "本文が無ければ誤り"
        );
    }
}
