mod schema;

use sqlx::mysql::{MySqlPool, MySqlPoolOptions};
use tracing::info;

pub async fn init(database_url: &str, max_connections: u32) -> Result<MySqlPool, sqlx::Error> {
    let pool = MySqlPoolOptions::new()
        .max_connections(max_connections.max(1))
        .connect(database_url)
        .await?;

    info!("Database connected successfully");
    schema::apply(&pool).await?;

    Ok(pool)
}
