# Connect

[Website and beta downloads](https://mailhantering.se) · [Server source](server) · [Website and directory source](website)

Connect is a free communication app for chatting, voice, screen sharing and self-hosted communities. This repository contains the verified standalone server source and the website.

## Source availability

- `server/`: recovered and tested Rust standalone server, transport, directory and server management panel. Build with `cargo build --locked --no-default-features --bin connect-server` from this folder.
- `website/`: Swedish landing page and signed PostgreSQL-backed address book. Run `npm ci`; see its README for database setup and deployment.
- `client-launcher/`: portable launcher connecting the preserved client to the online directory, preserving custom endpoints and backing up preferences.
- Desktop client: the preserved beta binary exists, but its complete buildable source was lost and is being recovered. **This repository cannot reproduce the desktop client binary.** We do not claim otherwise.

Remote control, annotation on screen sharing and friends are planned, not implemented features of this release. No published comparative RAM/speed benchmark is available.

Servers publish their current address automatically and can share a readable address such as `mailhantering.se/s/spelkvall`, with an optional password. The web page supplies a compatible connection through a copy button; native web-link activation is not claimed for the preserved client. Server names and personal profile namespaces are separate. See `server/packaging/server/ADRESSBOK.md` for upgrade instructions that preserve existing server data.

The standalone server keeps profiles, membership and room data in its own runtime data directory. Never commit live server data, invite tokens, local profiles or credentials.

## Website deployment and release assets

Release ZIPs are intentionally not committed. The current verified deployment uploads curated ZIPs directly to Vercel. Do not enable Git-only auto deployment until release assets are supplied through GitHub Releases or another real artifact store and site links are updated. Otherwise downloads will 404. Filenames and hashes are under `website/public/downloads`.

## Licenses

Original website: MIT (`website/LICENSE`). Server retains its original MIT/Apache-2.0 and dependency notices. Logos are product brand assets; font and icon licenses are included.
