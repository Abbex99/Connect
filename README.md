# Connect

Connect is a free communication app for chatting, voice, screen sharing and self-hosted communities. This repository contains the verified standalone server source and the website.

## Source availability

- `server/`: recovered and tested Rust standalone server, transport, directory and server management panel. Build with `cargo build --locked --no-default-features --bin connect-server` from this folder.
- `website/`: complete static Swedish landing page. Run `npm run dev` in this folder.
- Desktop client: the preserved beta binary exists, but its complete buildable source was lost and is being recovered. **This repository cannot reproduce the desktop client binary.** We do not claim otherwise.

Remote control, annotation on screen sharing and friends are planned, not implemented features of this release. No published comparative RAM/speed benchmark is available.

The standalone server keeps profiles, membership and room data in its own runtime data directory. Never commit live server data, invite tokens, local profiles or credentials.

## Website deployment and release assets

Release ZIPs are intentionally not committed. The current verified deployment uploads curated ZIPs directly to Vercel. Do not enable Git-only auto deployment until release assets are supplied through GitHub Releases or another real artifact store and site links are updated. Otherwise downloads will 404. Filenames and hashes are under `website/public/downloads`.

## Licenses

Original website: MIT (`website/LICENSE`). Server retains its original MIT/Apache-2.0 and dependency notices. Logos are product brand assets; font and icon licenses are included.
