CREATE TABLE IF NOT EXISTS company_profiles
(
    ticker     TEXT     NOT NULL PRIMARY KEY,
    summary    TEXT,
    sector     TEXT,
    industry   TEXT,
    source     TEXT     NOT NULL DEFAULT 'Yahoo Finance',
    fetched_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);
