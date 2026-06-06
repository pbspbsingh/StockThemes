use chrono::Local;

use crate::store::Store;

use super::{CachedTagSuggestion, SuggestionInput, SuggestionStatus};

impl Store {
    pub async fn get_tag_suggestion(
        &self,
        ticker: &str,
    ) -> sqlx::Result<Option<CachedTagSuggestion>> {
        let ticker = ticker.trim().to_uppercase();
        let row = sqlx::query!(
            r#"
            SELECT
                ticker as "ticker!: String",
                status,
                suggested_tags as "suggested_tags!: String",
                error,
                generated_at as "generated_at?: chrono::DateTime<Local>",
                requested_at as "requested_at: chrono::DateTime<Local>",
                provider,
                model
            FROM tag_suggestions
            WHERE ticker = $1
            "#,
            ticker,
        )
        .fetch_optional(&self.pool)
        .await?;

        row.map(|row| {
            cached_suggestion_from_row(CachedSuggestionRow {
                ticker: row.ticker,
                status: row.status,
                suggested_tags: row.suggested_tags,
                error: row.error,
                generated_at: row.generated_at,
                requested_at: row.requested_at,
                provider: row.provider,
                model: row.model,
            })
        })
        .transpose()
    }

    pub async fn save_pending_tag_suggestion_request(
        &self,
        ticker: &str,
        provider: &str,
        model: &str,
    ) -> sqlx::Result<()> {
        let ticker = ticker.trim().to_uppercase();
        let requested_at = Local::now();
        sqlx::query!(
            r#"
            INSERT INTO tag_suggestions
                (ticker, status, suggested_tags, error, profile_fetched_at, generated_at, requested_at, provider, model)
            VALUES
                ($1, 'pending', '[]', NULL, $2, NULL, $3, $4, $5)
            ON CONFLICT(ticker) DO UPDATE SET
                status = excluded.status,
                suggested_tags = excluded.suggested_tags,
                error = excluded.error,
                profile_fetched_at = excluded.profile_fetched_at,
                generated_at = excluded.generated_at,
                requested_at = excluded.requested_at,
                provider = excluded.provider,
                model = excluded.model
            "#,
            ticker,
            requested_at,
            requested_at,
            provider,
            model,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn update_pending_tag_suggestion_profile(
        &self,
        ticker: &str,
        input: &SuggestionInput,
    ) -> sqlx::Result<bool> {
        let ticker = ticker.trim().to_uppercase();
        let result = sqlx::query!(
            r#"
            UPDATE tag_suggestions
            SET profile_fetched_at = $1
            WHERE ticker = $2
              AND status = 'pending'
            "#,
            input.profile.fetched_at,
            ticker,
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn save_ready_tag_suggestion(
        &self,
        ticker: &str,
        suggested_tags: &[String],
    ) -> sqlx::Result<bool> {
        let ticker = ticker.trim().to_uppercase();
        let suggested_tags =
            serde_json::to_string(suggested_tags).unwrap_or_else(|_| "[]".to_string());
        let generated_at = Local::now();
        let result = sqlx::query!(
            r#"
            UPDATE tag_suggestions
            SET status = 'ready',
                suggested_tags = $1,
                error = NULL,
                generated_at = $2
            WHERE ticker = $3
              AND status = 'pending'
            "#,
            suggested_tags,
            generated_at,
            ticker,
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn save_failed_tag_suggestion(
        &self,
        ticker: &str,
        error: &str,
    ) -> sqlx::Result<bool> {
        let ticker = ticker.trim().to_uppercase();
        let generated_at = Local::now();
        let result = sqlx::query!(
            r#"
            UPDATE tag_suggestions
            SET status = 'failed',
                error = $1,
                generated_at = $2
            WHERE ticker = $3
              AND status = 'pending'
            "#,
            error,
            generated_at,
            ticker,
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn delete_tag_suggestion(&self, ticker: &str) -> sqlx::Result<()> {
        let ticker = ticker.trim().to_uppercase();
        sqlx::query!(
            r#"
            DELETE FROM tag_suggestions
            WHERE ticker = $1
            "#,
            ticker,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

struct CachedSuggestionRow {
    ticker: String,
    status: String,
    suggested_tags: String,
    error: Option<String>,
    generated_at: Option<chrono::DateTime<Local>>,
    requested_at: chrono::DateTime<Local>,
    provider: String,
    model: String,
}

fn cached_suggestion_from_row(row: CachedSuggestionRow) -> sqlx::Result<CachedTagSuggestion> {
    let status_raw = row.status;
    let status = match status_raw.as_str() {
        "pending" => SuggestionStatus::Pending,
        "ready" => SuggestionStatus::Ready,
        "failed" => SuggestionStatus::Failed,
        _ => SuggestionStatus::Failed,
    };
    let suggested_tags =
        serde_json::from_str::<Vec<String>>(&row.suggested_tags).unwrap_or_default();
    Ok(CachedTagSuggestion {
        ticker: row.ticker,
        status,
        suggested_tags,
        error: row.error,
        generated_at: row.generated_at,
        requested_at: row.requested_at,
        provider: row.provider,
        model: row.model,
    })
}
