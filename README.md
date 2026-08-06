# DeepSwitch

A cross-platform CLI that switches the shared Codex configuration between your
existing OpenAI/Codex setup and DeepSeek V4 Flash without storing the DeepSeek
API key in `config.toml`, an environment variable, or a shell profile.

The tool follows DeepSeek's Codex integration settings, but replaces the
documented plaintext `experimental_bearer_token` with Codex's command-backed
authentication and the operating system's credential store.

## Why use this instead of the official setup script?

DeepSeek's official [Codex integration](https://api-docs.deepseek.com/quick_start/agent_integrations/codex/)
and setup scripts write the API key directly to `config.toml` as
`experimental_bearer_token`. They also set global API-login overrides, which
can sign Codex out of an existing OpenAI session.

This switcher improves that design by:

- storing the DeepSeek key in the native OS credential store
- using a hidden key prompt instead of echoing the key or requiring an
  environment variable
- supplying the key only through Codex's provider-specific credential command
- preserving Codex's existing OpenAI login and credential-storage settings
- sanitizing legacy plaintext tokens from generated configuration and backups
- using private Unix permissions, validated configuration, atomic writes, and
  comment-preserving TOML updates
- testing Linux, macOS, Windows, switching behavior, release packaging, and
  Homebrew installation in CI

The official script currently supports DeepSeek V4 Flash and V4 Pro. This
switcher currently supports V4 Flash only.

## Requirements

- Codex CLI or Codex desktop initialized at least once
- Codex 0.144.0 or newer
- A DeepSeek API key beginning with `sk-`
- An available native credential store:
  - macOS Keychain
  - Windows Credential Manager
  - Linux Secret Service

Linux and WSL require a running, unlocked Secret Service provider and a working
session D-Bus. GNOME Keyring and KeePassXC's Secret Service integration are
common choices. The CLI reports platform-specific guidance if the keyring is
unavailable.

## Install

On macOS or Linux, install from the
[Homebrew tap](https://github.com/Juberstine/homebrew-tap):

```sh
brew install Juberstine/tap/deepswitch
```

Future releases are available through `brew update` and
`brew upgrade deepswitch`.

On any supported platform, you can instead download the archive from
[GitHub Releases](https://github.com/Juberstine/deepswitch/releases), extract
it, and place `deepswitch` (or `deepswitch.exe`) on your `PATH`. Each release
includes:

- static Linux binaries for x86-64 and ARM64
- macOS binaries for Intel and Apple Silicon
- Windows binaries for x86-64 and ARM64
- a `SHA256SUMS` checksum manifest

Linux artifacts are fully statically linked with musl. macOS artifacts are
single executables but dynamically link Apple system frameworks, which is
required for Keychain access.

Alternatively, install Rust and build the executable into Cargo's binary
directory:

```sh
cargo install --path .
```

Keep the installed executable at a stable path. If it moves, run
`deepswitch deepseek` again so Codex's credential-helper path is updated.

## Windows and WSL

Install and run the switcher in the same environment where the Codex process
runs. Windows and WSL are separate installations:

- Native Windows uses the `.exe`, `%USERPROFILE%\.codex`, and Windows
  Credential Manager. Use this for Codex desktop and for Codex CLI launched
  from Windows.
- WSL uses the Linux binary, the distro's `~/.codex`, and Linux Secret Service
  over the WSL session D-Bus. Use this for Codex CLI launched inside WSL.

Codex desktop is a native application and does not run inside WSL. A Windows
Codex process cannot execute the Linux credential-helper path, and a WSL Codex
CLI cannot use Windows Credential Manager through the Linux keyring API. Run
`setup` separately in Windows and WSL if you use both.

`deepswitch status` prints the detected runtime, Codex home, and credential
backend. WSL requires a Secret Service provider such as GNOME Keyring or
KeePassXC; it is not enabled by default in every distro.

## Use

Initial setup securely prompts for the key and activates DeepSeek:

```sh
deepswitch setup
```

Switch providers and inspect the current state:

```sh
deepswitch codex
deepswitch deepseek
deepswitch status
```

Running `deepswitch` without a command prompts you to choose Codex or DeepSeek.

Rotate or delete the DeepSeek key:

```sh
deepswitch key set
deepswitch key delete
```

Restart a running Codex CLI or Codex desktop session after a switch. Codex may
show different session-history groups for OpenAI login and third-party API
authentication; switching back makes the other group visible again.

## What setup changes

The switcher:

- saves the original top-level provider, model, reasoning, and catalog
  selections
- writes the DeepSeek V4 Flash metadata to `~/.codex/models.json`
- adds `[model_providers.deepseek]` using the Responses API at
  `https://api.deepseek.com/`
- configures `[model_providers.deepseek.auth]` to retrieve the key from this
  executable
- leaves Codex's OpenAI login method and credential storage unchanged

Unrelated configuration—including MCP servers and project trust—is preserved.
`deepswitch codex` restores the exact selection captured before the first DeepSeek
switch. The DeepSeek provider definition remains installed but inactive.
Switching providers does not clear or replace the saved OpenAI login, so both
providers remain usable without signing in again.

Configuration writes use validated TOML/JSON, same-directory temporary files,
and atomic replacement. Existing `config.toml` and `models.json` files are
backed up under `~/.codex/deepseek-switcher/backups/` before changes. On Unix,
state, backups, and generated files are restricted to the current user.
Known plaintext `experimental_bearer_token` values are removed rather than
copied into new configuration or backups.

The state directory and credential-store identifier retain their original
internal names so upgrades preserve existing keys and rollback data.

If DeepSeek is already selected before the switcher has captured original
state, setup stops and asks you to restore Codex manually first. This avoids
guessing which provider settings should be restored.

## Security model

The DeepSeek key exists persistently only in the native keychain. Codex invokes
the hidden credential-helper command when it needs a bearer token; the helper
prints only the token to its stdout, which Codex consumes. The key is held in
zeroizing memory while the helper runs and is never accepted as a command-line
argument.

The OS user account remains the trust boundary. A malicious process already
running as the same unlocked user may be able to invoke the helper or request
the credential from the OS keychain. This tool does not protect a compromised
user session.

## Docker development and testing

All formatting, linting, and tests run through the development container:

```sh
docker compose run --rm dev
```

This executes `cargo fmt --check`, Clippy with warnings denied, the full test
suite, a static musl release build, linkage verification, and archive
packaging. Cargo registry and target data are cached in named Docker volumes.

Containers normally have no access to the host's graphical OS keychain. Tests
therefore use an in-memory credential-store implementation for the keyring
contract and never access a real credential. Run the installed production
binary on the host, not inside the development container.

To run a single Cargo command in the same environment:

```sh
docker compose run --rm dev cargo test config::
```

## Publishing releases

The release workflow builds all six platform archives on native GitHub-hosted
runners. A manual workflow run builds and uploads temporary workflow artifacts
without publishing a release, making it safe for testing.

Every pull request must update the version in `Cargo.toml` and `Cargo.lock`.
CI rejects a version that already has a GitHub release. When the pull request
merges to `main`, the release workflow:

1. builds and smoke-tests all six platform archives
2. verifies that both Linux binaries are statically linked
3. generates SHA-256 checksums and public-repository provenance attestations
4. creates the matching `v<version>` tag and GitHub release
5. updates the macOS and Linux formula in `Juberstine/homebrew-tap`

The tap update uses the repository secret `HOMEBREW_TAP_DEPLOY_KEY`, paired
with a write-enabled deploy key scoped only to the tap repository.

The merge fails visibly at the release stage if its Cargo version was already
published. Manual workflow runs never create tags or releases.

## License

Licensed under the [MIT License](LICENSE).
