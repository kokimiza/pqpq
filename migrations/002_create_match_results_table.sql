-- ============================================================
-- match_results: 署名付き戦績テーブル
-- ============================================================
-- 対戦終了時に敗者がEd25519署名を付与した結果を記録する。
-- 署名により戦績の偽装を防止。
-- ランキング・統計はこのテーブルから集計する。

CREATE TABLE IF NOT EXISTS match_results (
    id              UUID        PRIMARY KEY DEFAULT gen_random_uuid(),

    -- 対戦者（Ed25519公開鍵 hex）
    winner_pubkey   TEXT        NOT NULL,
    loser_pubkey    TEXT        NOT NULL,

    -- 対戦詳細
    winner_hp       SMALLINT    NOT NULL CHECK (winner_hp BETWEEN 0 AND 255),
    loser_hp        SMALLINT    NOT NULL DEFAULT 0 CHECK (loser_hp BETWEEN 0 AND 255),
    duration_frames INTEGER     NOT NULL CHECK (duration_frames > 0),
    finish_type     TEXT        NOT NULL DEFAULT 'ko'
                                CHECK (finish_type IN ('ko', 'timeout', 'disconnect')),

    -- 署名検証
    -- raw_payload: 署名対象のバイナリをBase64エンコードしたもの
    -- loser_signature: 敗者による署名（Base64）
    raw_payload     TEXT        NOT NULL,
    loser_signature TEXT        NOT NULL,

    -- シグナリングとの紐付け（任意）
    ring_id         UUID        REFERENCES rings(id) ON DELETE SET NULL,

    -- タイムスタンプ
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- ランキング・戦績検索用インデックス
CREATE INDEX idx_match_results_winner     ON match_results (winner_pubkey);
CREATE INDEX idx_match_results_loser      ON match_results (loser_pubkey);
CREATE INDEX idx_match_results_created_at ON match_results (created_at DESC);
CREATE INDEX idx_match_results_ring       ON match_results (ring_id) WHERE ring_id IS NOT NULL;

-- ============================================================
-- player_stats: プレイヤー統計マテリアライズドビュー
-- ============================================================
-- match_resultsから集計。定期的にREFRESHする想定。
-- Supabase無料枠ではpg_cronが使えない場合、
-- アプリ側で REFRESH MATERIALIZED VIEW を呼ぶ。

CREATE MATERIALIZED VIEW IF NOT EXISTS player_stats AS
SELECT
    pubkey,
    COUNT(*) FILTER (WHERE is_winner)                       AS wins,
    COUNT(*) FILTER (WHERE NOT is_winner)                   AS losses,
    COUNT(*)                                                AS total_matches,
    ROUND(
        COUNT(*) FILTER (WHERE is_winner) * 100.0 / NULLIF(COUNT(*), 0),
        1
    )                                                       AS win_rate,
    COUNT(*) FILTER (WHERE finish_type = 'disconnect' AND NOT is_winner)
                                                            AS disconnects,
    MAX(created_at)                                         AS last_played_at
FROM (
    SELECT winner_pubkey AS pubkey, TRUE AS is_winner, finish_type, created_at
    FROM match_results
    UNION ALL
    SELECT loser_pubkey AS pubkey, FALSE AS is_winner, finish_type, created_at
    FROM match_results
) AS combined
GROUP BY pubkey;

-- ランキング検索用
CREATE UNIQUE INDEX idx_player_stats_pubkey ON player_stats (pubkey);
CREATE INDEX idx_player_stats_wins          ON player_stats (wins DESC);
CREATE INDEX idx_player_stats_win_rate      ON player_stats (win_rate DESC)
    WHERE total_matches >= 10;
