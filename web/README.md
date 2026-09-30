# pqpq Webクライアント

`pqpq-web`をWASMへビルドして使用するWebクライアントです。参加フォーム・Canvas描画・WebTransport通信をJS、codec・共有車両計算・予測・補正・補間をRust/WASMが担当します。ターミナル版と同じ部屋へ参加します。ゲーム全体の起動手順は[ルートREADME](../README.md)、仕様は[基本設計書](../docs/basic-design.md)を参照してください。

## ビルド

リポジトリのルートで実行します。wasm-bindgen CLIの版は `pqpq-web/Cargo.toml` の依存と一致させます。以下は既存のグローバルCLIを変更せず、プロジェクト内へインストールする手順です。

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.129 --locked --root target/wasm-tools
cargo build -p pqpq-web --target wasm32-unknown-unknown --release
./target/wasm-tools/bin/wasm-bindgen target/wasm32-unknown-unknown/release/pqpq_web.wasm --target web --out-dir web/pkg
```

PowerShellでは最後のコマンドの実行ファイルを `./target/wasm-tools/bin/wasm-bindgen.exe` とします。生成先は `web/pkg/pqpq_web.js` と `web/pkg/pqpq_web_bg.wasm` です。

## ローカルでの起動確認

ルートREADMEの手順で14日有効の開発用証明書を作ります。PowerShellではリポジトリのルートから次を実行します。

```powershell
cargo run -p pqpq-server --example dev_cert
$env:PQPQ_PIN_WEB_CERT = 'true'
cargo run -p pqpq-server
```

`https://localhost:8443/` を開き、自分のローカル証明書に限ってHTTPSの開発用例外を許可し、名前と部屋IDを入力してJOINします。Linux / macOSでは `PQPQ_PIN_WEB_CERT=true cargo run -p pqpq-server` を使用します。公開時は公開CAの証明書を指定し、ハッシュ設定を無効にします。

HTTPSの静的配信・`transport-config.json`・WebTransport接続をpqpq-serverが担当します。静的ファイルだけをHTTP配信してもゲームには接続できません。

`.wasm` を `file://` で直接開く方法は使用しません。生成物の `web/pkg/` とビルド用の `target/` はGitの管理対象外です。

## 自動確認

ルートから `node scripts/build-web.mjs` でWASMを生成し、`node web/check.mjs` でNativeとWASMの共通テストベクトルを照合できます。

`web/`で `npm ci`、`npx playwright install chromium` を実行した後、`npm run test:crossplay` で実ブラウザとNative QUICの混在レースを確認できます。試験専用の証明書を生成するため、通常プレイ用のCAをインストールせずに実行できます。
