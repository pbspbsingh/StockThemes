-- Migration: 001_create_stocks.sql
-- Creates the unified stocks table with a `source` column to differentiate
-- between TradingView (`tv`) and Yahoo Finance (`yf`) data sources.

CREATE TABLE IF NOT EXISTS stocks (
    source        TEXT NOT NULL,
    ticker        TEXT NOT NULL,
    exchange      TEXT NOT NULL,
    sector_name   TEXT NOT NULL,
    sector_url    TEXT NOT NULL,
    industry_name TEXT NOT NULL,
    industry_url  TEXT NOT NULL,
    last_update   DATE NOT NULL,

    PRIMARY KEY (source, ticker)
);

CREATE INDEX IF NOT EXISTS idx_stocks_source ON stocks (source);
CREATE INDEX IF NOT EXISTS idx_stocks_last_update ON stocks (last_update);
