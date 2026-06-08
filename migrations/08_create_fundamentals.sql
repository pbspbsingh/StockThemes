CREATE TABLE IF NOT EXISTS fundamentals
(
    exchange     TEXT     NOT NULL,
    ticker       TEXT     NOT NULL,
    payload      JSON     NOT NULL CHECK (json_valid(payload)),
    last_updated DATETIME NOT NULL,
    PRIMARY KEY (exchange, ticker)
);
