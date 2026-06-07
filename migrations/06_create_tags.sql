CREATE TABLE IF NOT EXISTS tags
(
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    name       TEXT     NOT NULL COLLATE NOCASE UNIQUE,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS stock_tags
(
    ticker     TEXT     NOT NULL,
    tag_id     INTEGER  NOT NULL,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (ticker, tag_id),
    FOREIGN KEY (tag_id) REFERENCES tags (id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_stock_tags_tag_id ON stock_tags (tag_id);

CREATE TABLE IF NOT EXISTS tag_categories
(
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    name       TEXT     NOT NULL COLLATE NOCASE UNIQUE,
    sort_order INTEGER  NOT NULL DEFAULT 0,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

ALTER TABLE tags ADD COLUMN category_id INTEGER REFERENCES tag_categories (id);

CREATE INDEX IF NOT EXISTS idx_tags_category_id ON tags (category_id);

CREATE TABLE IF NOT EXISTS tag_suggestions
(
    ticker             TEXT     NOT NULL PRIMARY KEY,
    status             TEXT     NOT NULL CHECK (status IN ('pending', 'ready', 'failed', 'ignored')),
    suggested_tags     JSON     NOT NULL DEFAULT '[]' CHECK (json_valid(suggested_tags)),
    error              TEXT,
    profile_fetched_at DATETIME NOT NULL,
    generated_at       DATETIME,
    requested_at       DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    provider           TEXT     NOT NULL,
    model              TEXT     NOT NULL
);
