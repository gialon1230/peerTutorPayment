# Backend (MVP)

This folder contains a minimal SQLite schema and a tiny demonstration script showing how to initialize the DB and run a simulated booking/confirmation flow.

Quick start

```bash
python backend/main.py
```

Next steps

- Wire the backend to a Soroban RPC client to submit transactions when both confirmations exist.
- Persist transaction hashes in `settlements` and implement polling/reconciliation logic.
- Implement API endpoints and authentication for users to create sessions and confirm completion.
