"""SQLite-backed tutoring backend with optional Soroban CLI wiring.

This script keeps the app state in SQLite and, when configured, submits the
corresponding session actions to the Soroban tutoring contract using the
`stellar` CLI.
"""
from __future__ import annotations

import argparse
import os
import sqlite3
import subprocess
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable, Optional


BASE_DIR = Path(__file__).resolve().parent
DB_PATH = Path(os.environ.get("DATABASE_URL", BASE_DIR / "db.sqlite3"))
SCHEMA_PATH = BASE_DIR / "db" / "schema.sql"


@dataclass(frozen=True)
class ChainConfig:
    contract_id: str
    source_account: str
    network: str


def open_db() -> sqlite3.Connection:
    conn = sqlite3.connect(DB_PATH)
    conn.row_factory = sqlite3.Row
    conn.execute("PRAGMA foreign_keys = ON")
    return conn


def init_db() -> None:
    with open(SCHEMA_PATH, "r", encoding="utf-8") as schema_file, open_db() as conn:
        conn.executescript(schema_file.read())
    print(f"initialized database at {DB_PATH}")


def get_chain_config() -> Optional[ChainConfig]:
    contract_id = os.environ.get("CONTRACT_ID")
    source_account = os.environ.get("CONTRACT_SOURCE")
    network = os.environ.get("CONTRACT_NETWORK", "testnet")
    if not contract_id or not source_account:
        return None
    return ChainConfig(contract_id=contract_id, source_account=source_account, network=network)


def ensure_user(conn: sqlite3.Connection, address: str, role: str, name: str) -> int:
    conn.execute(
        "INSERT OR IGNORE INTO users(address, role, name) VALUES(?, ?, ?)",
        (address, role, name),
    )
    row = conn.execute("SELECT id FROM users WHERE address = ?", (address,)).fetchone()
    if row is None:
        raise RuntimeError(f"failed to create or load user: {address}")
    return int(row["id"])


def create_session(
    conn: sqlite3.Connection,
    session_id: int,
    student_id: int,
    tutor_id: int,
    amount: int,
    token_contract: str,
) -> None:
    conn.execute(
        """
        INSERT OR REPLACE INTO sessions(
            id, student_id, tutor_id, amount, status, token_contract, chain_status
        ) VALUES(?, ?, ?, ?, 'locked', ?, 'pending')
        """,
        (session_id, student_id, tutor_id, amount, token_contract),
    )
    conn.execute(
        "INSERT OR REPLACE INTO settlements(session_id, contract_session_id, method, status) VALUES(?, ?, 'release', 'pending')",
        (session_id, session_id),
    )
    conn.commit()


def record_confirmation(conn: sqlite3.Connection, session_id: int, confirmed_by: str, source: str = "app") -> None:
    conn.execute(
        "INSERT OR REPLACE INTO session_confirmations(session_id, confirmed_by, source) VALUES(?, ?, ?)",
        (session_id, confirmed_by, source),
    )
    if confirmed_by == "student":
        conn.execute("UPDATE sessions SET status = 'student_confirmed', updated_at = strftime('%s','now') WHERE id = ?", (session_id,))
    elif confirmed_by == "tutor":
        conn.execute("UPDATE sessions SET status = 'tutor_confirmed', updated_at = strftime('%s','now') WHERE id = ?", (session_id,))
    elif confirmed_by == "admin":
        conn.execute("UPDATE sessions SET status = 'confirmed', updated_at = strftime('%s','now') WHERE id = ?", (session_id,))
    conn.commit()


def invoke_contract(chain: ChainConfig, method: str, args: Iterable[str]) -> subprocess.CompletedProcess[str]:
    command = [
        "stellar",
        "contract",
        "invoke",
        "--id",
        chain.contract_id,
        "--source-account",
        chain.source_account,
        "--network",
        chain.network,
        "--",
        method,
    ]
    command.extend(args)
    return subprocess.run(command, check=True, capture_output=True, text=True)


def sync_session_to_chain(conn: sqlite3.Connection, session_id: int, chain: Optional[ChainConfig]) -> None:
    session = conn.execute(
        """
        SELECT s.*, u_student.address AS student_address, u_tutor.address AS tutor_address
        FROM sessions s
        JOIN users u_student ON u_student.id = s.student_id
        JOIN users u_tutor ON u_tutor.id = s.tutor_id
        WHERE s.id = ?
        """,
        (session_id,),
    ).fetchone()
    if session is None:
        raise RuntimeError(f"session {session_id} not found")

    if chain is None:
        print("chain config missing; database updated only")
        return

    try:
        if session["chain_status"] == "pending":
            create_args = [
                "--session_id",
                str(session_id),
                "--student",
                session["student_address"],
                "--tutor",
                session["tutor_address"],
                "--amount",
                str(session["amount"]),
            ]
            result = invoke_contract(chain, "create_session", create_args)
            conn.execute(
                "UPDATE sessions SET chain_status = 'submitted', chain_tx_hash = ?, updated_at = strftime('%s','now') WHERE id = ?",
                (result.stdout.strip() or None, session_id),
            )

        confirmations = conn.execute(
            "SELECT confirmed_by FROM session_confirmations WHERE session_id = ?",
            (session_id,),
        ).fetchall()
        confirmed_by = {row["confirmed_by"] for row in confirmations}

        if "student" in confirmed_by:
            invoke_contract(chain, "confirm_by_student", ["--session_id", str(session_id), "--student", session["student_address"]])
        if "tutor" in confirmed_by:
            invoke_contract(chain, "confirm_by_tutor", ["--session_id", str(session_id), "--tutor", session["tutor_address"]])

        conn.execute(
            "UPDATE settlements SET status = 'succeeded', tx_hash = COALESCE(tx_hash, ?), payload_json = ? WHERE session_id = ?",
            (session["chain_tx_hash"], '{"synced": true}', session_id),
        )
        conn.execute(
            "UPDATE sessions SET status = 'paid', chain_status = 'confirmed', updated_at = strftime('%s','now') WHERE id = ?",
            (session_id,),
        )
        conn.commit()
        print(f"session {session_id} synced and marked paid")
    except subprocess.CalledProcessError as exc:
        conn.execute(
            "UPDATE settlements SET status = 'failed', payload_json = ? WHERE session_id = ?",
            (exc.stderr or exc.stdout or str(exc), session_id),
        )
        conn.execute(
            "UPDATE sessions SET chain_status = 'failed', updated_at = strftime('%s','now') WHERE id = ?",
            (session_id,),
        )
        conn.commit()
        print("chain sync failed; check the CLI output")


def demo_flow() -> None:
    chain = get_chain_config()
    with open_db() as conn:
        student_id = ensure_user(conn, "GABC_STUDENT", "student", "Alice")
        tutor_id = ensure_user(conn, "GABC_TUTOR", "tutor", "Bob")
        create_session(conn, 1, student_id, tutor_id, 100, os.environ.get("TOKEN_CONTRACT", "CAMPUS_TOKEN_ADDRESS"))
        record_confirmation(conn, 1, "student")
        record_confirmation(conn, 1, "tutor")
        sync_session_to_chain(conn, 1, chain)


def main() -> None:
    parser = argparse.ArgumentParser(description="Peer tutoring payment backend")
    parser.add_argument("command", choices=["init-db", "demo", "sync-session"])
    parser.add_argument("--session-id", type=int, default=1)
    args = parser.parse_args()

    if args.command == "init-db":
        init_db()
        return

    init_db()
    if args.command == "demo":
        demo_flow()
        return

    with open_db() as conn:
        sync_session_to_chain(conn, args.session_id, get_chain_config())


if __name__ == "__main__":
    main()
