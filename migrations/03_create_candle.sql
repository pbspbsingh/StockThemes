CREATE TABLE IF NOT EXISTS daily_candles
(
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    ticker       TEXT     NOT NULL,
    ds           DATETIME NOT NULL,
    open         REAL     NOT NULL,
    high         REAL     NOT NULL,
    low          REAL     NOT NULL,
    close        REAL     NOT NULL,
    volume       INTEGER  NOT NULL,
    last_updated DATETIME NOT NULL,
    UNIQUE (ticker, ds)
);

