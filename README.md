# tailscale-ui

`tailscale-ui` is a Linux tray controller for Tailscale written in Rust.

It wraps the `tailscale` CLI and gives you a tray menu for the common actions:

- show connection status
- connect or disconnect Tailscale
- choose and persist an exit node
- reapply the saved exit node after reconnecting
- toggle LAN access when using an exit node
- start or stop the local Tailscale web interface
- open the local web interface in a browser
- open the Tailscale login page and admin console
- toggle autostart on login
- open the config folder

## Release model

This repository is released by tag. The current release is `v0.1.2`.

That means later code changes will not modify the `v0.1.2` release. If the code changes again, it will be released under a new tag and new release assets.

## Requirements

- Linux desktop environment with tray support
- `tailscale` installed and available in `PATH`
- Rust toolchain if you want to build from source

## Known bug

The application requires `tailscale` to be installed and available on the machine.

If the `tailscale` command is missing from `PATH`, the app quits immediately and no tray icon is created.

## Install from source

If you want to build it yourself with Rust:

```bash
sudo apt install cargo pkg-config libdbus-1-dev xdg-utils tailscale
cargo build --release
./target/release/tailscale-ui
```

You can also install it into your Cargo bin directory:

```bash
cargo install --path .
```

## Install from the GitHub release

The release includes a Debian package. Install it directly with `apt`:

```bash
sudo apt install ./tailscale-ui_0.1.2_amd64.deb
```

If you downloaded the `.deb` somewhere else, point `apt` at that file instead:

```bash
sudo apt install /path/to/tailscale-ui_0.1.2_amd64.deb
```

## Debian package contents

The package installs:

- the tray binary in `/usr/bin/tailscale-ui`
- a desktop entry in `/usr/share/applications/tailscale-ui.desktop`
- an application icon in `/usr/share/icons/hicolor/scalable/apps/tailscale-ui.svg`
- documentation under `/usr/share/doc/tailscale-ui/`

The desktop entry is installed so the app appears in the desktop's application launcher or "Show Apps" grid.

The package depends on:

- `xdg-utils`
- `libdbus-1-3`

It recommends `tailscale`, which means the package can still be installed even if
`tailscale` is not available from your configured APT sources. The app will exit
at runtime if the `tailscale` command is missing from `PATH`.

## Building the Debian package

The repository includes a packaging script:

```bash
./scripts/build-deb.sh
```

It builds the release binary, stages a package tree, and creates a `.deb` in `dist/`.

## What the tool does

The tray app reads `tailscale status --json` on a timer and updates the indicator icon and menu. It can also call:

- `tailscale up`
- `tailscale down`
- `tailscale set --exit-node=...`
- `tailscale set --exit-node-allow-lan-access=...`
- `tailscale web --listen 127.0.0.1:8088`

It stores its settings in the user config directory and remembers:

- the chosen exit node
- whether exit-node use is enabled
- whether LAN access is allowed through the exit node
- whether autostart is enabled

## Release notes

The tray icon uses status colors that are visible across tray themes:

- green for connected
- yellow for login required
- red for errors
- gray for disconnected
