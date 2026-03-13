use crate::domain::entities::Ring;
use crate::domain::value_objects::Token;
use anyhow::Result;
use async_trait::async_trait;
use uuid::Uuid;

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait RingRepository: Send + Sync {
    async fn save(&self, ring: &Ring) -> Result<()>;
    async fn find_by_token(&self, token: &Token) -> Result<Option<Ring>>;
    async fn find_open_rings(&self) -> Result<Vec<Ring>>;
    async fn update(&self, ring: &Ring) -> Result<()>;
    async fn find_by_id(&self, id: &Uuid) -> Result<Option<Ring>>;
}
