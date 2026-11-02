# Serein Connect fork

The user explicitly requested an independent, self-hosted Rust communications app,
using Serein as the starting point and Cindy as the visual reference. This overrides
the upstream product restrictions against a backend/account system. Keep the upstream
source intact during migration. The user approved Tauri + Rust + web UI on 2026-11-01.
The independent `connect` workspace supplies Rust server/networking; `desktop` is
the Tauri client and `web` its local HTML/CSS/JS interface. The former egui client
remains as a prototype/reference, not the primary launcher. Keep fonts, assets and
licenses. Discord transports are not used by Connect.

This is a local ZIP-based project with no authorized upstream remote. Do not publish
or send changes to the original project's maintainers. Check this workspace with
`cargo fmt --check`, `cargo test`, `cargo clippy --all-targets -- -D warnings`, and
`cargo build --bins`. Upstream xtask does not package this new workspace.

Do not pretend unimplemented voice, screen sharing or remote control works. Use real
bounded networking and explicit consent for future media/input capture. Test synthetic
peers, never Discord accounts. Keep private identity keys in OS credential storage.

