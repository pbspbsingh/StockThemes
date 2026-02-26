CREATE TABLE IF NOT EXISTS performance
(
    ticker       TEXT     NOT NULL PRIMARY KEY,
    perf_1m      REAL     NOT NULL,
    perf_3m      REAL     NOT NULL,
    perf_6m      REAL     NOT NULL,
    perf_1y      REAL     NOT NULL,
    extra_info   JSON     NOT NULL DEFAULT '{}',
    last_updated DATETIME NOT NULL
);
