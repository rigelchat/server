# Installation & Configuration Guide

This guide covers deploying **Rigel Server** from pre-built binaries or source, configuring your database and environment variables, and placing the server behind a reverse proxy.

---

## 1. Prerequisites

* **MySQL** `>= 8.0` or **MariaDB** `>= 10.5`
* *(From source only)* **Rust** (latest stable, edition 2024)
* *(Recommended for production)* A reverse proxy with TLS termination (**Nginx**, **Caddy**, etc.)

---

## 2. Database Setup

Create a dedicated `utf8mb4` database and user:

```sql
CREATE DATABASE rigel CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;
CREATE USER 'rigel'@'localhost' IDENTIFIED BY 'strong_database_password';
GRANT ALL PRIVILEGES ON rigel.* TO 'rigel'@'localhost';
FLUSH PRIVILEGES;
```

> Schema tables and idempotent column migrations are applied automatically on startup.

---

## 3. Installing the Server

### Option A: Pre-built Binary (Recommended)

1. Download the release archive for your OS/architecture from [GitHub Releases](https://github.com/rigelchat/server/releases/latest).
2. Extract it into your deployment directory (e.g. `/opt/rigel`).
3. Create `.env` from `.env.example` and edit it:
   ```bash
   cp .env.example .env
   ```
4. Start the binary:
   ```bash
   # Linux / macOS
   chmod +x rigel-server
   ./rigel-server

   # Windows
   .\rigel-server.exe
   ```

### Option B: Building from Source

```bash
git clone https://github.com/rigelchat/server.git
cd server
cp .env.example .env
# Edit .env, then build and run:
cargo build --release
./target/release/rigel-server
```

---

## 4. Environment Configuration (`.env`)

Generate a strong `AUTH_SECRET` before starting in production:

```bash
openssl rand -base64 32
```

### Network & Logging

| Variable | Required | Default | Description |
| --- | --- | --- | --- |
| `HOST` | No | `127.0.0.1` | IP address the HTTP/WebSocket server binds to. Use `0.0.0.0` if exposed directly or across containers. |
| `PORT` | No | `3000` | TCP port to listen on. |
| `PUBLIC_HTTP_URL` | **Yes** | — | Public base URL without trailing slash (e.g. `https://chat.example.com`). Used in discovery endpoints. |
| `PUBLIC_WS_URL` | **Yes** | — | Public WebSocket base URL without trailing slash (e.g. `wss://chat.example.com`). |
| `CORS_ALLOWED_ORIGINS` | No | `*` | Comma-separated list of allowed origins (e.g. `https://chat.example.com,https://app.example.com`), or `*` to allow all. |
| `RUST_LOG` | No | `info` | Log filter (`error`, `warn`, `info`, `debug`, `trace`). |

### Database

| Variable | Required | Default | Description |
| --- | --- | --- | --- |
| `DATABASE_URL` | **Yes** | — | MySQL/MariaDB connection string (`mysql://user:password@host:3306/rigel`). |
| `DB_MAX_CONNECTIONS` | No | `5` | Maximum number of pooled database connections. |

### Authentication

| Variable | Required | Default | Values / Description |
| --- | --- | --- | --- |
| `AUTH_SECRET` | **Yes** | — | Secret key used to sign session tokens (HMAC-SHA256). Rotating it invalidates all existing sessions. |
| `AUTH_LOGIN_MODE` | No | `username` | `username` (email-less accounts) or `email` (requires and stores unique email addresses). |
| `AUTH_TAG_STYLE` | No | `none` | `none` (fixed `0000` discriminator) or `random` (assigns a random 4-digit tag on registration). |
| `AUTH_REGISTRATION_MODE` | No | `open` | `open` (anyone can register), `invite_only` (requires a valid `invite_code`), or `closed` (disables registration). |

### Instance Branding & Legal

| Variable | Required | Default | Description |
| --- | --- | --- | --- |
| `INSTANCE_NAME` | No | `Rigel` | Public display name of your instance. |
| `INSTANCE_DESCRIPTION` | No | — | Short instance description returned by `/api/ping`. |
| `INSTANCE_IMAGE` | No | — | Public URL to the instance logo/icon. |
| `INSTANCE_COLOR` | No | — | Brand accent color in `#rrggbb` format. |
| `INSTANCE_COUNTRY_CODE` | No | `FR` | Two-letter ISO 3166-1 alpha-2 country code sent in Gateway `READY`. |
| `INSTANCE_CONTACT_EMAIL` | No | — | Public operator contact email returned by `/api/ping`. |
| `INSTANCE_OWNER_ID` | No | — | User ID of the instance operator returned by `/api/ping`. |
| `INSTANCE_WEBSITE_URL` | No | — | Public website / marketing URL. |
| `INSTANCE_TERMS_URL` | No | — | Terms of Service URL. |
| `INSTANCE_PRIVACY_URL` | No | — | Privacy Policy URL. |
| `INSTANCE_GUIDELINES_URL` | No | — | Community Guidelines URL. |
| `INSTANCE_WEBAPP_URL` | No | — | Hosted web client URL. |

### Features

| Variable | Required | Default | Values / Description |
| --- | --- | --- | --- |
| `GUILD_CREATION_POLICY` | No | `staff` | `staff` (only users with the `STAFF` flag can create guilds) or `everyone` (any registered user can create guilds). |
| `ENABLE_ATTACHMENTS` | No | `false` | Advertises attachment upload capability in discovery metadata. |
| `ENABLE_AUTO_EMOJI_SYNC` | No | `true` | Automatically downloads Twemoji SVG assets into `static/emoji/` in the background on startup if missing. |

---

## 5. Reverse Proxy Setup

Make sure `PUBLIC_HTTP_URL` uses `https://` and `PUBLIC_WS_URL` uses `wss://` when behind TLS.

### Caddy

```caddyfile
chat.example.com {
    reverse_proxy 127.0.0.1:3000
}
```

### Nginx

```nginx
server {
    listen 443 ssl http2;
    server_name chat.example.com;

    # ssl_certificate / ssl_certificate_key ...

    location / {
        proxy_pass http://127.0.0.1:3000;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;

        # Required for WebSocket (/gateway)
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_read_timeout 86400s;
    }
}
```

---

## 6. First-Time Bootstrap

1. Start the server and register your first account via your client (or `POST /api/auth/register`).
2. The first non-bot account registered automatically receives the `STAFF` flag, allowing it to create the initial guilds when `GUILD_CREATION_POLICY="staff"`.
3. On first boot (with `ENABLE_AUTO_EMOJI_SYNC=true`), Twemoji SVG icons are downloaded automatically in the background into `static/emoji/` and served under `/static/emoji/`.