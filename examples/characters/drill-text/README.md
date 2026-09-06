# Character package extension drill

`manifest.json` is an importable asset-free package using the existing `text`
adapter. It declares its own character ID, display name and bounded presentation
preference. No adapter registry, core crate or normal product index is changed.

From a checkout with the normal Rust/Tauri and desktop dependencies installed:

```bash
node scripts/drills/character-package.mjs
```

The script records source SHA, dirty-tree status, commands, return codes and
elapsed time. It runs the production host `character_store::import` and `remove`
ports in a temporary app home, then applies the plain-text fallback through the
runtime Character hello bridge and persists preferences with the host writer.
It checks the package disappears, bundled removal is refused, the removal alone
does not rewrite prefs/session/audit, and explicit fallback plus runtime restart
preserves other preferences, audit history and session identity/epoch/truth.
Temporary app data is destroyed when the native test finishes.

The TypeScript checks use the normal manifest validator, adapter factory and text
renderer. The page test exercises the two-click removal flow and fallback prefs
write with a mocked host, then remounts the page. This is separate evidence from
the native production-port test; it does not claim a complete native UI loop.
The native host does not generate a package-remove audit event; existing Character
hello audit history is what this drill preserves and verifies.

For an interactive native walkthrough, import this manifest through the character
library (no assets required), select it, then remove it using the confirmation
button. The page chooses its configured default character after active removal;
when rendering fails the existing trusted text fallback remains available. That
UI walkthrough must be recorded separately from these process-local tests.
