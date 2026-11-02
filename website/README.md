# Connect website

Public repository: https://github.com/Abbex99/Connect
Website and directory: https://mailhantering.se (deployment alias: https://connect-web-zeta.vercel.app)

Swedish Connect beta website, made with local Inter fonts and the supplied Orbitom UI kit's visual language.

Run `npm ci`, then `npm run dev` for the static pages at http://127.0.0.1:4780. Vercel serves `public` and the Node directory function. Signed v1/v2 registrations support existing clients; v3 records locate standalone servers by public identity without claiming a personal username. PostgreSQL persists registrations; stale addresses disappear from lookup after 180 seconds. App chat, files and voice are not stored in this directory.

Provision a PostgreSQL database and set `DATABASE_URL` as a server-only environment variable. The hosted installation uses Neon Free in Frankfurt. Apply `node --env-file=.env.local scripts/migrate-directory.mjs` once before deployment. Do not commit or distribute `.env` files. `node --test tests/directory.test.mjs` verifies Rust-compatible signatures. `node --env-file=.env.local --test tests/directory-db.test.mjs` also tests atomic ownership, replay and expiry against a database using disposable synthetic records. `tests/directory-live.mjs` tests the deployed API and invitation page with Playwright. `vercel deploy --prod` publishes the configured project; download ZIPs must already be staged.

Shareable addresses use `/s/<chosen name>`. New servers are open by default, with an optional password. The server publishes signed access settings; passwords unlock an AES-GCM admission key locally in the browser using PBKDF2-SHA256 (600,000 rounds). The actual server checks the key on join, including returning members, and requires a current authorized chat session for non-owner control requests in password mode. Owner recovery remains exempt. Password changes invalidate previous password grants. Explicit separate invites remain usable. Legacy `/server/<public key>#<invitation key>` links are retained as an advanced fallback. The page produces a compatible `connect://join/…` invitation behind a copy button. Native URL activation is not implemented in the preserved client. A web link does not bypass network filtering: connections still use Iroh direct transport with external relay fallback.

To self-host later, move this Node service and its PostgreSQL database. Change `CONNECT_DIRECTORY_URL` on the server, or its address-book setting in the local panel while stopped. Empty configuration disables automatic registration. Existing saved custom directory addresses in clients are preserved; default localhost clients migrate via the portable launcher with an automatic preferences backup. Domain changes need old links redirected to the new domain, preserving paths and fragments.

For browser verification install Playwright (`npm install --no-save playwright`, `npx playwright install chromium`) and run `npm test` while the dev server is running. Download tests also require the real release ZIPs in `public/downloads`. `TEST_URL` can target a deployment, and `PLAYWRIGHT_MODULE` / `CHROMIUM_PATH` can select preinstalled test tools.

## Release files

ZIP files under `public/downloads` are deliberately excluded from Git. Filenames, exact sizes and SHA-256 hashes are in `releases.json` and `SHA256SUMS.txt`. The Windows client executable is preserved build `ebea803d2b6bbd49`, with a new launcher connecting it to the shared directory. Server packages are rebuilt from the supplied Rust source.

Never replace these artifacts with a working user's profile or server directory. Never publish `data`, `data.panel`, credentials, invite tokens, or .env files.

## Source status

The website source is complete. The standalone server source snapshot is provided separately. The original desktop sources were deleted; recovered desktop fragments are not a complete buildable representation of the downloadable client. This limitation must remain visible until resolved. Do not advertise reproducible client builds or fabricated performance multipliers.

## Licenses

Original website code: MIT, see LICENSE. Inter font: OFL. Lucide icons: ISC. Included notices are in `public/assets`. Connect logo is the user-supplied project brand asset. Downloaded Connect/server packages carry their own licenses.
