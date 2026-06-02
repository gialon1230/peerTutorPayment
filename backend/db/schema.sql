PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS users (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    address TEXT NOT NULL UNIQUE,
    role TEXT NOT NULL CHECK(role IN ('student','tutor','admin')),
    name TEXT
);

CREATE TABLE IF NOT EXISTS sessions (
    id INTEGER PRIMARY KEY,
    student_id INTEGER NOT NULL,
    tutor_id INTEGER NOT NULL,
    amount INTEGER NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('requested','locked','student_confirmed','tutor_confirmed','confirmed','paid','cancelled')),
    contract_session_id INTEGER,
    token_contract TEXT,
    chain_tx_hash TEXT,
    chain_status TEXT NOT NULL DEFAULT 'pending' CHECK(chain_status IN ('pending','submitted','confirmed','failed','skipped')),
    created_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
    updated_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
    FOREIGN KEY(student_id) REFERENCES users(id),
    FOREIGN KEY(tutor_id) REFERENCES users(id)
);

CREATE INDEX IF NOT EXISTS idx_sessions_status ON sessions(status);
CREATE INDEX IF NOT EXISTS idx_sessions_student ON sessions(student_id);
CREATE INDEX IF NOT EXISTS idx_sessions_tutor ON sessions(tutor_id);

CREATE TABLE IF NOT EXISTS settlements (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id INTEGER NOT NULL,
    contract_session_id INTEGER,
    method TEXT NOT NULL DEFAULT 'release',
    tx_hash TEXT,
    status TEXT NOT NULL CHECK(status IN ('pending','succeeded','failed')),
    payload_json TEXT,
    created_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
    FOREIGN KEY(session_id) REFERENCES sessions(id)
);

CREATE INDEX IF NOT EXISTS idx_settlements_session ON settlements(session_id);

CREATE TABLE IF NOT EXISTS session_confirmations (
    session_id INTEGER NOT NULL,
    confirmed_by TEXT NOT NULL CHECK(confirmed_by IN ('student','tutor','admin')),
    source TEXT NOT NULL DEFAULT 'app' CHECK(source IN ('app','chain','manual')),
    confirmed_at INTEGER NOT NULL DEFAULT (strftime('%s','now')),
    PRIMARY KEY(session_id, confirmed_by),
    FOREIGN KEY(session_id) REFERENCES sessions(id)
);

CREATE INDEX IF NOT EXISTS idx_confirmations_session ON session_confirmations(session_id);
