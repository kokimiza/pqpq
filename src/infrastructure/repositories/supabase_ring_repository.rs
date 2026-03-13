use crate::domain::entities::{Ring, RingStatus};
use crate::domain::repositories::RingRepository;
use crate::domain::value_objects::{PublicKey, Sdp, Token};
use crate::infrastructure::errors::InfrastructureError;
use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};
use uuid::Uuid;

pub struct SupabaseRingRepository {
    pool: PgPool,
}

impl SupabaseRingRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// DBの行からRingエンティティを復元する共通処理
    fn row_to_ring(row: &sqlx::postgres::PgRow) -> Result<Ring> {
        let id: Uuid = row.try_get("id").map_err(|e| {
            InfrastructureError::QueryExecution(format!("Failed to parse id: {}", e))
        })?;
        let token_str: String = row.try_get("token").map_err(|e| {
            InfrastructureError::QueryExecution(format!("Failed to parse token: {}", e))
        })?;
        let host_sdp_str: String = row.try_get("host_sdp").map_err(|e| {
            InfrastructureError::QueryExecution(format!("Failed to parse host_sdp: {}", e))
        })?;
        let guest_sdp_str: Option<String> = row.try_get("guest_sdp").map_err(|e| {
            InfrastructureError::QueryExecution(format!("Failed to parse guest_sdp: {}", e))
        })?;
        let host_pubkey_str: String = row.try_get("host_pubkey").map_err(|e| {
            InfrastructureError::QueryExecution(format!("Failed to parse host_pubkey: {}", e))
        })?;
        let guest_pubkey_str: Option<String> = row.try_get("guest_pubkey").map_err(|e| {
            InfrastructureError::QueryExecution(format!("Failed to parse guest_pubkey: {}", e))
        })?;
        let status_str: String = row.try_get("status").map_err(|e| {
            InfrastructureError::QueryExecution(format!("Failed to parse status: {}", e))
        })?;
        let created_at: DateTime<Utc> = row.try_get("created_at").map_err(|e| {
            InfrastructureError::QueryExecution(format!("Failed to parse created_at: {}", e))
        })?;
        let expires_at: DateTime<Utc> = row.try_get("expires_at").map_err(|e| {
            InfrastructureError::QueryExecution(format!("Failed to parse expires_at: {}", e))
        })?;

        let status = match status_str.as_str() {
            "matched" => RingStatus::Matched,
            "expired" => RingStatus::Expired,
            _ => RingStatus::Open,
        };

        let guest_sdp = match guest_sdp_str {
            Some(s) if !s.is_empty() => Some(Sdp::new(s)?),
            _ => None,
        };

        let guest_pubkey = match guest_pubkey_str {
            Some(s) if !s.is_empty() => Some(PublicKey::new(s)?),
            _ => None,
        };

        Ok(Ring::reconstruct(
            id,
            Token::new(token_str)?,
            Sdp::new(host_sdp_str)?,
            guest_sdp,
            PublicKey::new(host_pubkey_str)?,
            guest_pubkey,
            status,
            created_at,
            expires_at,
        ))
    }
}

const SELECT_COLUMNS: &str =
    "id, token, host_sdp, guest_sdp, host_pubkey, guest_pubkey, status, created_at, expires_at";

#[async_trait]
impl RingRepository for SupabaseRingRepository {
    async fn save(&self, ring: &Ring) -> Result<()> {
        let status = match ring.status() {
            RingStatus::Open => "open",
            RingStatus::Matched => "matched",
            RingStatus::Expired => "expired",
        };

        sqlx::query(
            r#"
            INSERT INTO rings (id, token, host_sdp, guest_sdp, host_pubkey, guest_pubkey, status, created_at, expires_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            "#,
        )
        .bind(ring.id())
        .bind(ring.token().value())
        .bind(ring.host_sdp().value())
        .bind(ring.guest_sdp().map(|s| s.value()))
        .bind(ring.host_pubkey().value())
        .bind(ring.guest_pubkey().map(|p| p.value()))
        .bind(status)
        .bind(ring.created_at())
        .bind(ring.expires_at())
        .execute(&self.pool)
        .await
        .map_err(|e| InfrastructureError::QueryExecution(format!("Failed to save ring: {}", e)))?;

        Ok(())
    }

    async fn find_by_token(&self, token: &Token) -> Result<Option<Ring>> {
        let query = format!("SELECT {} FROM rings WHERE token = $1", SELECT_COLUMNS);
        let row = sqlx::query(&query)
            .bind(token.value())
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| {
                InfrastructureError::QueryExecution(format!("Failed to find ring by token: {}", e))
            })?;

        match row {
            Some(ref r) => Ok(Some(Self::row_to_ring(r)?)),
            None => Ok(None),
        }
    }

    async fn find_open_rings(&self) -> Result<Vec<Ring>> {
        let query = format!(
            "SELECT {} FROM rings WHERE status = 'open' ORDER BY created_at DESC",
            SELECT_COLUMNS
        );
        let rows = sqlx::query(&query)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| {
                InfrastructureError::QueryExecution(format!("Failed to find open rings: {}", e))
            })?;

        let mut rings = Vec::new();
        for row in &rows {
            rings.push(Self::row_to_ring(row)?);
        }
        Ok(rings)
    }

    async fn update(&self, ring: &Ring) -> Result<()> {
        let status = match ring.status() {
            RingStatus::Open => "open",
            RingStatus::Matched => "matched",
            RingStatus::Expired => "expired",
        };

        let result = sqlx::query(
            r#"
            UPDATE rings
            SET guest_sdp = $1, guest_pubkey = $2, status = $3
            WHERE id = $4
            "#,
        )
        .bind(ring.guest_sdp().map(|s| s.value()))
        .bind(ring.guest_pubkey().map(|p| p.value()))
        .bind(status)
        .bind(ring.id())
        .execute(&self.pool)
        .await
        .map_err(|e| {
            InfrastructureError::QueryExecution(format!("Failed to update ring: {}", e))
        })?;

        if result.rows_affected() == 0 {
            return Err(InfrastructureError::QueryExecution(format!(
                "Ring not found for update: {}",
                ring.id()
            ))
            .into());
        }

        Ok(())
    }

    async fn find_by_id(&self, id: &Uuid) -> Result<Option<Ring>> {
        let query = format!("SELECT {} FROM rings WHERE id = $1", SELECT_COLUMNS);
        let row = sqlx::query(&query)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| {
                InfrastructureError::QueryExecution(format!("Failed to find ring by id: {}", e))
            })?;

        match row {
            Some(ref r) => Ok(Some(Self::row_to_ring(r)?)),
            None => Ok(None),
        }
    }
}
