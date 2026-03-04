CREATE TABLE IF NOT EXISTS daily_candles
(
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    ticker       TEXT     NOT NULL,
    day          DATE     NOT NULL,
    open         REAL     NOT NULL,
    high         REAL     NOT NULL,
    low          REAL     NOT NULL,
    close        REAL     NOT NULL,
    adj_close    REAL,
    volume       INTEGER  NOT NULL,
    last_updated DATETIME NOT NULL,
    UNIQUE (ticker, day)
);

