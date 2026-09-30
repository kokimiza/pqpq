# pqpq 要件定義

## 1. 概要

`pqpq` は、ターミナルまたはWebブラウザから参加できる、リアルタイム対戦型レーシングゲームである。

プレイヤーはアカウントを作成せず、以下の2つのみを指定してゲームへ参加する。

- ユーザ名
- 部屋ID

CUIクライアントでは次の形式で起動する。

```bash
pqpq foo 1234
```

この場合、ユーザ名 `foo` として部屋 `1234` に参加する。

Webブラウザでは、同様にユーザ名と部屋IDを入力して参加する。

CUIクライアントとWebクライアントは、同じゲームサーバ上の同じ部屋へ参加できる。

```text
Terminal
$ pqpq foo 1234
        │
        │
        ▼
    Room 1234
        ▲
        │
Browser
name=bar
room=1234
```

`foo` と `bar` は異なるUIを利用しているが、同じレース世界を共有する。

---

# 2. プロダクトコンセプト

pqpqの基本思想は、

> URLまたはCLIコマンドだけで、その場限りのレースに参加できること

とする。

アカウント作成、ログイン、戦績保存、プロフィール管理などを要求しない。

プレイヤーはその場で名前を名乗り、部屋IDを共有して集まり、レースを行う。

レースが終われば、何も残さない。

---

# 3. 恒久的な設計原則

pqpqは、以下を一時的な簡略化ではなく、恒久的な設計原則とする。

```text
0 database
0 account
0 authentication
0 persistent session
0 persistent player data
```

データベースは将来追加を想定しない。

認証機能も将来追加を前提としない。

pqpqにおいて永続性を持つものは、原則としてソースコードと配布バイナリのみとする。

---

# 4. システム全体構成

システムは以下の3層で構成する。

```text
Presentation
├── Terminal / Ratatui
└── Browser

Shared Game Logic
├── Protocol
└── Simulation

Game Server
└── Authoritative State
```

実行時の概念構成は以下とする。

```text
┌──────────────────┐
│ pqpq-client      │
│ Ratatui          │
│ Native QUIC      │
└────────┬─────────┘
         │
         │ QUIC
         │
         ▼
┌──────────────────────────┐
│ pqpq-server              │
│                          │
│ Server Authoritative     │
│ Rooms                    │
│ Game Loop                │
│ Simulation               │
│                          │
└───────────┬──────────────┘
            ▲
            │
            │ WebTransport / HTTP/3
            │
┌───────────┴──────────────┐
│ pqpq-web                 │
│ Browser                  │
│ WASM / Web UI            │
└──────────────────────────┘
```

---

# 5. リポジトリ構成

pqpqはCargo Workspaceによるモノレポとする。

```text
pqpq/
├── Cargo.toml
├── Cargo.lock
├── crates/
│   ├── pqpq-client/
│   ├── pqpq-server/
│   ├── pqpq-protocol/
│   ├── pqpq-sim/
│   └── pqpq-web/
└── web/
```

ルート:

```toml
[workspace]
resolver = "3"
members = ["crates/*"]
```

---

# 6. クレート責務

## 6.1 pqpq-client

ネイティブCUIクライアント。

```text
責務:
- CLI
- キーボード入力
- Ratatui描画
- Native QUIC通信
- Client-side Prediction
- Interpolation
- Reconciliation
```

バイナリクレートとする。

最終的な実行ファイル名は、

```text
pqpq
```

とする。

---

## 6.2 pqpq-server

ゲーム世界の正解を管理するゲームサーバ。

```text
責務:
- QUIC接続
- Webクライアント接続
- Room管理
- Player管理
- 入力受付
- 固定Tickゲームループ
- 衝突判定
- ラップ判定
- 順位判定
- GameSnapshot配信
```

バイナリクレートとする。

---

## 6.3 pqpq-protocol

client / server / web間で交換するメッセージ形式を定義する。

```text
責務:
- PlayerId
- RoomId
- Input
- Join
- Ready
- RaceStart
- Snapshot
- RaceFinished
- エンコード形式
```

QUICそのものは実装しない。

`quiche` は通信方式を提供する。

`pqpq-protocol` はその通信上を流れるpqpq独自データを定義する。

---

## 6.4 pqpq-sim

ゲーム物理およびゲーム状態遷移を定義する。

```text
責務:
- CarState
- InputState
- 車両移動
- 加減速
- ステアリング
- コース制約
- 衝突
- ラップ判定
```

最も重要な要件として、

```text
server
native client
web client
```

の3者で可能な限り同一のシミュレーションコードを使用する。

これによりClient-side Predictionとサーバ計算との差を小さくする。

---

## 6.5 pqpq-web

ブラウザ版pqpqを提供する。

主な役割:

```text
- username入力
- room_id入力
- レース描画
- キーボード入力
- WebTransport通信
- Prediction
- Interpolation
- Reconciliation
```

必要な共有RustコードはWASMとして利用してよい。

---

# 7. 依存関係

基本的な依存方向は以下とする。

```text
pqpq-client ────────┐
                    ├── pqpq-protocol
pqpq-server ────────┤
                    └── pqpq-sim
pqpq-web ───────────┘
```

`pqpq-protocol` および `pqpq-sim` は上位アプリケーション層へ依存しない。

禁止:

```text
pqpq-protocol → pqpq-server
pqpq-protocol → pqpq-client

pqpq-sim → Ratatui
pqpq-sim → Web API
pqpq-sim → quiche
```

---

# 8. ユーザ参加

## 8.1 CUI

```bash
pqpq <username> <room_id>
```

例:

```bash
pqpq foo 1234
```

---

## 8.2 Web

Web画面では最低限、

```text
Name [ foo  ]
Room [ 1234 ]

[ JOIN ]
```

のみを要求する。

アカウント作成画面は設けない。

---

# 9. ユーザ名

ユーザ名は表示名のみとする。

一意性を要求しない。

同じ部屋に、

```text
foo
foo
foo
```

が存在してよい。

内部識別にはサーバが払い出す `PlayerId` を使用する。

例:

```text
PlayerId=41 Name=foo
PlayerId=42 Name=foo
PlayerId=43 Name=foo
```

---

# 10. Room

Roomはメモリ上のみ存在する。

```text
HashMap<RoomId, Room>
```

指定されたRoomIdが存在しなければ、新規作成する。

```text
pqpq foo 1234

1234 not found
↓
Room 1234 created
↓
foo joined
```

既に存在すれば、そのまま参加する。

専用の、

```text
create-room
join-room
```

操作は作らない。

同じ操作で両方を行う。

---

# 11. Roomの寿命

Roomは参加者がいる間だけ存在する。

```text
Not Exists
    ↓
Waiting
    ↓
Racing
    ↓
Finished
    ↓
Empty
    ↓
Deleted
```

全プレイヤーが退出したRoomは削除する。

Room IDはその時点で再利用可能となる。

---

# 12. データベース

データベースは使用しない。

以下の種類を含め、外部永続ストレージをゲーム要件として採用しない。

```text
PostgreSQL
MySQL
SQLite
Redis
DynamoDB
MongoDB
Object Storage
```

すべてのゲーム状態はRAMに保持する。

---

# 13. サーバ再起動

サーバプロセスが停止した場合、

```text
Rooms
Players
Races
Results
```

はすべて消失する。

復元しない。

クライアントは再度、

```bash
pqpq foo 1234
```

等を実行して新しいセッションへ参加する。

---

# 14. 認証

認証機能は持たない。

以下を実装しない。

```text
password
email
OAuth
MFA
JWT login
account recovery
profile
```

`foo` という名前を使用したユーザが、以前の `foo` と同一人物である保証は行わない。

これは仕様である。

---

# 15. 通信方式

## 15.1 Native Client

CUIクライアントはQUICを使用する。

実装にはCloudflare `quiche` を使用する。

```text
pqpq-client
    ↓
QUIC
    ↓
pqpq-server
```

---

## 15.2 Browser Client

ブラウザは任意UDPソケットを直接扱えないため、WebTransportを利用する。

```text
pqpq-web
    ↓
WebTransport
    ↓
HTTP/3 / QUIC
    ↓
pqpq-server
```

---

# 16. Transport抽象化

ゲームロジックは、

```text
Native QUIC
WebTransport
```

の違いを意識しないようにする。

アプリケーション層では最終的に、

```text
Reliable Message
Unreliable Message
```

の2種類として扱う。

---

# 17. Reliable通信

順序と到達保証が必要な情報にはQUIC Stream相当を使用する。

対象:

```text
Join
Joined
Ready
RaceStart
RaceFinished
PlayerJoined
PlayerLeft
```

---

# 18. Unreliable通信

最新状態が重要な情報はDatagramを使用する。

対象:

```text
Input
GameSnapshot
```

例:

```text
snapshot 100 received
snapshot 101 lost
snapshot 102 received
```

この場合、

```text
101 retransmit
```

を待たず、

```text
102
```

を利用する。

---

# 19. 入力方式

入力はイベントではなく、現在状態として送信する。

悪い例:

```text
ACCELERATOR_PRESSED
```

良い例:

```text
throttle = true
brake = false
steering = -1
```

これによりDatagramが1つ欠落しても、次の入力情報から現在状態を復元できる。

---

# 20. Input Sequence

各入力には単調増加するSequence番号を付与する。

例:

```text
100
101
102
103
```

例:

```rust
struct PlayerInput {
    sequence: u64,
    throttle: bool,
    brake: bool,
    steering: i8,
}
```

---

# 21. Server Authoritative

ゲーム状態の最終決定権はサーバのみが持つ。

クライアントが送信可能なのは基本的に入力のみとする。

クライアントは以下を確定できない。

```text
position
velocity
lap
rank
finish
collision result
```

---

# 22. 固定Tick

ゲームサーバは固定Tickで動作する。

初期値:

```text
30 Hz
```

約33.3msごとにゲーム世界を更新する。

処理:

```text
1. Input収集
2. 車両更新
3. 物理計算
4. 衝突判定
5. コース判定
6. ラップ判定
7. ゴール判定
8. 順位更新
9. Snapshot生成
10. Snapshot配信
```

---

# 23. Client Rendering

CUIおよびWeb UIはゲームTickと独立して描画する。

目標:

```text
60 FPS
```

ゲームロジックを描画FPSへ依存させない。

---

# 24. Client-side Prediction

自プレイヤーはサーバ応答を待たず、ローカルで移動結果を予測する。

処理:

```text
Keyboard Input
      ↓
Input(sequence=105)
      ↓
      ├── Serverへ送信
      │
      └── Local simulate()
             ↓
          Render
```

これによりネットワークRTTによる操作遅延を隠蔽する。

---

# 25. Input Buffer

クライアントは、サーバ処理が確認されていない入力を一時的に保持する。

例:

```text
105
106
107
108
```

---

# 26. Server Acknowledgement

Server Snapshotには、

```text
last_processed_input
```

を含める。

例:

```text
last_processed_input = 106
```

この場合クライアントは、

```text
105
106
```

を入力バッファから削除できる。

---

# 27. Reconciliation

サーバ状態を受信した場合、クライアントは、

```text
1. サーバ状態を正とする
2. acknowledged inputを削除
3. 未処理Inputを再実行
```

する。

例:

```text
Server:
state after #106

Client pending:
#107
#108

↓

server state
+ #107
+ #108

↓

new predicted state
```

---

# 28. Interpolation

他プレイヤーはPredictionではなくInterpolationを基本とする。

Snapshot間を補間して表示する。

```text
Snapshot A
      ↓
  interpolation
      ↓
Snapshot B
```

これによりネットワーク更新周期が30Hzでも、60FPS相当の滑らかな描画を可能にする。

---

# 29. Simulation共有

`pqpq-sim` の同じ計算ロジックを、

```text
pqpq-server
pqpq-client
pqpq-web/WASM
```

で共有する。

この設計により、

```text
server simulation
client prediction
browser prediction
```

の挙動差を小さくする。

---

# 30. 初期ゲーム仕様

初期ゲームは以下とする。

```text
Genre        Racing
Course       1
Lap          3
Vehicle      same spec
Collision    enabled
Ranking      enabled
Item         none
NPC          none
```

---

# 31. CUI

CUIにはRatatuiを使用する。

例:

```text
┌────────────────────────────────────────────┐
│ pqpq                  ROOM 1234            │
│                                            │
│ LAP 2 / 3                    POS 2 / 4     │
│                                            │
│       ╭───────────────────────────╮        │
│       │                           │        │
│       │  foo▶        bar▶         │        │
│       │                     baz▶  │        │
│       │                           │        │
│       ╰───────────────────────────╯        │
│                                            │
│ SPEED 142 km/h                PING 18 ms   │
└────────────────────────────────────────────┘
```

CUIはWeb版の簡易版ではなく、正式なpqpqクライアントの一つとする。

---

# 32. Web UI

Web版も同一ゲームへ参加できる正式クライアントとする。

最低限、

```text
Join screen
Race screen
Result screen
```

を提供する。

Web版独自のアカウント機能等は追加しない。

---

# 33. クロスプラットフォーム

以下の組み合わせで同一Roomへ参加できることを要求する。

```text
Windows terminal
Linux terminal
macOS terminal
Browser
```

クライアントの種類によってゲームルールを変更しない。

---

# 34. 公平性

CUI版とWeb版でゲームシミュレーション条件を同一とする。

以下をクライアント依存にしない。

```text
acceleration
top speed
turn rate
collision
lap condition
```

---

# 35. ローカル開発

Dockerを必須としない。

開発時は同一PC上で直接複数プロセスを起動する。

Terminal 1:

```bash
cargo run -p pqpq-server
```

Terminal 2:

```bash
cargo run -p pqpq-client -- foo 1234
```

Terminal 3:

```bash
cargo run -p pqpq-client -- bar 1234
```

Webブラウザ:

```text
localhost
name=baz
room=1234
```

結果:

```text
Room 1234

foo    Terminal
bar    Terminal
baz    Browser
```

---

# 36. ネットワーク試験

ローカル環境で人工的に以下を発生させて試験する。

```text
latency
jitter
packet loss
packet reorder
```

確認対象:

```text
QUIC
WebTransport
Prediction
Interpolation
Reconciliation
```

---

# 37. 配置

初期サーバ環境としてOracle Cloud Infrastructureの単一VMを想定する。

```text
OCI Osaka
└── VM
    └── pqpq-server
```

ゲームサーバ以外の常駐バックエンドサービスは原則設けない。

---

# 38. インフラ原則

通常構成では以下を持たない。

```text
DB server
Redis
Message Queue
Kubernetes
Load Balancer
Object Storage
Authentication Server
```

ユーザ数の増加時も、まず単一VMの性能拡張を優先する。

---

# 39. 性能

初期利用想定:

```text
1〜3 players
```

ただし、

```text
10
20
```

等の同時接続試験を行ってよい。

無料または小規模VMで性能低下が発生しても、それ自体を測定結果として扱う。

---

# 40. 高可用性

高可用性を要求しない。

```text
server down
↓
race gone

server restart
↓
all rooms gone
```

でよい。

---

# 41. セキュリティ境界

本人認証は行わないが、クライアントを信用するわけではない。

以下はサーバ側で必ず検証する。

```text
RoomId length
Username length
Message size
Message type
Input range
Packet frequency
Player existence
Room membership
```

異常なクライアントからの入力によってサーバ全体が停止しないことを要求する。

---

# 42. DoS対策

認証なしの公開UDP/QUICサービスであるため、最低限以下を行う。

```text
最大パケットサイズ
最大ユーザ名長
最大RoomId長
接続数上限
Room人数上限
入力頻度上限
```

大規模DDoS対策そのものはpqpqの責務外とする。

---

# 43. 明示的な非要件

以下はpqpqでは実装しない。

```text
Database
Account
Authentication
Password
Email
OAuth
MFA
Profile
Persistent Ranking
Persistent Statistics
Persistent Replay
Friend List
Matchmaking Service
Social Graph
Payment
Advertising
Kubernetes
Redis
Message Queue
```

---

# 44. 初期完成条件

以下を満たした時点を最初の完成とする。

1. `pqpq-server` が起動できる
2. `pqpq foo 1234` で参加できる
3. 存在しないRoomが自動作成される
4. 同じRoomへ別クライアントが参加できる
5. 同じユーザ名を使用できる
6. 内部PlayerIdで識別できる
7. Native QUICで接続できる
8. WebTransportで接続できる
9. TerminalとBrowserが同じRoomでプレイできる
10. Server Authoritativeでゲームが進行する
11. 30Hz固定Tickで動作する
12. Client-side Predictionが動作する
13. Reconciliationが動作する
14. Interpolationが動作する
15. 3周レースを完走できる
16. 順位を判定できる
17. Roomから全員退出するとRoomが消える
18. サーバ再起動ですべてのゲーム状態が消える

---

# 45. pqpqの定義

pqpqは、

```text
URLまたはCLIから、
名前と部屋IDだけで参加できる、
永続データを一切持たない、
QUICベースのリアルタイムCUI/Webレーシングゲーム
```

である。

技術構成の原則は、

```text
CUI + Web

Native QUIC + WebTransport

Server Authoritative

Shared Simulation

1 Game Server

0 Database
0 Account
0 Authentication
0 Persistence
```

とする。

ゲーム世界は、その部屋に誰かがいる間だけ存在する。

最後の一人が退出すれば、その世界も消える。