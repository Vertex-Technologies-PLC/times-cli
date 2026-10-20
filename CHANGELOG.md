# Changelog

## 0.4.12

- `times upgrade` installs quietly and no longer asks you to source shellenv or open a new terminal. Shell config is still written automatically.
- Fresh install one-liner: `curl … | . /dev/stdin`. The script applies shellenv in that shell (PATH + builtin disable).

## 0.4.11

- `times aip deploy --bump <major|minor|patch>` bumps `times.aip.json` (and `package.json` when present) before packing and staging. Same as `times manifest bump`, then deploy.
- `times manifest bump` also syncs `package.json` version when that file exists.
- First CLI run and `times upgrade` write the shellenv hook that disables the POSIX `times` builtin. cargo-dist install success text points at `~/.local/bin/times setup-shell` so the builtin cannot block the fix.

## 0.4.10

- Local deploy and `times aip scan --remote` upload source through `POST /api/v1/depot/source-upload` (gateway → depot → MinIO). Fixes Cloudflare 522 from public MinIO presign URLs.
- Upload progress bar, retries on edge 521–524, and job follow with streamed build logs stay the default.

## 0.4.9

- `times aip deploy` and `times aip scan --remote` follow depot jobs by default (use `--no-follow` to exit after queue). Progress steps, upload bar, status labels, and streamed build logs print while the job runs.
- Source upload retries on Cloudflare edge failures (HTTP 521–524) and surfaces a clear MinIO / `--remote` hint.
- `times init` fetches the matching framework folder from `times-aip-templates` (vite, static, next), then writes a fresh manifest and catalogs. Falls back to the built-in scaffold offline.
- Local `times aip scan` prints numbered steps while it runs.

## 0.4.8

- `times aip scan --remote` uploads source and runs an authoritative depot `scan_only` job (`--follow` polls until done).
- `times aip retry <job-id>` rebuilds a failed, cancelled, or timed-out depot job.
- Default developer PAT create includes `depot:read`, `depot:deploy`, and `depot:repos`.

## 0.4.7

- `times aip deploy` zips the project and stages via depot, or queues a remote build with `--remote <github_url>`.
- Pass `--follow` to stream job status, and `--submit` after a successful deploy.

## 0.4.6

- Manifest validate allows `lender:read`, `lender:write`, and `lender:admin` for TIMES Lender apps.

## 0.4.5

- Theme validation requires guest apps to load user preferences (`createTimesAipClient` prefs sync, `api.preferences`, or `/api/v1/apps/runtime/v1/preferences`) so locale, palette, and color mode match TIMES.
- Built CSS/JS/HTML must include all TIMES palettes from `themes.css` and a preferences fetch path. Missing palettes or prefs sync fails validation.
- Color mode checks accept light, dark, and system (`prefers-color-scheme` / `THEME_BOOT_SCRIPT`).

## 0.4.4

- Every command prints a newer-version warning after its own output, including `times help` and `times --version`. The warning names the new version and `times upgrade`.
- If the saved check is stale, the command asks GitHub before that warning. A fresh check does not wait. `times upgrade`, shell completions, JSON output, and `TIMES_NO_UPDATE_CHECK=1` stay quiet.

## 0.4.3

- `times manifest validate` checks the schema, version, scopes, TIMES tokens, light and dark icons, entry, locales, changelog, catalogs, and palette adoption. Each check prints PASS, WARN, or FAIL while a progress bar fills.
- Catalogs need `meta.name`. Amharic, Afaan Oromo, and Tigrinya catalogs must not be a copy of English. User-facing catalog strings cannot use an em dash.
- Apps and integrations need both `ui.icon` and `ui.iconDark`, and those files must differ.
- When source is present, it must link `themes.css`, follow the host color mode, and import `@times/aip-sdk` unless the app is static.
- `times init` scaffolds the dark icon, palette stylesheet, and color mode hook.

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
