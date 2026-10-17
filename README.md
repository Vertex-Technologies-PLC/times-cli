# times

The TIMES developer CLI. Build, test and publish TIMES AIPs from your terminal.

## Install

macOS and Linux:

```sh
curl -fsSL https://developers.times.et/install.sh | sh
```

zsh and bash ship a builtin named `times`. The installer disables it in your shell config so this CLI can run. The binary stays `times`. If an existing terminal still prints `too many arguments`, run `. ~/.config/times/shellenv` or open a new terminal.

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
times auth login
times init my-app
cd my-app
times aip create
times aip push
times aip submit
```

## Commands

| Command | What it does |
| --- | --- |
| `times auth login` | Sign in with your browser. `--token` signs in with a developer token. `--profile` names the account. |
| `times auth logout` | Sign out of the current profile. |
| `times auth whoami` | Show the signed in user, profile and organization. |
| `times auth refresh` | Refresh the stored browser session. |
| `times auth list` / `times auth use <name>` / `times auth delete <name>` | List, switch and remove profiles. |
| `times org list` / `times org use <slug>` | List and switch organizations. |
| `times init [dir]` | Create a new AIP project. Creates a folder named after the slug unless `dir` is `.`. |
| `times manifest validate` / `times manifest bump <level>` | Check or bump `times.aip.json`. |
| `times dev` | Serve your build locally. |
| `times aip list\|create\|info\|push\|submit\|status\|scan\|doctor\|listing\|origin\|delete\|open\|logs\|withdraw` | Manage AIPs and versions. |
| `times stats` | Installs, usage and revenue. |
| `times earnings` | Balance and earnings. |
| `times payout list\|accounts\|request` | Withdrawals and payout accounts. |
| `times token list\|create\|revoke` | Developer tokens for CI. |
| `times completions <shell>` | Shell completions. |
| `times setup-shell` | Disable the POSIX `times` builtin in your shell config. |
| `times upgrade` | Install the latest CLI. |

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
| `TIMES_PROFILE` | Credentials profile for this invocation. Does not change the saved current profile. |
| `TIMES_NO_INPUT` | Never prompt. |
| `TIMES_NO_UPDATE_CHECK` | Skip the GitHub version check. |
| `TIMES_CONFIG_DIR` | Config directory. |
| `TIMES_CREDENTIAL_STORE=keyring` | Store credentials in the OS keychain (macOS/Windows). Default is `~/.config/times/credentials.json` (mode 0600). |

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
