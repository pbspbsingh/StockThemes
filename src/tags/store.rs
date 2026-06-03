use chrono::Local;
use sqlx::Sqlite;
use std::collections::HashMap;

use crate::store::{DeleteTagResult, StockTags, Store, Tag, TagCategory};

#[derive(Debug, Clone, Default)]
pub struct AddTagsResult {
    pub created_tags: Vec<String>,
    pub added_tags: Vec<String>,
    pub duplicates_skipped: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ReplaceTagsResult {
    pub created_tags: Vec<String>,
    pub set_tags: Vec<String>,
    pub removed_tags: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ReplaceImportResult {
    pub created_tags: Vec<String>,
    pub mappings_set: usize,
    pub mappings_removed: usize,
}

impl Store {
    pub async fn list_tag_categories(&self) -> sqlx::Result<Vec<TagCategory>> {
        sqlx::query_as::<_, TagCategory>(
            r#"
            SELECT
                c.id,
                c.name,
                c.sort_order,
                COUNT(DISTINCT st.ticker) as stock_count
            FROM tag_categories c
            LEFT JOIN tags t ON t.category_id = c.id
            LEFT JOIN stock_tags st ON st.tag_id = t.id
            GROUP BY c.id, c.name, c.sort_order
            ORDER BY stock_count DESC, c.sort_order, lower(c.name)
            "#,
        )
        .fetch_all(&self.pool)
        .await
    }

    pub async fn category_exists(&self, category_id: i64) -> sqlx::Result<bool> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tag_categories WHERE id = $1")
            .bind(category_id)
            .fetch_one(&self.pool)
            .await?;
        Ok(count > 0)
    }

    pub async fn create_tag_category(&self, name: &str) -> sqlx::Result<TagCategory> {
        let name = normalize_tag_name(name);
        let sort_order: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(sort_order), 0) + 10 FROM tag_categories WHERE sort_order < 999",
        )
        .fetch_one(&self.pool)
        .await?;

        sqlx::query(
            r#"
            INSERT INTO tag_categories (name, sort_order, created_at, updated_at)
            VALUES ($1, $2, $3, $3)
            ON CONFLICT(name) DO UPDATE SET updated_at = tag_categories.updated_at
            "#,
        )
        .bind(&name)
        .bind(sort_order)
        .bind(Local::now())
        .execute(&self.pool)
        .await?;

        self.get_tag_category_by_name(&name).await
    }

    pub async fn list_tags(&self) -> sqlx::Result<Vec<Tag>> {
        sqlx::query_as::<_, Tag>(
            r#"
            SELECT
                t.id,
                t.name,
                t.category_id,
                COUNT(st.ticker) as stock_count
            FROM tags t
            LEFT JOIN stock_tags st ON st.tag_id = t.id
            GROUP BY t.id, t.name, t.category_id
            ORDER BY t.category_id, stock_count DESC, lower(t.name)
            "#,
        )
        .fetch_all(&self.pool)
        .await
    }

    pub async fn create_tag(&self, name: &str) -> sqlx::Result<Tag> {
        self.create_tag_in_category(name, None).await
    }

    pub async fn create_tag_in_category(
        &self,
        name: &str,
        category_id: Option<i64>,
    ) -> sqlx::Result<Tag> {
        let name = normalize_tag_name(name);
        sqlx::query(
            r#"
            INSERT INTO tags (name, category_id, created_at, updated_at)
            VALUES ($1, $2, $3, $3)
            ON CONFLICT(name) DO UPDATE SET updated_at = tags.updated_at
            "#,
        )
        .bind(&name)
        .bind(category_id)
        .bind(Local::now())
        .execute(&self.pool)
        .await?;

        self.get_tag_by_name(&name).await
    }

    pub async fn rename_tag(&self, id: i64, name: &str) -> sqlx::Result<Tag> {
        let name = normalize_tag_name(name);
        sqlx::query("UPDATE tags SET name = $1, updated_at = $2 WHERE id = $3")
            .bind(&name)
            .bind(Local::now())
            .bind(id)
            .execute(&self.pool)
            .await?;

        self.get_tag_by_id(id).await
    }

    pub async fn move_tag_to_category(&self, id: i64, category_id: i64) -> sqlx::Result<Tag> {
        sqlx::query("UPDATE tags SET category_id = $1, updated_at = $2 WHERE id = $3")
            .bind(category_id)
            .bind(Local::now())
            .bind(id)
            .execute(&self.pool)
            .await?;

        self.get_tag_by_id(id).await
    }

    pub async fn delete_tag(&self, id: i64) -> sqlx::Result<DeleteTagResult> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM stock_tags WHERE tag_id = $1")
            .bind(id)
            .fetch_one(&self.pool)
            .await?;
        if count > 0 {
            return Ok(DeleteTagResult::InUse(count));
        }

        sqlx::query("DELETE FROM tags WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(DeleteTagResult::Deleted)
    }

    pub async fn list_stock_tags(&self) -> sqlx::Result<Vec<StockTags>> {
        #[derive(sqlx::FromRow)]
        struct Row {
            ticker: String,
            tag_id: Option<i64>,
            tag_name: Option<String>,
            tag_category_id: Option<i64>,
        }

        let rows = sqlx::query_as::<_, Row>(
            r#"
            WITH all_tickers AS (
                SELECT ticker FROM stocks
                UNION
                SELECT ticker FROM stock_tags
            )
            SELECT
                all_tickers.ticker,
                tags.id as tag_id,
                tags.name as tag_name,
                tags.category_id as tag_category_id
            FROM all_tickers
            LEFT JOIN stock_tags st ON st.ticker = all_tickers.ticker
            LEFT JOIN tags ON tags.id = st.tag_id
            ORDER BY all_tickers.ticker, lower(tags.name)
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        let mut stocks = Vec::<StockTags>::new();
        let mut index_by_ticker = HashMap::<String, usize>::new();
        for row in rows {
            let idx = match index_by_ticker.get(&row.ticker) {
                Some(idx) => *idx,
                None => {
                    let idx = stocks.len();
                    index_by_ticker.insert(row.ticker.clone(), idx);
                    stocks.push(StockTags {
                        ticker: row.ticker.clone(),
                        tags: Vec::new(),
                    });
                    idx
                }
            };
            if let (Some(id), Some(name), Some(category_id)) =
                (row.tag_id, row.tag_name, row.tag_category_id)
            {
                stocks[idx].tags.push(Tag {
                    id,
                    name,
                    category_id,
                    stock_count: 0,
                });
            }
        }

        Ok(stocks)
    }

    pub async fn list_untagged_stocks(&self) -> sqlx::Result<Vec<String>> {
        sqlx::query_scalar(
            r#"
            SELECT ticker
            FROM stocks
            WHERE ticker NOT IN (SELECT ticker FROM stock_tags)
            ORDER BY ticker
            "#,
        )
        .fetch_all(&self.pool)
        .await
    }

    pub async fn add_tags_to_stock(
        &self,
        ticker: &str,
        tags: &[String],
    ) -> sqlx::Result<AddTagsResult> {
        let ticker = ticker.trim().to_uppercase();
        let tags = normalize_tag_names(tags);
        let mut result = AddTagsResult::default();
        let mut tx = self.pool.begin().await?;

        for tag in tags {
            let insert_tag = sqlx::query(
                r#"
                INSERT OR IGNORE INTO tags (name, created_at, updated_at)
                VALUES ($1, $2, $2)
                "#,
            )
            .bind(&tag)
            .bind(Local::now())
            .execute(&mut *tx)
            .await?;
            if insert_tag.rows_affected() > 0 {
                result.created_tags.push(tag.clone());
            }

            let tag_id: i64 = sqlx::query_scalar("SELECT id FROM tags WHERE name = $1")
                .bind(&tag)
                .fetch_one(&mut *tx)
                .await?;

            let insert_mapping = sqlx::query(
                r#"
                INSERT OR IGNORE INTO stock_tags (ticker, tag_id, created_at)
                VALUES ($1, $2, $3)
                "#,
            )
            .bind(&ticker)
            .bind(tag_id)
            .bind(Local::now())
            .execute(&mut *tx)
            .await?;
            if insert_mapping.rows_affected() > 0 {
                result.added_tags.push(tag);
            } else {
                result.duplicates_skipped.push(tag);
            }
        }

        tx.commit().await?;
        Ok(result)
    }

    pub async fn replace_tags_for_stocks(
        &self,
        assignments: &[(String, Vec<String>)],
    ) -> sqlx::Result<ReplaceImportResult> {
        let mut tx = self.pool.begin().await?;
        let mut result = ReplaceImportResult::default();

        for (ticker, tags) in assignments {
            let row_result = Self::replace_tags_for_stock_in_tx(&mut tx, ticker, tags).await?;
            for tag in row_result.created_tags {
                if !result
                    .created_tags
                    .iter()
                    .any(|existing| existing.eq_ignore_ascii_case(&tag))
                {
                    result.created_tags.push(tag);
                }
            }
            result.mappings_set += row_result.set_tags.len();
            result.mappings_removed += row_result.removed_tags.len();
        }

        result.created_tags.sort_by_key(|tag| tag.to_lowercase());
        tx.commit().await?;
        Ok(result)
    }

    async fn replace_tags_for_stock_in_tx(
        tx: &mut sqlx::Transaction<'_, Sqlite>,
        ticker: &str,
        tags: &[String],
    ) -> sqlx::Result<ReplaceTagsResult> {
        let ticker = ticker.trim().to_uppercase();
        let tags = normalize_tag_names(tags);
        let mut result = ReplaceTagsResult::default();

        #[derive(sqlx::FromRow)]
        struct ExistingTagRow {
            tag_id: i64,
            tag_name: String,
        }

        let existing_rows = sqlx::query_as::<_, ExistingTagRow>(
            r#"
            SELECT
                tags.id as tag_id,
                tags.name as tag_name
            FROM stock_tags
            JOIN tags ON tags.id = stock_tags.tag_id
            WHERE stock_tags.ticker = $1
            ORDER BY lower(tags.name)
            "#,
        )
        .bind(&ticker)
        .fetch_all(&mut **tx)
        .await?;

        let existing_by_name = existing_rows
            .iter()
            .map(|row| (row.tag_name.to_lowercase(), row.tag_id))
            .collect::<HashMap<_, _>>();
        let desired_by_name = tags
            .iter()
            .map(|tag| (tag.to_lowercase(), tag))
            .collect::<HashMap<_, _>>();

        for row in &existing_rows {
            if !desired_by_name.contains_key(&row.tag_name.to_lowercase()) {
                sqlx::query("DELETE FROM stock_tags WHERE ticker = $1 AND tag_id = $2")
                    .bind(&ticker)
                    .bind(row.tag_id)
                    .execute(&mut **tx)
                    .await?;
                result.removed_tags.push(row.tag_name.clone());
            }
        }

        for tag in tags {
            if existing_by_name.contains_key(&tag.to_lowercase()) {
                result.set_tags.push(tag);
                continue;
            }

            let tag_id: i64 = sqlx::query_scalar("SELECT id FROM tags WHERE name = $1")
                .bind(&tag)
                .fetch_one(&mut **tx)
                .await?;

            sqlx::query(
                r#"
                INSERT INTO stock_tags (ticker, tag_id, created_at)
                VALUES ($1, $2, $3)
                ON CONFLICT(ticker, tag_id) DO NOTHING
                "#,
            )
            .bind(&ticker)
            .bind(tag_id)
            .bind(Local::now())
            .execute(&mut **tx)
            .await?;
            result.set_tags.push(tag);
        }

        Ok(result)
    }

    pub async fn set_tags_for_stock(
        &self,
        ticker: &str,
        tags: &[String],
    ) -> sqlx::Result<ReplaceTagsResult> {
        let mut tx = self.pool.begin().await?;
        let result = Self::replace_tags_for_stock_in_tx(&mut tx, ticker, tags).await?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn remove_tag_from_stock(&self, ticker: &str, tag_id: i64) -> sqlx::Result<()> {
        let ticker = ticker.trim().to_uppercase();
        sqlx::query("DELETE FROM stock_tags WHERE ticker = $1 AND tag_id = $2")
            .bind(ticker)
            .bind(tag_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn get_tag_by_id(&self, id: i64) -> sqlx::Result<Tag> {
        sqlx::query_as::<_, Tag>(
            r#"
            SELECT
                t.id,
                t.name,
                t.category_id,
                COUNT(st.ticker) as stock_count
            FROM tags t
            LEFT JOIN stock_tags st ON st.tag_id = t.id
            WHERE t.id = $1
            GROUP BY t.id, t.name, t.category_id
            "#,
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await
    }

    async fn get_tag_by_name(&self, name: &str) -> sqlx::Result<Tag> {
        sqlx::query_as::<_, Tag>(
            r#"
            SELECT
                t.id,
                t.name,
                t.category_id,
                COUNT(st.ticker) as stock_count
            FROM tags t
            LEFT JOIN stock_tags st ON st.tag_id = t.id
            WHERE t.name = $1
            GROUP BY t.id, t.name, t.category_id
            "#,
        )
        .bind(name)
        .fetch_one(&self.pool)
        .await
    }

    async fn get_tag_category_by_name(&self, name: &str) -> sqlx::Result<TagCategory> {
        sqlx::query_as::<_, TagCategory>(
            r#"
            SELECT
                c.id,
                c.name,
                c.sort_order,
                COUNT(DISTINCT st.ticker) as stock_count
            FROM tag_categories c
            LEFT JOIN tags t ON t.category_id = c.id
            LEFT JOIN stock_tags st ON st.tag_id = t.id
            WHERE c.name = $1
            GROUP BY c.id, c.name, c.sort_order
            "#,
        )
        .bind(name)
        .fetch_one(&self.pool)
        .await
    }
}

fn normalize_tag_name(name: &str) -> String {
    name.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn normalize_tag_names(tags: &[String]) -> Vec<String> {
    tags.iter()
        .map(|s| normalize_tag_name(s))
        .filter(|s| !s.is_empty())
        .fold(Vec::new(), |mut acc, tag| {
            if !acc
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(&tag))
            {
                acc.push(tag);
            }
            acc
        })
}
