# Windows: run inside WSL2 Ubuntu

This is the **Linux application inside WSL2**, not a native Windows `.exe`.
Native Windows Python, PowerShell builds and Windows tool executables are not
supported by this starter. macOS remains the tested desktop platform. Synthetic
platform checks and a credential-free Linux container check do not establish
physical Windows/WSL2 or authenticated agent acceptance.

## Install WSL2

In an administrator PowerShell terminal:

```powershell
wsl --install -d Ubuntu
```

Restart if prompted, open **Ubuntu**, and create a normal Linux user. Do not run
the app as root. Check the distribution uses version **2** from PowerShell:

```powershell
wsl --list --verbose
# Only if the Ubuntu entry is version 1:
wsl --set-version Ubuntu 2
```

See Microsoft's [WSL installation guide](https://learn.microsoft.com/en-us/windows/wsl/install).
Use Ubuntu 24.04 or newer for its Python 3.11+ package; an older Ubuntu release
with Python 3.10 does not meet this starter's minimum.

## Install Linux tools and sign in inside Ubuntu

All following commands run in the **Ubuntu terminal**, not PowerShell. Install
Git, Python, a C compiler for bundled SQLite, process inspection and the Linux
sandbox prerequisites:

```sh
sudo apt update
sudo apt install git gh python3 build-essential pkg-config ca-certificates procps bubblewrap ripgrep libssl-dev
```

Install [Rust using rustup](https://www.rust-lang.org/tools/install) **inside
Ubuntu**, with Rust/Cargo 1.85 or newer. Install a Linux Node.js 20+ and npm using
[Node's installation guidance](https://nodejs.org/en/download); Windows Node/npm
cannot be used. Then install the pinned Linux Codex package:

```sh
npm install -g @openai/codex@0.153.4
python3 --version
rustc --version
cargo --version
node --version
codex --version
command -v git gh cargo node npm codex
```

The paths must be Linux paths (for example `/usr/bin` or `$HOME/.cargo/bin`),
not `/mnt/c`, Windows Program Files or `.exe` files. If PATH selects Windows
tools, install their Linux versions and put those Linux directories first.
The bootstrap removes Windows-mounted PATH entries and refuses Windows PE
executables, including renamed ones. It never falls back to unsandboxed agents.

Sign in separately within Ubuntu. Existing Windows login caches are not reused:

```sh
gh auth login --hostname github.com
codex login
chmod 700 "$HOME/.codex"
chmod 600 "$HOME/.codex/auth.json"
```

A dedicated Linux Codex home may be selected with `--codex-home`. Never use a
Windows credential directory under `/mnt/c` and never copy someone else's auth.

## Clone, initialize and open the UI

Keep the checkout, state, credentials and managed repositories on the **Linux
filesystem**, for example your Ubuntu home directory. Do not clone under `/mnt/c` or another
Windows drive; the bootstrap also rejects detected Windows-backed custom mounts.
Changing NTFS permissions with chmod is not a substitute. Microsoft's
[filesystem and localhost guidance](https://learn.microsoft.com/en-us/windows/dev-environment/wsl-interop)
explains the filesystem boundary and Windows-to-WSL localhost forwarding.

```sh
mkdir -p ~/projects
cd ~/projects
git clone https://github.com/agonza1/Software-Agent-Orchestrator.git
cd Software-Agent-Orchestrator
./orchestrator init
./orchestrator doctor
./orchestrator start
```

`init` builds **Linux** binaries. The default private state is
`~/.local/share/software-agent-orchestrator` (or `$XDG_DATA_HOME` on a Linux
filesystem). Do not reuse a macOS/Windows binary directory. Overrides such as
`--state-dir`, `--projects-root` and `--read-root` must also stay in Linux.
Read roots grant toolchain access; do not grant your whole home directory.

Open **http://127.0.0.1:4311/#work** in your Windows browser. In a **second Ubuntu
terminal**, inside the same checkout, run:

```sh
./orchestrator key
```

The key is revealed only in an interactive terminal. Paste it into the local UI
once; never put it in shell logs, tickets or screenshots. The app starts paused;
review System before resuming launches. Stop the foreground server with Ctrl+C.
If localhost is unavailable, check WSL is running and your firewall/VPN's WSL
localhost support. Do not solve it by binding this single-user app publicly.

## Sandbox and validation limits

Linux execution requires working user/mount/PID namespaces and the Codex Linux
sandbox (bubblewrap or its bundled helper). See [official Codex sandbox guidance](https://learn.chatgpt.com/docs/sandboxing).
Containers, older kernels or corporate WSL policies can prohibit unprivileged
namespaces. A failure must remain blocked/unknown: do not add privileged mode,
disable sandboxing or use a dangerous bypass flag. `doctor` checks saved pins and
account references; it does **not** prove kernel sandbox support or run a model.
WSL1 is not a supported alternative.

Before creating a real task, you can check that the installed CLI can enter its
Linux sandbox without credentials or inference. In Ubuntu:

```sh
probe=$(mktemp -d)
mkdir "$probe/home" "$probe/cli" "$probe/workspace"
env -i PATH="$PATH" HOME="$probe/home" CODEX_HOME="$probe/cli" \
  codex sandbox --permission-profile starter-check --cd "$probe/workspace" \
  -c 'permissions.starter-check.extends=":read-only"' \
  -c 'permissions.starter-check.network.enabled=false' /usr/bin/true
# Expected: exit 0. If namespaces are denied, stop here and resolve WSL support.
rm -rf -- "$probe"
```

This only checks sandbox entry; it does not validate provider access or the full
application policy. It may report unavailable namespaces inside Docker even
though a WSL2 installation supports them. Do not run the app outside its sandbox
as a workaround.

The validation for this release is reported with its handoff. It does not claim
physical Windows testing, a WSL2 browser/provider canary, Windows-native packaging
or unrestricted filesystem confidentiality against the same OS user. Main
remains text-only/read-only; Coder and QA retain their separate native sandbox,
exact-commit checks, cleanup/deadline enforcement and scoped publisher.
