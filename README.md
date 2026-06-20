# j3GridDocker

j3GridDocker is a Windows desktop utility for organizing external application
windows into a tabbed splitter grid.

It provides a lightweight workspace where top-level application windows can be
dropped into regions, resized with splitters, moved between tabs, hidden with
inactive tabs, and restored when they are undocked.

## GitHub Description

Tabbed Windows desktop window docker with splitter-based layouts and external
app window docking.

## Project Status

This project was created as an in-house tool with AI assistance. It is still
experimental, and test coverage is not sufficient yet. Please expect rough
edges, review behavior carefully before relying on it, and verify important
workflows in your own environment.

## Features

- Tab-based workspaces, each with an independent splitter layout.
- Drag-and-drop docking for external top-level application windows.
- Region splitting, resizing, deletion, and undocking.
- Tab presets for saving and reusing layouts.
- Optional workspace-control hiding so docked windows can remain visible with a
  smaller j3GridDocker UI footprint.
- English and Korean UI language support.
- Settings stored next to the executable as a TOML file.

## How It Works

j3GridDocker keeps docked applications as normal top-level windows. It controls
their position, size, visibility, and owner relationship instead of embedding
them as child windows. When a tab becomes inactive, its docked windows are
hidden; when the tab becomes active again, they are shown and aligned to the
current grid regions.

## Build

Requirements:

- Windows
- Rust 1.93 or newer
- Cargo available in `PATH`

From the Rust project directory:

```powershell
cd src
cargo build --release
```

The release executable is generated under:

```text
src/target/release/
```

You can also use the helper script:

```powershell
cd src
python build_release.py
```

## Test

Run the available Rust tests from the Rust project directory:

```powershell
cd src
cargo test
```

The current test suite is incomplete. Passing tests do not mean the full window
docking workflow has been validated.

## License

This project is distributed under the GNU General Public License v3.0. See
`LICENSE` for details.

## Third-Party Notice

This project uses icons from
[Google Fonts Material Symbols and Icons](https://fonts.google.com/icons).
Google's [Material Symbols guide](https://developers.google.com/fonts/docs/material_symbols)
and [Material Icons guide](https://developers.google.com/fonts/docs/material_icons)
state that these icons are available under the
[Apache License Version 2.0](https://www.apache.org/licenses/LICENSE-2.0).

Thank you to Google and the Material Symbols and Icons contributors for making
these icons available.
