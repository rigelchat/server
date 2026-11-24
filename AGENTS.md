# Rigel Server — Project Instructions

Minimalist, self-hosted chat backend in Rust (`edition = 2024`, Axum 0.8, SQLx 0.8 MySQL/MariaDB, Tokio, WebSocket Gateway).

## Architecture (1 file = 1 responsibility)

- `src/routes/` — HTTP handlers using extractors (`AuthUser`, `AppJson` in `src/extractors.rs`, `AppError` in `src/error.rs`). **Zero SQL.**
- `src/services/` — Business logic and typed SQL (`sqlx::query_as`), split by domain (`guild/`, `channel/`, `role/`, `user/`, plus focused single-file services and `queries.rs` for shared SQL). **Zero HTTP `StatusCode`** (use semantic `AppError::*` constructors from `src/error.rs`).
- `src/models/` — Typed request/response/DB structs. Avoid raw `serde_json::Value` / `json!()` blobs where a struct fits.
- `src/db/` — Pool setup (`mod.rs`) and idempotent schema/index management (`schema.rs`).
- `src/gateway/` — WebSocket lifecycle, rate limiting, and 1 handler file per opcode under `src/gateway/handlers/`.
- `src/rate_limit.rs` — Bounded in-memory HTTP rate limiter middleware.
- `src/constants/` — Domain constants (`flags`, `gateway`, `guild`, `limits`, `user`).
- `src/utils/` — Shared utilities (`assets`, `snowflake`, `token`, `validation`).
- `src/config.rs` — Deployment & instance policy loaded from `.env` (protocol limits live in `src/constants/limits.rs`).

## Coding Standards

- Logs, error messages, and code comments in **English only**.
- Minimalist comments (only explain non-obvious invariants).
- **No `.unwrap()` or `.expect()`** outside `#[cfg(test)]` modules in `routes/`, `services/`, `extractors.rs`, `gateway/`, `utils/`, `models/`, or `rate_limit.rs`.
- Stay **minimalist & lightweight**: no voice/WebRTC, billing, quests, or protobuf bloat.
- Verify compilation with `cargo check` / `cargo clippy --all-targets -- -D warnings` after every code change.