# pqpq

ターミナルとブラウザから、名前と部屋IDだけで同じレースへ参加するゲームです。1コースを3周し、全員がReadyになると開始します。同名参加、観戦、衝突、順位、予測・補正・補間に対応します。

5クレートのCargo Workspaceです。`pqpq-client`がCUI、`pqpq-web`がWASM、`pqpq-server`が単一のゲームサーバ、`pqpq-protocol`が通信形式、`pqpq-sim`が共通の物理計算を担当します。Native QUICはquiche、WebTransportはwtransportを使用します。

## ビルド

Rust、`wasm32-unknown-unknown`、Node.jsを使用します。ネイティブ版のビルドにはC/C++ツールチェーンとquiche/BoringSSLのビルド環境が必要です。WindowsではMSVC Build Tools・CMake・NASM・libclangの場所を確認してください。[quicheのビルド手順](https://github.com/cloudflare/quiche#building)

リポジトリのルートで実行します。

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.129 --locked --root target/wasm-tools
node scripts/build-web.mjs
cargo build --workspace --all-targets --locked
```

WASMとJSは `web/pkg/`、ネイティブ実行ファイルは `target/debug/` に生成します。既存のサーバを起動したままWindowsで再ビルドする場合は、実行ファイルのロックを避けるため `--target-dir target/verify` を指定できます。

## ローカルで遊ぶ

ブラウザのWebTransportにはTLS証明書が必要です。ローカルでは14日有効のECDSA証明書を作り、HTTPS経由でその証明書のSHA-256をブラウザへ渡します。ChromiumではローカルCAをHTTPSで信頼してもWebTransportで信頼しない場合があるため、この標準の証明書ハッシュ方式を使います。[WebTransportの証明書ハッシュ仕様](https://www.w3.org/TR/webtransport/#certificate-hashes)

```sh
cargo run -p pqpq-server --example dev_cert
```

生成処理は既存の証明書を上書きせず、OSの信頼ストアも変更しません。期限切れ時には別ディレクトリに生成し、`PQPQ_TLS_CERT` / `PQPQ_TLS_KEY`で指定します。

PowerShellのサーバ用ターミナルで起動します。

```powershell
$env:PQPQ_PIN_WEB_CERT = 'true'
cargo run -p pqpq-server
```

Linux / macOSでは `PQPQ_PIN_WEB_CERT=true cargo run -p pqpq-server` です。

ブラウザで `https://localhost:8443/#room=1234` を開きます。自己署名証明書なのでHTTPSの警告が出ます。自分で起動したlocalhostであることを確認して開発用の例外を許可し、名前を入力してJOINします。これはローカル検証用の手順です。公開サーバでは公開CAの証明書を用い、例外許可もハッシュ設定も不要です。

同じPCの別ターミナルでは、生成した証明書を明示して接続します。

PowerShell:

```powershell
$env:PQPQ_CA_FILE = (Resolve-Path certs/localhost.pem).Path
cargo run -p pqpq-client -- foo 1234
```

Linux / macOS:

```sh
PQPQ_CA_FILE=certs/localhost.pem cargo run -p pqpq-client -- foo 1234
```

2人以上が参加して全員Readyになると3秒後にスタートします。途中参加は観戦です。全員退出すると部屋と結果を削除し、同じ部屋IDで新しく始められます。

| 操作 | キー |
| --- | --- |
| アクセル | ↑ / W |
| ブレーキ | ↓ / S |
| 左右 | ← → / A D |
| Ready | Enter / WebのREADYボタン |
| 退出 | Q / CUIのCtrl+C / Webの退出ボタン |

CUIは80列×24行以上を使用します。キー解放通知のない端末では短時間の入力保持による互換モードになります。ブラウザのフォーカス喪失・非表示時は入力を中立化し、レース自体はサーバで進行します。

## 接続先と運用

通常設定ではNative QUICはUDP 4433、WebTransportはUDP 8443、Web配信はTCP 8443でループバック待受します。TCPだけを許可してもレースには接続できません。

| 環境変数 | 既定値・用途 |
| --- | --- |
| `PQPQ_BIND_ADDR` | `127.0.0.1:4433`、Native UDP |
| `PQPQ_WEBTRANSPORT_ADDR` | `127.0.0.1:8443`、WebTransport UDP |
| `PQPQ_HTTPS_ADDR` | `127.0.0.1:8443`、HTTPS TCP |
| `PQPQ_WEB_ORIGIN` | `https://localhost:8443`、参加を許可するWebページのOrigin |
| `PQPQ_WEB_ROOT` | `web`、静的配布物 |
| `PQPQ_TLS_CERT` / `PQPQ_TLS_KEY` | `certs/localhost.pem` / `certs/localhost-key.pem` |
| `PQPQ_PIN_WEB_CERT` | `false`。開発用の14日以内・ECDSA証明書では `true` |
| `PQPQ_SERVER_ADDR` | クライアント接続先。既定 `localhost:4433` |
| `PQPQ_TLS_SERVER_NAME` | 接続先証明書の名前。既定は接続先ホスト |
| `PQPQ_CA_FILE` | ネイティブクライアントの信頼するCAのPEM |
| `PQPQ_MAX_CONNECTIONS` / `PQPQ_MAX_ROOMS` | 128 / 64 |
| `PQPQ_MAX_ROOM_PLAYERS` / `PQPQ_MIN_RACERS` | 32 / 2。観戦者も接続人数に含む。1人検証では最少人数を1にする |

公開時は公開ホストに有効な証明書を指定し、HTTPSとWebTransportを同じ公開ホスト・ポートで提供します。Nativeの接続先も配布先へ設定します。秘密鍵はWeb配信ディレクトリ外に置きます。サーバは1プロセスで静的配信も行うため、別のWebサーバやDBは不要です。

全ゲーム状態はRAMのみです。サーバ再起動で復元せず、再接続は新しいPlayerになります。ブラウザのCookie・Web Storageへプレイヤー情報を保存しません。証明書・配布物・試験の出力はゲーム履歴とは別に扱います。

## 検証

```sh
cargo fmt --all -- --check
cargo test --workspace --locked
node scripts/build-web.mjs
node web/check.mjs
```

実際のChromiumとNative QUICをつないだ受入試験:

```sh
cd web
npm ci
npx playwright install chromium
npm run test:crossplay
npm run test:crossplay -- --impaired
```

この試験は検証用サーバと短期証明書を作り、既存サーバとは別ポートで動作します。ブラウザはテスト証明書の公開鍵だけをHTTPSで信頼する専用プロセスを使い、WebTransportでは実際のアプリのハッシュ設定を使います。OSの信頼ストアは変更しません。テスト用の操作で両者を3周走らせ、確定結果の一致・退出後の部屋削除・繰り返し再参加を検証します。テスト用の自動操作は通常のゲームには組み込みません。

`--impaired` は両クライアントのQUICパケットに双方向5%欠落、片道25±20msの遅延、10%のパケットへの追加50ms遅延による順序逆転を加えます。制御StreamとDatagramの両方が同じ転送器を通ります。

実行結果とスクリーンショットは `target/crossplay/run-*/` に出力します。異なるOS・ブラウザの組み合わせやネットワーク劣化試験は、[基本設計書](docs/basic-design.md)の受入条件に沿って実施します。

詳細は[要件定義](docs/requirements.md)、[基本設計書](docs/basic-design.md)、[WASMのビルド手順](web/README.md)を参照してください。
