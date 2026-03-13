-- ============================================================
-- rings: P2Pシグナリング用の一時テーブル
-- ============================================================
-- ホストがリングを立て、ゲストが参加するまでの
-- WebRTC SDP交換を仲介する。対戦開始後は不要になる。
-- 15分経過で自動削除（pg_cronまたはアプリ側で実施）。

CREATE TABLE IF NOT EXISTS rings (
    id            UUID        PRIMARY KEY,
    token         TEXT        UNIQUE NOT NULL,

    -- WebRTC シグナリング
    host_sdp      TEXT        NOT NULL,
    guest_sdp     TEXT,

    -- プレイヤー識別（Ed25519公開鍵 hex）
    host_pubkey   TEXT        NOT NULL,
    guest_pubkey  TEXT,

    -- リング状態
    status        TEXT        NOT NULL DEFAULT 'open'
                              CHECK (status IN ('open', 'matched', 'expired')),

    -- タイムスタンプ
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at    TIMESTAMPTZ NOT NULL DEFAULT (NOW() + INTERVAL '15 minutes')
);

-- 検索用インデックス
CREATE INDEX idx_rings_token      ON rings (token);
CREATE INDEX idx_rings_status     ON rings (status) WHERE status = 'open';
CREATE INDEX idx_rings_expires_at ON rings (expires_at);

-- 期限切れリングの自動クリーンアップ用関数
CREATE OR REPLACE FUNCTION cleanup_expired_rings()
RETURNS INTEGER AS $$
DECLARE
    deleted_count INTEGER;
BEGIN
    DELETE FROM rings
    WHERE expires_at < NOW()
      AND status != 'matched';
    GET DIAGNOSTICS deleted_count = ROW_COUNT;
    RETURN deleted_count;
END;
$$ LANGUAGE plpgsql;

-- ============================================================
-- Row Level Security (RLS) ポリシー設定
-- ============================================================
-- Supabaseを使用する場合、以下のRLSポリシーを設定してください：
--
-- 1. RLSを有効化:
--    ALTER TABLE rings ENABLE ROW LEVEL SECURITY;
--
-- 2. 全ユーザーに読み取り権限を付与（匿名ユーザー含む）:
--    CREATE POLICY "Allow public read access"
--    ON rings FOR SELECT
--    USING (true);
--
-- 3. 全ユーザーに挿入権限を付与（ホストがリングを作成）:
--    CREATE POLICY "Allow public insert"
--    ON rings FOR INSERT
--    WITH CHECK (true);
--
-- 4. 全ユーザーに更新権限を付与（ゲストがguest_sdpを設定）:
--    CREATE POLICY "Allow public update"
--    ON rings FOR UPDATE
--    USING (true)
--    WITH CHECK (true);
--
-- 5. 削除権限（オプション、クリーンアップ用）:
--    CREATE POLICY "Allow public delete"
--    ON rings FOR DELETE
--    USING (true);
--
-- 注意: 本番環境では、より厳密なポリシー（例: tokenやpubkeyによる認証）を
--       設定することを推奨します。現在の設定は開発・テスト用です。
--
-- RLSポリシーが正しく設定されていないと、以下の問題が発生します：
-- - ホスト側のポーリングでguest_sdpが検出されない（SELECT権限不足）
-- - ゲスト側のupdateが失敗する（UPDATE権限不足）
-- - 異なる接続プール間でデータが見えない（READ COMMITTED分離レベルでも）
-- ============================================================
