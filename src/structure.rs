//! `/structure`: 文章・画像・PDF を、渡された JSON Schema どおりの JSON にする。

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::AppError;
use crate::llm::{Chat, Llm, LlmError};

/// PDF から画像にする頁の上限。画像 1 枚が数百 token を使うので 32K の文脈に収める。
pub const MAX_PAGES: usize = 20;

const IMAGE_TYPES: [&str; 4] = ["image/png", "image/jpeg", "image/webp", "image/gif"];

const SYSTEM: &str = "あなたは書類や文章を読み取り、指定された JSON Schema どおりの JSON を作る係です。
画像に写った文字・数値・日付は見えるとおりに読み取ります。読み取れない値や書かれていない値は推測で埋めず、Schema が許すなら null や空にします。
Schema の description に書かれた意味と書式（日付の形など）に従います。";

#[derive(Deserialize)]
pub struct Request {
    pub schema: Value,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub files: Vec<File>,
    /// 読み取りの補足の指示。
    #[serde(default)]
    pub prompt: Option<String>,
}

/// shoebox が Gemini へ送る `inline_data` と同じ形。
#[derive(Deserialize)]
pub struct File {
    pub mime_type: String,
    /// base64。
    pub data: String,
}

/// Gemma へ渡す画像 (mime type と base64)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub mime_type: String,
    pub data: String,
}

/// 問い合わせの本文。指示・文章・画像を一つの user message に並べる。
#[must_use]
pub fn messages(prompt: Option<&str>, text: Option<&str>, images: &[Image]) -> Vec<Value> {
    let mut parts: Vec<Value> = [prompt, text]
        .into_iter()
        .flatten()
        .filter(|t| !t.is_empty())
        .map(|t| json!({ "type": "text", "text": t }))
        .collect();
    parts.extend(images.iter().map(|i| {
        json!({
            "type": "image_url",
            "image_url": { "url": format!("data:{};base64,{}", i.mime_type, i.data) }
        })
    }));
    vec![
        json!({ "role": "system", "content": SYSTEM }),
        json!({ "role": "user", "content": parts }),
    ]
}

/// 添付を画像の列にする。PDF は頁ごとの PNG にする。
async fn images(files: Vec<File>) -> Result<Vec<Image>, AppError> {
    let mut out = Vec::new();
    for file in files {
        let bytes = BASE64.decode(file.data.trim()).map_err(|e| {
            AppError::BadRequest(format!("{}: invalid base64: {e}", file.mime_type))
        })?;
        if file.mime_type == "application/pdf" {
            out.extend(pdf_pages(&bytes).await?.into_iter().map(|png| Image {
                mime_type: "image/png".into(),
                data: BASE64.encode(png),
            }));
        } else if IMAGE_TYPES.contains(&file.mime_type.as_str()) {
            out.push(Image {
                mime_type: file.mime_type,
                data: BASE64.encode(bytes),
            });
        } else {
            return Err(AppError::BadRequest(format!(
                "unsupported mime_type {}; use application/pdf or {}",
                file.mime_type,
                IMAGE_TYPES.join(", ")
            )));
        }
    }
    Ok(out)
}

/// poppler の `pdftoppm` で頁ごとに 150 dpi の PNG にする。
async fn pdf_pages(pdf: &[u8]) -> Result<Vec<Vec<u8>>, AppError> {
    let internal = |e: std::io::Error| AppError::Internal(format!("pdf: {e}"));
    let dir = tempfile::tempdir().map_err(internal)?;
    let input = dir.path().join("in.pdf");
    tokio::fs::write(&input, pdf).await.map_err(internal)?;
    let out = tokio::process::Command::new("pdftoppm")
        .args(["-png", "-r", "150", "-l", &(MAX_PAGES + 1).to_string()])
        .arg(&input)
        .arg(dir.path().join("page"))
        .output()
        .await
        .map_err(|e| AppError::Internal(format!("pdftoppm: {e}")))?;
    if !out.status.success() {
        let reason = String::from_utf8_lossy(&out.stderr);
        return Err(AppError::BadRequest(format!("pdf: {}", reason.trim())));
    }
    let mut pages: Vec<_> = std::fs::read_dir(dir.path())
        .map_err(internal)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "png"))
        .collect();
    if pages.len() > MAX_PAGES {
        return Err(AppError::BadRequest(format!(
            "pdf has more than {MAX_PAGES} pages"
        )));
    }
    pages.sort();
    pages
        .iter()
        .map(|p| std::fs::read(p).map_err(internal))
        .collect()
}

/// # Errors
/// 要求の形が誤っているとき、PDF を画像にできないとき、Gemma の答えが JSON でないとき。
pub async fn structure<L: Llm>(llm: &L, req: Request) -> Result<Value, AppError> {
    if !req.schema.is_object() {
        return Err(AppError::BadRequest(
            "schema must be a JSON Schema object".into(),
        ));
    }
    if req.text.as_deref().is_none_or(str::is_empty) && req.files.is_empty() {
        return Err(AppError::BadRequest("give text or files".into()));
    }
    let images = images(req.files).await?;
    let chat = Chat {
        messages: messages(req.prompt.as_deref(), req.text.as_deref(), &images),
        schema: req.schema,
        thinking: false,
        max_tokens: 8192,
    };
    let content = llm.complete(chat).await?;
    serde_json::from_str(&content)
        .map_err(|e| AppError::Llm(LlmError::Malformed(format!("{e}: {content}"))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::tests::Stub;

    fn request(value: Value) -> Request {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn messages_put_prompt_text_and_images_in_order() {
        let png = Image {
            mime_type: "image/png".into(),
            data: "AAAA".into(),
        };
        let m = messages(Some("金額だけ"), Some("請求書"), &[png]);
        assert_eq!(m[0]["role"], "system", "最初に役割");
        let parts = m[1]["content"].as_array().unwrap();
        assert_eq!(
            parts[0],
            json!({"type": "text", "text": "金額だけ"}),
            "補足の指示"
        );
        assert_eq!(parts[1], json!({"type": "text", "text": "請求書"}), "文章");
        assert_eq!(
            parts[2],
            json!({"type": "image_url", "image_url": {"url": "data:image/png;base64,AAAA"}}),
            "画像は data URL"
        );
    }

    #[tokio::test]
    async fn returns_answer_as_json_and_forwards_schema() {
        let llm = Stub::new(vec![r#"{"total":1200}"#]);
        let schema = json!({"type": "object", "properties": {"total": {"type": "integer"}}});
        let png = BASE64.encode(b"\x89PNG");
        let req = request(json!({
            "schema": schema,
            "files": [{"mime_type": "image/png", "data": png}]
        }));
        let out = structure(&llm, req).await.unwrap();
        assert_eq!(out, json!({"total": 1200}), "答えを JSON で返す");
        let sent = llm.sent();
        assert_eq!(sent[0].schema, schema, "渡された schema で形を強制する");
        let parts = sent[0].messages[1]["content"].as_array().unwrap();
        assert_eq!(
            parts[0]["image_url"]["url"],
            format!("data:image/png;base64,{png}"),
            "画像を Gemma に渡す"
        );
    }

    #[tokio::test]
    async fn rejects_bad_requests() {
        let llm = Stub::new(vec![]);
        let cases = [
            json!({"schema": "x", "text": "a"}),
            json!({"schema": {}, "text": ""}),
            json!({"schema": {}, "files": [{"mime_type": "text/html", "data": "AAAA"}]}),
            json!({"schema": {}, "files": [{"mime_type": "image/png", "data": "!!"}]}),
        ];
        for case in cases {
            let err = structure(&llm, request(case.clone())).await.unwrap_err();
            assert!(matches!(err, AppError::BadRequest(_)), "{case} は 400");
        }
        assert!(llm.sent().is_empty(), "誤った要求は Gemma に渡さない");
    }

    #[tokio::test]
    async fn rejects_non_json_answer() {
        let llm = Stub::new(vec!["sorry"]);
        let err = structure(&llm, request(json!({"schema": {}, "text": "a"})))
            .await
            .unwrap_err();
        assert!(
            matches!(err, AppError::Llm(LlmError::Malformed(_))),
            "JSON でない答えは誤り"
        );
    }
}
