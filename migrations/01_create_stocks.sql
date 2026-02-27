CREATE TABLE IF NOT EXISTS stocks
(
    ticker        TEXT NOT NULL PRIMARY KEY,
    exchange      TEXT NOT NULL,
    sector_name   TEXT NOT NULL,
    sector_url    TEXT NOT NULL,
    industry_name TEXT NOT NULL,
    industry_url  TEXT NOT NULL,
    last_update   DATE NOT NULL
);
