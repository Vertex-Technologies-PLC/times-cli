# Changelog

## 0.4.2

- Credentials live in `~/.config/times/credentials.json` (mode 0600). macOS Keychain no longer prompts on every command. Existing Keychain sessions are copied to that file on first use.
- `times aip doctor` reports auth from the stored session, not from whether an org is already selected.
- Browser CLI sessions last 10 years and slide forward on refresh, matching GitHub `gh` OAuth tokens. Run `times auth logout` to end them. Set `TIMES_CREDENTIAL_STORE=keyring` to keep using the OS keychain.

## 0.4.1

- `times upgrade` always asks GitHub for the latest release. A stale local cache can no longer claim you are current.
- Other commands print a cached newer-version hint immediately, then refresh GitHub in the background (hourly while current, daily after an update is known). Commands are not blocked on the network.

## 0.4.0

- `times aip push` runs `package.json` `build` when the TIMES-hosted output folder is missing. Pass `--no-build` in CI that already built.
- `times.aip.json` can set `hosting.provider` to `times` (zip to TIMES) or `developer` (public HTTPS origin). Old manifests still infer from `ui.entry`.
- Push is fail-closed: local scan, then a server re-scan. Findings block the upload. Human review is still required.
- Origin checks refuse private, loopback, link-local, and metadata IPs, including DNS that resolves there. HTTPS on port 443 only. Redirects fail closed.
- Guest zip scan rejects remote HTML tags, `javascript:` URLs, inline event handlers, PE/ELF/Mach-O/Wasm binaries, and extra secret prefixes.
- Source `eval(` / `new Function` / `innerHTML` matching ignores comments and string literals.
- New commands: `times aip scan`, `times aip origin check`, `times aip doctor`, `times aip listing get|set`, `times aip delete`, `times aip open`.
