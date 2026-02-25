-- Migration: 002_create_candles.sql

CREATE TABLE IF NOT EXISTS candles (
    ticker     TEXT     NOT NULL,
    timestamp  DATETIME NOT NULL,
    open       REAL     NOT NULL,
    high       REAL     NOT NULL,
    low        REAL     NOT NULL,
    close      REAL     NOT NULL,
    volume     INTEGER  NOT NULL,
    PRIMARY KEY (ticker, timestamp)
);

CREATE INDEX IF NOT EXISTS idx_candles_ticker_ts ON candles (ticker, timestamp);
