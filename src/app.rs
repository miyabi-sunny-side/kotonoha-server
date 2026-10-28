//! HTTP の窓口。`/health` 以外は API の鍵が要る。
//! `/v1/` の窓口は OpenAI・Anthropic 形式の要求を `llama-server` へ中継する。

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
use crate::relay::Relay;
use crate::{mask, structure, transcript};

/// 画像・PDF を base64 で受けるので大きめにする。
pub const BODY_LIMIT: usize = 64 * 1024 * 1024;

struct Shared<L> {
    llm: L,
    relay: Relay,
    keys: Vec<String>,
}

type AppState<L> = State<Arc<Shared<L>>>;

pub fn router<L: Llm>(llm: L, relay: Relay, keys: Vec<String>) -> Router {
    let shared = Arc::new(Shared { llm, relay, keys });
    let api = Router::new()
        .route("/mask", post(mask_handler::<L>))
        .route("/structure", post(structure_handler::<L>))
        .route("/transcript", post(transcript_handler::<L>))
        .route("/v1/chat/completions", post(relay_handler::<L>))
        .route("/v1/models", get(relay_handler::<L>))
        .route("/v1/messages", post(relay_handler::<L>))
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
    let headers = req.headers();
    let value = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    if auth::authorized(
        value(header::AUTHORIZATION.as_str()),
        value("x-api-key"),
        &s.keys,
    ) {
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

async fn relay_handler<L: Llm>(State(s): AppState<L>, req: Request) -> Result<Response, AppError> {
    s.relay.forward(req).await
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

    /// 上流の `llama-server` が居ない構成。中継は 502 になる。
    fn app(answers: Vec<&str>) -> Router {
        app_with_upstream(answers, "http://127.0.0.1:9")
    }

    fn app_with_upstream(answers: Vec<&str>, upstream: &str) -> Router {
        router(
            Stub::new(answers),
            Relay::new(upstream).unwrap(),
            vec!["good-key".into()],
        )
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

    /// 上流の `llama-server` の代わり。受けた要求を写して返す。
    /// `stream: true` の chat は 1 片目を送った後、`release` が通知されるまで 2 片目を止める。
    async fn upstream(release: Arc<tokio::sync::Notify>) -> String {
        use axum::extract::RawQuery;
        use axum::http::HeaderMap;

        fn echo(headers: &HeaderMap, query: Option<&str>, body: &Value) -> Value {
            let get = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
            json!({
                "body": body,
                "query": query,
                "authorization": get("authorization"),
                "x_api_key": get("x-api-key"),
                "content_type": get("content-type"),
            })
        }

        let chat = move |headers: HeaderMap, Json(body): Json<Value>| {
            let release = release.clone();
            async move {
                if body["stream"] != true {
                    return Json(echo(&headers, None, &body)).into_response();
                }
                let chunks = futures_util::stream::unfold(0, move |n| {
                    let release = release.clone();
                    async move {
                        let chunk = match n {
                            0 => "data: {\"n\":1}\n\n",
                            1 => {
                                release.notified().await;
                                "data: [DONE]\n\n"
                            }
                            _ => return None,
                        };
                        Some((Ok::<_, std::io::Error>(chunk), n + 1))
                    }
                });
                (
                    [(header::CONTENT_TYPE, "text/event-stream")],
                    Body::from_stream(chunks),
                )
                    .into_response()
            }
        };
        let models = |headers: HeaderMap, RawQuery(query): RawQuery| async move {
            Json(echo(
                &headers,
                query.as_deref(),
                &json!({"data": [{"id": "gemma"}]}),
            ))
        };
        let messages = |headers: HeaderMap, Json(body): Json<Value>| async move {
            if body["bad"] == true {
                (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"type": "error", "error": {"message": "bad"}})),
                )
                    .into_response()
            } else {
                Json(echo(&headers, None, &body)).into_response()
            }
        };
        let stub = Router::new()
            .route("/v1/chat/completions", post(chat))
            .route("/v1/models", get(models))
            .route("/v1/messages", post(messages));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, stub).await.unwrap() });
        format!("http://{addr}/")
    }

    async fn relay_app() -> (Router, Arc<tokio::sync::Notify>) {
        let release = Arc::new(tokio::sync::Notify::new());
        let base = upstream(release.clone()).await;
        (app_with_upstream(vec![], &base), release)
    }

    async fn send(app: Router, req: Request<Body>) -> (StatusCode, Value) {
        let res = app.oneshot(req).await.unwrap();
        let status = res.status();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    #[tokio::test]
    async fn relay_needs_the_key() {
        let (app, _) = relay_app().await;
        for (method, path) in [
            ("POST", "/v1/chat/completions"),
            ("GET", "/v1/models"),
            ("POST", "/v1/messages"),
        ] {
            let req = || Request::builder().method(method).uri(path);
            let (status, _) = send(app.clone(), req().body(Body::empty()).unwrap()).await;
            assert_eq!(
                status,
                StatusCode::UNAUTHORIZED,
                "{path} は鍵が無ければ 401"
            );
            let (status, _) = send(
                app.clone(),
                req()
                    .header(header::AUTHORIZATION, "Bearer bad-key")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{path} は違う鍵なら 401");
            let (status, _) = send(
                app.clone(),
                req()
                    .header("x-api-key", "bad-key")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
            assert_eq!(
                status,
                StatusCode::UNAUTHORIZED,
                "{path} は違う x-api-key なら 401"
            );
        }
    }

    #[tokio::test]
    async fn openai_chat_is_relayed_without_our_key() {
        let (app, _) = relay_app().await;
        let body =
            json!({"model": "gemma", "messages": [{"role": "user", "content": "こんにちは"}]});
        let (status, got) = call(app, "/v1/chat/completions", Some("good-key"), body.clone()).await;
        assert_eq!(status, StatusCode::OK, "Bearer の鍵で通る");
        assert_eq!(got["body"], body, "本文をそのまま渡す");
        assert_eq!(
            got["content_type"], "application/json",
            "Content-Type を渡す"
        );
        assert_eq!(
            got["authorization"],
            Value::Null,
            "kotonoha の鍵は上流へ渡さない"
        );
    }

    #[tokio::test]
    async fn anthropic_messages_accept_x_api_key() {
        let (app, _) = relay_app().await;
        let body = json!({"model": "gemma", "max_tokens": 16, "messages": [{"role": "user", "content": "こんにちは"}]});
        let req = Request::post("/v1/messages")
            .header(header::CONTENT_TYPE, "application/json")
            .header("x-api-key", "good-key")
            .header("anthropic-version", "2023-06-01")
            .body(Body::from(body.to_string()))
            .unwrap();
        let (status, got) = send(app, req).await;
        assert_eq!(status, StatusCode::OK, "x-api-key の鍵で通る");
        assert_eq!(got["body"], body, "本文をそのまま渡す");
        assert_eq!(
            got["x_api_key"],
            Value::Null,
            "kotonoha の鍵は上流へ渡さない"
        );
    }

    #[tokio::test]
    async fn models_keep_the_query() {
        let (app, _) = relay_app().await;
        let req = Request::get("/v1/models?limit=5")
            .header(header::AUTHORIZATION, "Bearer good-key")
            .body(Body::empty())
            .unwrap();
        let (status, got) = send(app, req).await;
        assert_eq!(status, StatusCode::OK, "GET も中継する");
        assert_eq!(got["body"]["data"][0]["id"], "gemma", "上流の答えを返す");
        assert_eq!(got["query"], "limit=5", "query を渡す");
    }

    #[tokio::test]
    async fn upstream_error_status_is_kept() {
        let (app, _) = relay_app().await;
        let (status, got) = call(app, "/v1/messages", Some("good-key"), json!({"bad": true})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "上流の status を保つ");
        assert_eq!(got["error"]["message"], "bad", "上流の誤りの本文を保つ");
    }

    #[tokio::test]
    async fn stream_is_relayed_chunk_by_chunk() {
        let (app, release) = relay_app().await;
        let req = Request::post("/v1/chat/completions")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::AUTHORIZATION, "Bearer good-key")
            .body(Body::from(json!({"stream": true}).to_string()))
            .unwrap();
        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK, "stream も通る");
        assert_eq!(
            res.headers()[header::CONTENT_TYPE],
            "text/event-stream",
            "SSE の種類を保つ"
        );
        let mut body = res.into_body();
        let first = tokio::time::timeout(std::time::Duration::from_secs(5), body.frame())
            .await
            .expect("上流が終わる前に 1 片目が届く")
            .unwrap()
            .unwrap()
            .into_data()
            .unwrap();
        assert_eq!(&first[..], b"data: {\"n\":1}\n\n", "1 片目をそのまま返す");
        release.notify_one();
        let rest = body.collect().await.unwrap().to_bytes();
        assert_eq!(&rest[..], b"data: [DONE]\n\n", "残りも返す");
    }

    #[tokio::test]
    async fn unreachable_upstream_is_bad_gateway() {
        let (status, body) = call(
            app(vec![]),
            "/v1/chat/completions",
            Some("good-key"),
            json!({"messages": []}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_GATEWAY, "上流に届かなければ 502");
        assert!(body["error"].is_string(), "理由を返す");
    }
}
