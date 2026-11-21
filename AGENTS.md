# Rigel Server — Project Instructions

Minimalist, self-hosted chat backend in Rust (`edition = 2024`, Axum 0.8, SQLx 0.8 MySQL/MariaDB, Tokio, WebSocket Gateway).

## Architecture (1 file = 1 responsibility)

- `src/routes/` — HTTP handlers and extractors (`AuthUser`, `ClientDialect`, `AppError`). **Zero SQL.**
- `src/services/` — Business logic and typed SQL (`sqlx::query_as`). **Zero HTTP `StatusCode`** (use semantic `AppError::*` constructors from `src/error.rs`).
- `src/models/` — Typed request/response/DB structs. Avoid raw `serde_json::Value` / `json!()` blobs where a struct fits.
- `src/gateway/` — WebSocket lifecycle + 1 handler file per opcode under `src/gateway/handlers/`.
- `src/constants/` — Domain constants (`dialect`, `flags`, `gateway`, `guild`, `limits`, `user`).
- `src/config.rs` — Deployment & instance policy loaded from `.env` (protocol limits live in `src/constants/limits.rs`).

## Coding Standards

- Logs, error messages, and code comments in **English only**.
- Minimalist comments (only explain non-obvious invariants).
- **No `.unwrap()`** in `routes/`, `services/`, `extractors.rs`, or `gateway/`.
- Stay **minimalist & lightweight**: no voice/WebRTC, billing, quests, or protobuf bloat.
- Verify compilation with `cargo check` after every code change.