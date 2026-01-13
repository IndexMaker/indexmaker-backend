use sea_orm::{Database, DatabaseConnection, DbErr};
use std::env;

/// Set up test database connection
/// Uses TEST_DATABASE_URL environment variable or falls back to default
pub async fn setup_test_db() -> Result<DatabaseConnection, DbErr> {
    let database_url = env::var("TEST_DATABASE_URL").unwrap_or_else(|_| {
        "postgresql://indexmaker_user@localhost:5432/indexmaker_test".to_string()
    });

    Database::connect(&database_url).await
}

/// Optional cleanup function for test database
/// For this story, no cleanup needed as we don't write to DB
#[allow(dead_code)]
pub async fn cleanup_test_db(_db: &DatabaseConnection) -> Result<(), DbErr> {
    // No cleanup needed for read-only operations
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_setup_test_db() {
        let db = setup_test_db().await;
        assert!(db.is_ok(), "Test database connection should succeed");
    }
}
