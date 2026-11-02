# Connect shared directory — implemented design and verification ledger

User authorized implementation, deployment, GitHub publication, and Neon Free terms. Final steering: TeamSpeak-style readable server address and optional password; no random code in the normal shared link.

## Delivered design
- Vercel Node API + Neon Free Frankfurt, preserving signed v1/v2 clients. New v3 location signatures use their own domain and never claim a personal namespace.
- `/s/<chosen name>` resolves a server-owned name to a fresh, signed server address. Optional password unlocks a random admission key locally with PBKDF2-SHA256/AES-GCM; the actual Rust server enforces admission, including returning peers. Non-owner control requests require a currently authorized chat session when password mode is enabled. Owner recovery remains separate.
- Names bind to server keys. Duplicate names are rejected and a server does not advertise the link until publication succeeds. Stale locations are reclaimed after their accepted lifetime. Persistent names are bounded to 10,000 globally, eight per server and 20 new claims per IP bucket/day. Existing ownership is never silently reassigned.
- Old direct invites remain available under advanced controls. Website supplies the client-compatible connection with a copy button. The long technical value is collapsed by default.
- Portable launcher changes only the old localhost default, backs up preferences atomically and preserves custom HTTPS endpoints. Current running installations are not replaced or restarted.

## Verification completed
- Node signature tests include actual Rust v1/v2/v3 vectors, forged signature/expiry/size rejection and cross-protocol replay rejection.
- Real PostgreSQL tests: concurrent first owner claim, replay, independent location records, expiry cleanup preserving names, duplicate readable-name rejection.
- Deployed HTTPS API/browser: publication/lookup, malformed requests, expired server state, 390px layout, readable password URL, wrong/right password, Rust-to-WebCrypto interoperability, hidden technical value, no password/token request URLs.
- Rust integration: actual live publication and private relay-only connection passed. Password tests prove wrong/missing passwords, returning-member enforcement, password rotation, control-channel enforcement and preservation of administrator role on authorized invite rejoin.
- Full Rust tests, strict clippy, fmt and Windows builds; Linux x64 release cross-build. The new Linux binary has not been rerun on the user's CasaOS installation.

## Review fixes and rulings
- First fresh review: v3/v1 replay was reproduced RED and fixed GREEN with a distinct signature domain. Expired location capacity was reproduced RED and fixed GREEN with reclamation and separated budgets. Corrected the manager endpoint and composed publication status messages.
- User changed the invitation requirement, so the same reviewer assessed the new password/readable-name scope. Fixed control-channel bypass (RED→GREEN), admin-role demotion, advertising names before ownership confirmation, and permanent unbounded name growth. Added actual join/rotation and collision tests.
- Ruling: preserve the working client binary instead of rebuilding incomplete recovered desktop sources. Cost: one copy/paste step remains; do not claim native deep-link activation or a reproducible client build.
- Ruling: permanent name ownership is protected within explicit service quotas, rather than reassigned when a server goes offline. Cost: lost server keys need operator recovery/cleanup and large-scale abuse monitoring remains an operational responsibility.
- Review did not independently run deployments, preserved binary internals, or final packages. Parent verifies deployment/package content separately. No claim of testing the friend's particular network or current CasaOS runtime.

## Deployment and packaging
Domain DNS and HTTPS verified; service at https://mailhantering.se and https://connect-web-zeta.vercel.app. Neon resource connect-directory, free_v3, Frankfurt. Credentials are server-only environment variables and never included in source/package archives.
Release manifests record actual bytes and SHA-256; existing user's server data and profiles are preserved. Final source is published at https://github.com/Abbex99/Connect.
