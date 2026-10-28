//! OpenAI 形式と Anthropic 形式の API を、上流の `llama-server` へそのまま中継する。
//! kotonoha の鍵は上流へ渡さない。stream の答えは届いた片から返す。

use std::time::Duration;

use axum::body::{self, Body};
use axum::extract::Request;
use axum::http::{HeaderName, header};
use axum::response::Response;

use crate::app::BODY_LIMIT;
use crate::error::AppError;
use crate::llm::LlmError;

/// 要求から上流へ渡す header。鍵の header は含めない。
const REQUEST_HEADERS: [HeaderName; 2] = [header::CONTENT_TYPE, header::ACCEPT];
/// 上流の答えから返す header。長さは stream で変わるので含めない。
const RESPONSE_HEADERS: [HeaderName; 2] = [header::CONTENT_TYPE, header::CACHE_CONTROL];

pub struct Relay {
    base: String,
    http: reqwest::Client,
}

impl Relay {
    /// # Errors
    /// HTTP client を作れないとき。
    pub fn new(base: &str) -> anyhow::Result<Self> {
        // 全体の時間は区切らない。stream は長く続き、thinking の間は片が途切れる。
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_mins(15))
            .build()?;
        Ok(Self {
            base: base.trim_end_matches('/').to_owned(),
            http,
        })
    }

    /// 同じ method・path・query で上流へ送り、status・本文をそのまま返す。
    ///
    /// # Errors
    /// 本文が大きすぎるとき、または上流に届かないとき。
    pub async fn forward(&self, req: Request) -> Result<Response, AppError> {
        let (parts, body) = req.into_parts();
        let path = parts
            .uri
            .path_and_query()
            .map_or_else(|| parts.uri.path(), |p| p.as_str());
        let url = format!("{}{path}", self.base);
        let body = body::to_bytes(body, BODY_LIMIT)
            .await
            .map_err(|e| AppError::BadRequest(format!("read request body: {e}")))?;
        let mut upstream = self.http.request(parts.method, &url).body(body);
        for name in REQUEST_HEADERS {
            if let Some(value) = parts.headers.get(&name) {
                upstream = upstream.header(name, value);
            }
        }
        let res = upstream
            .send()
            .await
            .map_err(|e| LlmError::Upstream(format!("{url}: {e}")))?;

        let mut out = Response::builder().status(res.status());
        for name in RESPONSE_HEADERS {
            if let Some(value) = res.headers().get(&name) {
                out = out.header(name, value);
            }
        }
        out.body(Body::from_stream(res.bytes_stream()))
            .map_err(|e| AppError::Internal(format!("build response: {e}")))
    }
}
