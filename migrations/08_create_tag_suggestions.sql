CREATE TABLE IF NOT EXISTS tag_suggestions
(
    ticker         TEXT     NOT NULL PRIMARY KEY,
    status         TEXT     NOT NULL CHECK (status IN ('pending', 'ready', 'failed')),
    suggested_tags JSON     NOT NULL DEFAULT '[]' CHECK (json_valid(suggested_tags)),
    error          TEXT,
    profile_fetched_at DATETIME NOT NULL,
    generated_at   DATETIME,
    requested_at   DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    provider       TEXT     NOT NULL,
    model          TEXT     NOT NULL,
    prompt_hash    TEXT     NOT NULL
);
