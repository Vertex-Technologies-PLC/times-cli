# times

The TIMES developer CLI. Build, test and publish TIMES AIPs from your terminal.

## Install

macOS and Linux:

```sh
curl -fsSL https://developers.times.et/install.sh | sh
```

Windows (PowerShell):

```powershell
irm https://developers.times.et/install.ps1 | iex
```

Or download a binary from [Releases](https://github.com/Vertex-Technologies-PLC/times-cli/releases).

From source:

```sh
cargo install --git https://github.com/Vertex-Technologies-PLC/times-cli
```

## Quick start

```sh
times login
times init my-app
cd my-app
bun install && bun run build
times dev
times aip create
times aip push
times aip submit
```

## Commands

| Command | What it does |
| --- | --- |
| `times login` | Sign in with your browser. `--token` signs in with a developer token. |
| `times logout` | Sign out and remove stored credentials. |
| `times whoami` | Show the signed in user and organization. |
| `times org list` / `times org use <slug>` | List and switch organizations. |
| `times init [name]` | Create a new AIP project. |
| `times manifest validate` / `times manifest bump <level>` | Check or bump `times.aip.json`. |
| `times dev` | Serve your build locally. |
| `times aip list\|create\|info\|push\|submit\|status\|logs\|withdraw` | Manage AIPs and versions. |
| `times stats` | Installs, usage and revenue. |
| `times earnings` | Balance and earnings. |
| `times payout list\|accounts\|request` | Withdrawals and payout accounts. |
| `times token list\|create\|revoke` | Developer tokens for CI. |
| `times completions <shell>` | Shell completions. |

Global flags: `--json`, `--org <slug>`, `--api-url <url>`, `--profile <name>`, `--no-input`, `--yes`.

## CI

Create a token with `times token create --name ci --scopes aips:read,aips:write,aips:submit`, then set it as `TIMES_TOKEN`. The CLI exchanges it for a short lived access token on every run.

```sh
TIMES_TOKEN=tdk_... times aip push --no-input
```

## Environment

| Variable | Use |
| --- | --- |
| `TIMES_TOKEN` | Developer token. Overrides stored credentials. |
| `TIMES_API_URL` | API gateway. Default `https://api.times.et`. |
| `TIMES_ORG` | Organization slug. |
| `TIMES_PROFILE` | Credentials profile. |
| `TIMES_NO_INPUT` | Never prompt. |
| `TIMES_CONFIG_DIR` | Config directory. |
| `TIMES_CREDENTIAL_STORE=file` | Store credentials in a file instead of the OS keychain. |

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | Success |
| 1 | Unexpected error |
| 2 | Bad usage |
| 3 | Not signed in or session expired |
| 4 | API rejected the request |
| 5 | Validation failed |
| 6 | Network error |

## Development

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Docs: https://developers.times.et/docs/cli-reference
