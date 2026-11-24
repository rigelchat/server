<div align="center">
  <img src="https://avatars.githubusercontent.com/u/247460033?s=200&v=4" alt="Rigel Server" width="100" height="100"/>

  # Rigel Server

  A lightweight, self-hostable chat server written in Rust, compatible with **Fluxer**, **Spacebar**, and **Discord** clients.

  [![Release](https://img.shields.io/github/v/release/rigelchat/server?style=for-the-badge&logo=github)](https://github.com/rigelchat/server/releases/latest)
  [![License](https://img.shields.io/badge/License-AGPLv3-blue.svg?style=for-the-badge)](LICENSE)
  [![Rust](https://img.shields.io/badge/Rust-000000?style=for-the-badge&logo=rust&logoColor=white)](https://www.rust-lang.org)
</div>

## Introduction

Rigel is an independent, minimalist chat backend built with Rust, Axum, and MySQL. Designed to stay fast and easy to self-host, it avoids unnecessary platform bloat while remaining compatible across multiple client ecosystems through a shared core and a lightweight protocol adapter.

## Prerequisites

* **MySQL** (`>= 8.0`) *(recommended; uses `VIRTUAL` generated columns for indexed snowflake and case-insensitive lookups)* or **MariaDB** (`>= 10.5`)
* *(From source only)* **Rust** (latest stable, edition 2024)

## Installation & Setup

### 1. Create the Database

```sql
CREATE DATABASE rigel CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;
```

### 2. Configure Environment

```bash
cp .env.example .env
```

Edit `.env` with your `DATABASE_URL`, a strong `AUTH_SECRET` (at least 32 characters, never the example placeholder), and your public URLs (`PUBLIC_HTTP_URL`, `PUBLIC_WS_URL`).

> **Note:** On a fresh instance, the first registered non-bot account automatically receives staff privileges to bootstrap initial communities.

### 3. Run the Server

#### Option A: Pre-built Binary (Recommended)

1. Download the archive matching your OS and architecture from the [**Latest Releases**](https://github.com/rigelchat/server/releases/latest).
2. Extract the archive next to your `.env` in a writable working directory (used for `static/emoji/` for automatic Twemoji SVG downloads).
3. Start the server:

```bash
# Linux / macOS
chmod +x rigel-server
./rigel-server

# Windows
.\rigel-server.exe
```

#### Option B: Building from Source

```bash
git clone https://github.com/rigelchat/server.git
cd server
cp .env.example .env
# Edit .env with your DATABASE_URL, AUTH_SECRET, and public URLs
cargo run --release
```

## Endpoints

By default, Rigel listens on `127.0.0.1:3000`:

| Service | Endpoint |
| --- | --- |
| **REST API** | `http://localhost:3000/api` (`/api/v9`) |
| **WebSocket Gateway** | `ws://localhost:3000/gateway` |
| **Discovery** | `/.well-known/fluxer` · `/.well-known/spacebar/client` |
| **Static Assets** | `http://localhost:3000/static` |

---

<div align="center">
  <sub>Built with 💙 by the <a href="https://github.com/rigelchat">Rigel Team</a>.</sub>
</div>