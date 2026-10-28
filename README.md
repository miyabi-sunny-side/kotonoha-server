# kotonoha-server

手元の llama-server で動く Gemma に日本語の処理を任せる、JSON の Web API です。
相談相手の chat ではなく、プログラムから呼ぶ部品として使います。

- `POST /mask`: ソースコードや文書から、伏せるべき文字列と区分を返す
- `POST /structure`: 文章・画像・PDF を、渡した JSON Schema どおりの JSON にする
- `POST /transcript`: 文字起こしの生の文を整え、話者ごとの発言と要点にする

## 必要な環境

- Linux x86_64 のマシン
- OpenAI 互換の `/v1/chat/completions` と JSON Schema の構造化出力を持つ
  [llama-server](https://github.com/ggml-org/llama.cpp)。Gemma 4 のような画像を読めるモデルと
  `--mmproj` の部品を載せ、`--jinja` で起動したもの。上流は HTTP だけに対応します。
- PDF を読むなら poppler の `pdftoppm` を `PATH` に置く

文脈は 32K token を前提にしています。長い入力は分けて渡します（[長い入力](#長い入力)）。
llama-server の起動例です。model と mmproj の file 名は手元の物に置き換えます。

```sh
llama-server --model gemma-4-26B_q4_0-it.gguf --mmproj gemma-4-26B-it-mmproj.gguf \
  --host 127.0.0.1 --port 5012 --ctx-size 32768 --jinja
```

## 起動する

[Releases](https://github.com/miyabi-sunny-side/kotonoha-server/releases/latest) から
`kotonoha-server-linux-x86_64.tar.gz` と同名の `.sha256` を同じディレクトリへダウンロードします。

```sh
sha256sum -c kotonoha-server-linux-x86_64.tar.gz.sha256
tar -xzf kotonoha-server-linux-x86_64.tar.gz
./kotonoha-server --version
```

source から作るなら、Rust 1.96 以降で `cargo build --release` を実行します。binary は `target/release/kotonoha-server` にできます。

API の鍵を repo の外の file に 1 行 1 個で書きます。空行と `#` で始まる行は読み飛ばします。
鍵が 1 個も無ければ起動しません。

```sh
mkdir -p ~/.config/kotonoha-server
openssl rand -hex 32 > ~/.config/kotonoha-server/api-keys
chmod 600 ~/.config/kotonoha-server/api-keys
./kotonoha-server
```

| 環境変数 | 既定 | 意味 |
|---|---|---|
| `PORT` | `5013` | 待ち受けるポート。全 IPv4 インターフェースで待ち受けます |
| `LLAMA_SERVER_URL` | `http://127.0.0.1:5012` | 上流の llama-server |
| `API_KEYS_FILE` | `~/.config/kotonoha-server/api-keys` | 鍵の file |
| `LOG_LEVEL` | `info` | `off` `error` `warn` `info` `debug` `trace` |

起動すると `listening addr=0.0.0.0:5013` を出力します。SIGTERM か Ctrl+C で止まります。
別の端末で、鍵なしで答える `/health` を確かめます。

```sh
curl -s localhost:5013/health   # {"status":"ok","version":"0.1.0"}
KEY=$(grep -v '^#' ~/.config/kotonoha-server/api-keys | head -1)
```

## 呼び出す

`/health` 以外は `Authorization: Bearer <鍵>` が要ります。鍵が無い・違う要求には 401 を返します。
誤った要求は 400、上流の失敗や壊れた答えは 502 で、どちらも `{"error": "…"}` を返します。

### /mask

```sh
curl -s localhost:5013/mask -H "Authorization: Bearer $KEY" -H 'content-type: application/json' \
  -d '{"text": "// 担当: 山田 太郎 <taro@acme.example.co.jp>\nconst HOST = \"10.0.0.5\";", "filename": "app.ts"}'
```

```json
[
  {"text": "山田 太郎", "type": "person"},
  {"text": "taro@acme.example.co.jp", "type": "mail"},
  {"text": "10.0.0.5", "type": "ip"}
]
```

`filename` は任意で、Gemma が文脈を読む手がかりにします。返すのは伏せる文字列と区分だけで、
置き換えは呼び出し側（CLI）が行います。結果は入力に現れる順で、同じ文字列は一度だけ返します。

| 区分 | 対象 | 拾う仕組み |
|---|---|---|
| `mail` | メールアドレス | 正規表現 |
| `ip` | IPv4 アドレス | 正規表現 |
| `ipv6` | IPv6 アドレス | 正規表現 |
| `url` | `scheme://` で始まる URL | 正規表現 |
| `phone` | 日本の電話番号（`03-1234-5678`・`09012345678`・`+81-90-…` など） | 正規表現 |
| `secret` | API の鍵（`sk-`・`ghp_`・`AKIA`・`xoxb-`・`AIza`・JWT・秘密鍵の PEM・Bearer）と、`password` `token` `api_key` `パスワード` などに代入された値 | 正規表現 |
| `person` | 人名。氏名を返したときは、単独で書かれた姓・名も伏せる | Gemma |
| `company` | 会社名・組織名・部署名 | Gemma |
| `host` | 社内のホスト名・サーバー名・内部のドメイン名 | Gemma |
| `project` | 案件名・プロジェクト名・コードネーム | Gemma |
| `internal_id` | 社員番号・顧客番号・チケット番号など社内の ID | Gemma |

型の決まった区分は、上の形に当てはまるものを Gemma を介さず正規表現で拾います。
Gemma には型の無い区分だけを JSON Schema で
配列の形に強制して答えさせ、入力に現れない文字列と 1 文字以下の文字列は捨てます。
候補が重なれば長い方を残し、同じ範囲なら正規表現の区分を採ります。
例えば URL の中の IP は URL として一つで返ります。
コードの参照に見える代入（`config.token`、`API_SECRET` のような定数名、関数呼び出し）は値とみなしません。

### /structure

```json
{
  "schema": {"type": "object", "properties": {"total_yen": {"type": "integer"}}, "required": ["total_yen"]},
  "text": "任意の文章",
  "files": [{"mime_type": "application/pdf", "data": "<base64>"}],
  "prompt": "任意の補足の指示"
}
```

`text` か `files` の少なくとも一方が要ります。`files` は Gemini の `inline_data` と同じ形です。
受ける形式は `image/png` `image/jpeg` `image/webp` `image/gif` `application/pdf` です。
PDF は頁ごとに 150 dpi の PNG にして渡し、20 頁を超えるものは断ります。
答えは `schema` どおりの JSON をそのまま返します。Schema の `description` に書いた意味と書式も読み取りの指示になります。

### /transcript

```json
{"text": "えーそれでは定例を始めます司会の山口です…"}
```

```json
{
  "text": "山口: それでは定例を始めます。司会の山口です。…",
  "segments": [{"speaker": "山口", "text": "それでは定例を始めます。司会の山口です。…"}],
  "points": ["新しい在庫管理画面のリリースが予定通り完了した。"]
}
```

つなぎ言葉と言い直しを除き、誤変換を直し、句読点を補います。要約はせず、話された内容は全て残します。
`text` は `segments` を並べた本文で、話者が 2 人以上なら行頭に話者を付けます。
話者が分からない発言の `speaker` は「不明」です。

## thinking

Gemma 4 の thinking は 3 つの窓口とも止めています（`chat_template_kwargs.enable_thinking=false`）。

| 窓口 | thinking | 理由 |
|---|---|---|
| `/mask` | なし | `eval/mask/` の 5 件で、thinking なしの区分ごとの再現率は 93〜100%。thinking ありは 1 件目で推論だけで 8192 token を使い切った |
| `/structure` | なし | 作り物の請求書の画像で読み取った金額・日付・明細は同じで、thinking ありは 40 秒、なしは 22 秒 |
| `/transcript` | なし | 作り物の会議の文字起こしで、発言の分け方と要点の件数は同じで、thinking ありは 36 秒、なしは 4 秒 |

## 長い入力

`/mask` は 12,000 token、`/transcript` は 6,000 token を超える入力を、行の境目で分けて Gemma に渡し、
結果を合わせます。token 数は非 ASCII の 1 文字を 1 token、ASCII の 3 byte を 1 token と多めに見積もります。
`/mask` の正規表現は分けずに全文へかけます。行をまたぐ固有名詞は分け目で拾えないことがあります。
`/transcript` で分けた場合、話者の呼び名や要点が片ごとにそろわないことがあります。

## 開発

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

テストでは Gemma を stub に置き換えます。`eval/mask/` には作り物の機密と正解の区分を付けた評価の文があり、
正規表現の区分は `cargo test` が全件を拾うことを確かめます。Gemma の区分の再現率は、
動いている llama-server を相手に次で出します。

```sh
LLAMA_SERVER_URL=http://127.0.0.1:5012 cargo run --release --example mask_eval
```

`v*` の tag を push すると、GitHub Actions が Linux x86_64 の静的 binary を build して Releases に公開します。

## License

MIT
