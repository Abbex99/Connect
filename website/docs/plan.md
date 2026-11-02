# Connect website implementation plan

Goal: A polished Swedish downloadable Connect beta website, prepared for mailhantering.se and independently testable on Vercel. User explicitly said to go ahead and test.
Architecture: static HTML, local CSS/JS and fonts; downloads are real curated ZIP artifacts. No database required for this website stage. Existing Connect binaries and server are untouched.
Scope: landing page, interactive illustrative app preview, free features vs roadmap, FAQ, accessible download dialog, dark/light theme, real downloads, standalone source archive, Vercel deployment.
Out of scope: central directory deployment, relaying, desktop reconstruction, domain transfer, automated updates.
Brand: Connect logo supplied by user; use visual language from UI kit, not its English-only documentation instruction. Swedish copy.
Claims: no invented performance benchmarks, user counts, or fabricated repositories. 1080p free. OBS marked beta. Remote desktop and annotation marked planned. Source availability accurately scoped to supplied source artifacts.
Tasks:
- [ ] Curate actual client/server releases and checksums without user data.
- [ ] Build responsive landing page and real download flows.
- [ ] Test mobile/desktop, keyboard, themes, links and download hashes.
- [ ] Fresh review, fix findings, deploy website and validate live download responses.
- [ ] Attach mailhantering.se to new Vercel project; give exact required Strato DNS if no authenticated Strato access.

Ruling: Independent static project folder, no existing git repository/worktree, no upstream publication.
Ruling: User's 'kor ... testar' authorizes implementation and test deployment of the website; no additional design gate.
