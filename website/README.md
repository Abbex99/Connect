# Connect website

Public repository: https://github.com/Abbex99/Connect
Website: https://connect-web-zeta.vercel.app

Swedish Connect beta website, made with local Inter fonts and the supplied Orbitom UI kit's visual language.

Run `npm run dev` and open http://127.0.0.1:4780. No dependency install is required. Vercel serves the `public` folder. The website has no account system, database, directory, or relay backend yet.

For browser verification install Playwright (`npm install --no-save playwright`, `npx playwright install chromium`) and run `npm test` while the dev server is running. Download tests also require the real release ZIPs in `public/downloads`. `TEST_URL` can target a deployment, and `PLAYWRIGHT_MODULE` / `CHROMIUM_PATH` can select preinstalled test tools.

## Release files

ZIP files under `public/downloads` are deliberately excluded from Git. They are curated from preserved release artifacts, with filenames and SHA-256 hashes in `releases.json` and `SHA256SUMS.txt`. The Windows client is build `ebea803d2b6bbd49`, not a new client build. Server packages are Windows `13d69e2741a8642d` and Linux `c21e96f85dcea38d`.

Never replace these artifacts with a working user's profile or server directory. Never publish `data`, `data.panel`, credentials, invite tokens, or .env files.

## Source status

The website source is complete. The standalone server source snapshot is provided separately. The original desktop sources were deleted; recovered desktop fragments are not a complete buildable representation of the downloadable client. This limitation must remain visible until resolved. Do not advertise reproducible client builds or fabricated performance multipliers.

## Licenses

Original website code: MIT, see LICENSE. Inter font: OFL. Lucide icons: ISC. Included notices are in `public/assets`. Connect logo is the user-supplied project brand asset. Downloaded Connect/server packages carry their own licenses.
