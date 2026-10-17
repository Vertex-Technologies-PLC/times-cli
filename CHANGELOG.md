# Changelog

## 0.4.0

- `times aip push` runs `package.json` `build` when the TIMES-hosted output folder is missing. Pass `--no-build` in CI that already built.
- `times.aip.json` can set `hosting.provider` to `times` (zip to TIMES) or `developer` (public HTTPS origin). Old manifests still infer from `ui.entry`.
- Push is fail-closed: local scan, then a server re-scan. Findings block the upload. Human review is still required.
- Origin checks refuse private, loopback, link-local, and metadata IPs, including DNS that resolves there. HTTPS on port 443 only. Redirects fail closed.
- Guest zip scan rejects remote HTML tags, `javascript:` URLs, inline event handlers, PE/ELF/Mach-O/Wasm binaries, and extra secret prefixes.
- Source `eval(` / `new Function` / `innerHTML` matching ignores comments and string literals.
- New commands: `times aip scan`, `times aip origin check`, `times aip doctor`, `times aip listing get|set`, `times aip delete`, `times aip open`.
