# WishSync Tauri shell

Build manually (no before*Command): `cd apps/web && npm run build:native`, then `cargo tauri dev|build` from this directory.
Custom commands: session_token_get / session_token_set / session_token_clear (OS keychain, service tw.wishsync.app, user session).
Android has no native keyring backend wired up (commands will error there); iOS uses Keychain.
