# 🥊 pqpq

**P**eer-to-peer **Q**uick **P**unch **Q**uest

ターミナル上で動作する、リアルタイム・フルP2P格闘ゲーム。  
中央集権的なAPIサーバーを排除し、**PostgreSQL (Supabase)** をシグナリングとランキング基盤として活用する、サーバーレス・アーキテクチャの実験作。

> 🎯 **コンセプト:** "ロジックはクライアント、データはDB、通信はP2P"  
> 従来の「バックエンドがゲームロジックを持つ」構成を完全に排除し、Rust製バイナリに全てを集約。Supabaseは純粋な「データの墓場」として機能します。

---

## 🌟 技術的新奇性：3つのエッジ

### 1. 🎮 Rollback Netcode in Rust
物理的距離による遅延を吸収するため、**GGPO スタイルのロールバック・ネットコード**を実装。  
WebRTC DataChannel で送信するのは「ゲーム状態」ではなく「入力（Input）」のみ。予測が外れた場合、数フレーム前の状態に巻き戻して再シミュレート。

- Rust の `Copy` トレイトを活用した軽量な状態保存
- 東京-ロンドン間でも体感遅延を最小化
- 格闘ゲームの「読み合い」を物理法則から解放

### 2. 📡 P2P カスケード配信（観戦モード）
`pqpq watch` 実装時、ホストに負荷を集中させない **メッシュ・ストリーミング** を採用。  
観戦者Bはホストからデータを受け取り、次の観戦者Cにリレー。ASCII文字列データ（数KB/s）なので、ブラウザ配信より遥かに効率的。

- ホストの帯域を節約
- 観戦者数が増えても線形スケール
- ターミナルならではの超軽量配信

### 3. 🏆 Ghost in the DB：自律的ランキング集計
`match_results` が溜まる中、フロントエンドが毎回全件集計するのは非効率。  
PostgreSQL の **Materialized View** を活用し、`pg_cron` で1時間ごとにイロレーティングを自動計算。

```sql
-- 自律的に動くランキングエンジン
CREATE MATERIALIZED VIEW player_rankings AS
SELECT 
    pubkey,
    COUNT(*) FILTER (WHERE winner_pubkey = pubkey) AS wins,
    COUNT(*) FILTER (WHERE loser_pubkey = pubkey) AS losses,
    calculate_elo_rating(pubkey) AS rating
FROM match_results
GROUP BY pubkey
ORDER BY rating DESC;
```

`pqpq rank` を叩くだけで、DB側で計算済みの「最強エンジニア・ランキング」を即座に取得。

---

## 🏗️ システムアーキテクチャ：The Stateless Stack

ロジックをバイナリ（Rust）側に寄せ、バックエンドは純粋なデータストアとして運用します。

* **Frontend/Engine:** Rust + Ratatui (TUI)
* **Networking:** `webrtc-rs` (P2P DataChannel / UDP)
* **Signaling & Persistence:** Supabase (PostgreSQL + Realtime + RLS)
* **Security:** Ed25519 署名 + Row Level Security (RLS)
* **Netcode:** GGPO-style Rollback (Input-only transmission)

### アーキテクチャ図

```
┌─────────────────────────────────────────────────────────────┐
│                    Rust Binary (pqpq)                       │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐     │
│  │ Ratatui TUI  │  │ Rollback     │  │ Ed25519      │     │
│  │ (80x24 ASCII)│  │ Netcode      │  │ Signer       │     │
│  └──────────────┘  └──────────────┘  └──────────────┘     │
│         │                  │                  │             │
│         └──────────────────┴──────────────────┘             │
│                           │                                 │
└───────────────────────────┼─────────────────────────────────┘
                            │
        ┌───────────────────┼───────────────────┐
        │                   │                   │
   WebRTC P2P          Supabase           WebRTC P2P
   (Input Only)     (Signaling Only)    (Input Only)
        │                   │                   │
        │            ┌──────┴──────┐            │
        │            │ PostgreSQL  │            │
        │            │  - rings    │            │
        │            │  - results  │            │
        │            │  - rankings │            │
        │            │    (M.View) │            │
        │            └─────────────┘            │
        │                                       │
        └───────────────────┬───────────────────┘
                            │
                    ┌───────┴───────┐
                    │  Peer Client  │
                    │   (Opponent)  │
                    └───────────────┘
```

---

## 🔄 接続シーケンス：The Zero-Logic Signaling

### Phase 1: リング設営 (Host)

1. `pqpq host` を実行。Ed25519 キーペアを一時生成。
2. Supabase の `rings` テーブルに `INSERT`。
* `token`: 招待コード
* `host_offer`: WebRTC Offer SDP
* `host_pubkey`: 自身の公開鍵


3. Supabase Realtime で自身のレコードの `UPDATE` を購読（Listen）開始。

### Phase 2: 参加 (Guest)

1. `pqpq join <TOKEN>` を実行。
2. `rings` テーブルから `host_offer` を取得。
3. 自身の `answer_sdp` を生成し、該当レコードを `UPDATE`。
4. P2P 接続待機状態へ。

### Phase 3: ゴング (P2P Established)

1. Host が Realtime 通知を受け取り、`answer_sdp` を取得。
2. P2P 接続が確立。以降のゲームデータ（座標・攻撃）は **Supabase を一切介さない。**
3. 対戦終了後、勝者が署名付きリザルトを `match_results` へ送信。

---

## 3. データモデル & 放置死対策 (PostgreSQL)

### テーブル設計

```sql
-- リング管理（シグナリング用）
CREATE TABLE rings (
    id UUID PRIMARY KEY DEFAULT uuid_generate_v4(),
    token TEXT UNIQUE NOT NULL,
    host_sdp TEXT NOT NULL,
    guest_sdp TEXT,
    host_pubkey TEXT NOT NULL,
    status TEXT DEFAULT 'open', -- open | matched
    created_at TIMESTAMPTZ DEFAULT NOW()
);

-- 戦績管理（ランキング用）
CREATE TABLE match_results (
    id SERIAL PRIMARY KEY,
    winner_pubkey TEXT NOT NULL,
    loser_pubkey TEXT NOT NULL,
    signature TEXT NOT NULL, -- 敗者による署名（偽装防止）
    raw_data TEXT NOT NULL,    -- "winner:A, loser:B, score:2-0" 形式の文字列
    created_at TIMESTAMPTZ DEFAULT NOW()
);

```

### 放置死（Ghost Record）対策

Supabase の `pg_net` または `pg_cron` エクステンションを使用し、DB 側で自浄作用を持たせます。

```sql
-- 15分以上経過した未成立のリング、または成立済みのリングを自動削除
SELECT cron.schedule('cleanup-rings', '*/5 * * * *', 
    $$ DELETE FROM rings WHERE created_at < NOW() - INTERVAL '15 minutes' $$
);

```

---

## 4. 偽装対策：Evidence-Based Ranking

ランキングの信頼性を担保するため、対戦終了時に **「敗者の署名」** を取得する仕組みを導入します。

1. **試合終了:** クライアント A が勝利。
2. **署名要求:** A は B に対して「Aが勝利した」というデータの署名を P2P 経由で要求。
3. **合意:** B のクライアントが（整合性を確認し）自身の秘密鍵で署名して A に返送。
4. **提出:** A は「Bの署名付きリザルト」を Supabase に `INSERT`。
5. **検証:** Supabase の **RLS** または **Check Constraint** により、署名が不正なデータは DB への書き込みを拒否。

> [!TIP]
> 敗者が悔しくてシグナルを切った（署名を拒否した）場合は「無効試合」となりますが、この「切断率」も統計として残すことで、マナーの悪いプレイヤーを可視化できます。

---

## 5. UI/UX：Terminal Brutalism

* **Visuals:** ASCIIアートキャラが `80x24` のキャンバスで激突。
* **Latency:** `webrtc-rs` による 20ms 以下の超低遅延バトル。
* **CLI First:** * `pqpq list`: 現在立っている（Openな）リング一覧を表示。
* `pqpq watch <TOKEN>`: (Future) 観戦モード。
