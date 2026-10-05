# Linux and Windows setup

The CLI and desktop share `orochi init`. macOS uses Lima; Linux uses local Incus.
Windows runs Orochi, its agents and its desktop inside WSL2. The Windows PowerShell
launcher handles WSL installation, building the Linux binaries and translating Windows
working-directory/config/data paths. This is WSL support, not a native Windows executable.
All platforms keep one Incus sandbox per conversation and independent Git worktrees for
writable agents in that conversation.

## Linux

From a source checkout:

```sh
bash tools/linux/install.sh             # CLI, build dependencies, Rust and Incus setup
bash tools/linux/install.sh --desktop   # also build and install the desktop
```

If Orochi is already installed, use `orochi init` or the desktop's
**Sandboxes → Initialize sandbox environment**. Missing local Incus is installed and its
storage, networking and profiles are prepared automatically. `orochi init --host-only`
defers the large golden image downloads. The installer supports apt (Debian/Ubuntu), dnf
(Fedora) and pacman (Arch); other distributions need their build/Incus packages installed
manually. A running systemd system is required. Desktop dependencies follow the
[Tauri prerequisites](https://v2.tauri.app/start/prerequisites/).

Only host setup runs with administrator authorization: sudo in the terminal, or pkexec
from the desktop when available. If desktop authorization is unavailable, its job reports
the error and asks you to run `orochi init` in a terminal. Orochi itself runs as your normal
user. The installed `orochi-incus` wrapper uses `sg incus-admin`, allowing immediate access
after setup without requiring a logout or a permanent sudo exemption. Incus administrators
have privileged control of the host, as described in [Incus first steps](https://linuxcontainers.org/incus/docs/main/tutorial/first_steps/).

Lima keeps its ZFS pool. A new native Linux/WSL pool uses Btrfs when the kernel supports it
and a loop device is available, otherwise directory storage. Existing pools are preserved. Directory storage does not
offer per-container disk quotas or copy-on-write clones; Orochi omits unsupported root
quotas and reports that limitation. Linux/WSL setup never installs a host poweroff timer.
The VM tier requires working KVM support; containers and runner mode do not.

Explicit custom Incus commands and configured remotes are reused, not installed or
modified. Prepare those remote hosts separately with `orochi sandbox host-script`.

## Windows

Use a current WSL2 installation and Ubuntu 24.04. In PowerShell, from this checkout:

```powershell
./tools/windows/orochi.ps1 -Setup
./tools/windows/orochi.ps1 -Setup -Desktop
./tools/windows/orochi.ps1 -WorkingDirectory /home/me/project "Implement the task"
./tools/windows/orochi.ps1 -Desktop -WorkingDirectory /home/me/project
```

If WSL is not available, run `wsl --install` in administrator PowerShell. Windows may
require a reboot; rerun `-Setup` afterwards. Complete Ubuntu's initial Linux username and
password prompts. The launcher installs the distribution if missing and selects WSL2.
Use `-Distribution` to select another installed distribution. It does not terminate other
running distributions or modify your global WSL configuration.

Systemd must be enabled in the selected distribution. On older WSL installations, add
`[boot]` / `systemd=true` to `/etc/wsl.conf`, restart that distribution, then rerun setup,
as documented by [Microsoft](https://learn.microsoft.com/en-us/windows/wsl/wsl-config#systemd-support).
The desktop is a Linux application shown on Windows through
[WSLg](https://learn.microsoft.com/en-us/windows/wsl/tutorials/gui-apps).
Its agents, credentials, Git worktrees and SQLite data all remain in WSL.

Prefer repositories on the Linux filesystem (`/home/...`). Without `-WorkingDirectory`,
the launcher translates the current Windows directory. Separate `--cwd`/`-C`, `--config`
and `--data-dir` values that contain drive paths are also translated; other path arguments
should be supplied as Linux paths. Task text is passed as an argument, never as shell code.
The optional VM tier is unavailable when WSL does not expose KVM; use container/runner mode.

## Validation limits

Mac regression tests exercise the Incus client and directory-storage behavior with a
simulated Incus backend. The PowerShell launcher is tested with a simulated WSL executable
for argument preservation, path conversion, setup, desktop selection and errors, including
in a Windows CI job. WSL2/WSLg execution still requires validation on an actual Windows
machine; a passing launcher test does not establish kernel, networking or GUI compatibility.
