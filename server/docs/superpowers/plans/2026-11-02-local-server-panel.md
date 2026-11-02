# Local server launcher implementation plan

> **For agentic workers:** Use superpowers:executing-plans to implement this plan.

**Goal:** Double-click Starta Server.bat, configure a new server in the browser and administer it without a Connect client.
**Architecture:** A loopback-only manager survives server stop/start. An OS-protected local launch credential opens a cookie/CSRF session. Explicit in-process operator authority shares domain logic without impersonating or resetting any Connect member. Existing remote approval remains unchanged.
**Tech Stack:** Rust, Axum, SQLite, existing server-ui JavaScript, PowerShell launcher.
**Spec:** User request in this conversation, 2026-11-02.

## Constraints and review focus
- Preserve server data, identity, owner/admin roles; never automatically use --pair.
- No local operator authority may be supplied through serialized network requests.
- Check exact Origin/Host, bound sessions/bodies; only bind manager to loopback.
- Setup is exclusive, cannot overwrite existing or partial data; errors remain visible in UI.
- Reopening launcher reuses manager; stop/start retain server identity and settings.
- No secrets/user data in distributed packages. No reconstructed client build.

## Tasks
- [x] Tests for unauthorized/foreign-origin access, initial setup, operator channel creation, owner preservation, stop/start and duplicate setup rejection.
- [x] Internal authority type and local lifecycle manager with cookie authentication.
- [x] Browser setup/lifecycle/settings plus reuse existing channel/member/invite panel; backup controls using existing encrypted backup implementation.
- [x] BAT launcher opens panel automatically; update guide and packaging whitelist.
- [x] Rust tests, fmt, strict Clippy, server builds; browser end-to-end wizard/restart; source/runtime backups and delivery.

## Ledger
Ruling: use a separate loopback manager port (4749, ephemeral fallback), so existing deployments can coexist until upgraded. Remote admin still uses run and its existing approval flow.
Ruling: do not silently transfer address-book namespaces. Invitations work independently; existing signed registrations are preserved. Registering a profile-owned namespace still requires its owner signature.

Validation: full no-default-features Rust suite, strict Clippy, fmt check and all server bins passed. Browser wizard, repeated login, channels, invites, stop/start, access settings, encrypted backup and restore passed with preserved identity and channels. Independent review findings fixed: per-instance cookies and background jobs retaining lifecycle lock. Package verification recorded separately in delivery.json.
