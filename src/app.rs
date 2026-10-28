//! HTTP の窓口。`/health` 以外は API の鍵が要る。

use std::sync::Arc;

use axum::extract::{DefaultBodyLimit, Request, State};
use axum::http::{StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::auth;
use crate::error::AppError;
use crate::llm::Llm;
use crate::{mask, structure, transcript};

/// 画像・PDF を base64 で受けるので大きめにする。
const BODY_LIMIT: usize = 64 * 1024 * 1024;

struct Shared<L> {
    llm: L,
    keys: Vec<String>,
}

type AppState<L> = State<Arc<Shared<L>>>;

pub fn router<L: Llm>(llm: L, keys: Vec<String>) -> Router {
    let shared = Arc::new(Shared { llm, keys });
    let api = Router::new()
        .route("/mask", post(mask_handler::<L>))
        .route("/structure", post(structure_handler::<L>))
        .route("/transcript", post(transcript_handler::<L>))
        .route_layer(middleware::from_fn_with_state(
            shared.clone(),
            require_key::<L>,
        ));
    Router::new()
        .route("/health", get(health))
        .merge(api)
        .layer(DefaultBodyLimit::max(BODY_LIMIT))
        .with_state(shared)
}

async fn require_key<L: Llm>(State(s): AppState<L>, req: Request, next: Next) -> Response {
    let header = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());
    if auth::authorized(header, &s.keys) {
        next.run(req).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "invalid or missing API key" })),
        )
            .into_response()
    }
}

async fn health() -> Json<Value> {
    Json(json!({ "status": "ok", "version": env!("CARGO_PKG_VERSION") }))
}

#[derive(Deserialize)]
struct MaskRequest {
    text: String,
    #[serde(default)]
    filename: Option<String>,
}

async fn mask_handler<L: Llm>(
    State(s): AppState<L>,
    Json(req): Json<MaskRequest>,
) -> Result<Json<Vec<mask::Item>>, AppError> {
    Ok(Json(
        mask::mask(&s.llm, &req.text, req.filename.as_deref()).await?,
    ))
}

async fn structure_handler<L: Llm>(
    State(s): AppState<L>,
    Json(req): Json<structure::Request>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(structure::structure(&s.llm, req).await?))
}

#[derive(Deserialize)]
struct TranscriptRequest {
    text: String,
}

async fn transcript_handler<L: Llm>(
    State(s): AppState<L>,
    Json(req): Json<TranscriptRequest>,
) -> Result<Json<transcript::Transcript>, AppError> {
    Ok(Json(transcript::transcript(&s.llm, &req.text).await?))
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use super::*;
    use crate::llm::tests::Stub;

    fn app(answers: Vec<&str>) -> Router {
        router(Stub::new(answers), vec!["good-key".into()])
    }

    async fn call(app: Router, path: &str, key: Option<&str>, body: Value) -> (StatusCode, Value) {
        let mut req = Request::post(path).header(header::CONTENT_TYPE, "application/json");
        if let Some(key) = key {
            req = req.header(header::AUTHORIZATION, format!("Bearer {key}"));
        }
        let res = app
            .oneshot(req.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    #[tokio::test]
    async fn every_endpoint_needs_the_key() {
        for path in ["/mask", "/structure", "/transcript"] {
            let (status, _) = call(app(vec![]), path, None, json!({"text": "a"})).await;
            assert_eq!(
                status,
                StatusCode::UNAUTHORIZED,
                "{path} は鍵が無ければ 401"
            );
            let (status, _) = call(app(vec![]), path, Some("bad-key"), json!({"text": "a"})).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{path} は違う鍵なら 401");
        }
    }

    #[tokio::test]
    async fn health_needs_no_key() {
        let res = app(vec![])
            .oneshot(Request::get("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::OK, "health は鍵なしで答える");
    }

    #[tokio::test]
    async fn mask_returns_bare_array() {
        let (status, body) = call(
            app(vec![r#"{"items":[]}"#]),
            "/mask",
            Some("good-key"),
            json!({"text": "mail: a@acme.co.jp", "filename": "x.txt"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "鍵があれば通る");
        assert_eq!(
            body,
            json!([{"text": "a@acme.co.jp", "type": "mail"}]),
            "区分つきの配列"
        );
    }

    #[tokio::test]
    async fn transcript_returns_text_segments_points() {
        let answer = r#"{"segments":[{"speaker":"不明","text":"こんにちは。"}],"points":["挨拶"]}"#;
        let (status, body) = call(
            app(vec![answer]),
            "/transcript",
            Some("good-key"),
            json!({"text": "えー こんにちは"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "鍵があれば通る");
        assert_eq!(
            body,
            json!({"text": "こんにちは。", "segments": [{"speaker": "不明", "text": "こんにちは。"}], "points": ["挨拶"]}),
            "本文と発言と要点"
        );
    }

    #[tokio::test]
    async fn upstream_failure_is_bad_gateway() {
        let (status, body) = call(
            app(vec![]),
            "/transcript",
            Some("good-key"),
            json!({"text": "a"}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_GATEWAY, "上流の失敗は 502");
        assert!(body["error"].is_string(), "理由を返す");
    }

    #[tokio::test]
    async fn bad_structure_request_is_400() {
        let (status, _) = call(
            app(vec![]),
            "/structure",
            Some("good-key"),
            json!({"schema": {}, "text": ""}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "誤った要求は 400");
    }
}
