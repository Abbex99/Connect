# Connect Server Linux/CasaOS implementation plan

> **For agentic workers:** Use superpowers:executing-plans, then a fresh read-only review.

**Goal:** Deliver a separate Intel/AMD Linux server package for CasaOS, preserving installed Windows artifacts.
**Architecture:** Build a native Linux x86_64 executable. Package it as a local Docker image with persistent data and a Caddy HTTPS frontend. The manager accepts explicit HTTPS Origin/Host and an OS-protected persistent operator key. Keep the local Windows launch path unchanged.
**Tech Stack:** Rust/Axum, musl cross compilation, Docker Compose, Caddy, POSIX shell.
**Spec:** User requested a separate Linux version for CasaOS and confirmed Intel/AMD.

## Constraints and review focus
- No registry publication or external account required; installer builds the small image locally from the bundled executable.
- Only HTTPS proxy port published. Backend inaccessible directly from LAN. Exact origin, HttpOnly/Secure cookies, CSRF, bounded failed logins and request bodies.
- Persistent state survives container recreation. Installation never replaces data or an existing operator key silently.
- No Connect login required for server setup. Administration key is separate from member/owner identity.
- Preserve default Windows manager behavior and existing member permissions.
- Verify Linux compilation and attempt isolated Linux runtime tests; disclose remaining CasaOS host testing limitations.

## Tasks
- [x] Add failing tests for remote origin validation, Secure sessions, wrong-origin/anonymous denial, login throttling and member authority preservation.
- [x] Implement explicit remote manager mode, persistent-key loading and browser key entry.
- [x] Cross-build Linux x86_64; resolve target-specific issues without changing client profile behavior.
- [x] Docker/CasaOS compose, HTTPS proxy, install/start/stop scripts, persistent volumes and Swedish guide.
- [x] Rust/static checks, browser remote-mode test, Linux smoke where possible, independent review, separate distribution/source backup.

## Ledger
User opted into the Docker recommendation by requesting the separate Linux package; no redundant design approval needed.
Windows has no WSL/Docker runtime. Use isolated portable cross tools, not system-level WSL installation.

Validation: Linux x86_64-musl release built; native Alpine Linux and Docker 28.3.3 runtime tested in isolated QEMU. Real HTTPS proxy/browser setup, authentication, channels/invites, restart, backup/restore and installer recreation passed. 93 tests passed, 5 ignored; final manager tests and strict Clippy passed. CasaOS UI on physical host remains untested. Runtime review fixes: Caddy NET_BIND_SERVICE executable capability, default SNI for IP clients, explicit GET healthcheck, forced recreation to reload proxy config.
