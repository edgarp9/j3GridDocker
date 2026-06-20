#!/usr/bin/env python3
"""Build the Rust project in release mode and open the binary directory."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path


def run_checked(command: list[str], cwd: Path) -> subprocess.CompletedProcess[str]:
    print(f"$ {' '.join(command)}", flush=True)
    return subprocess.run(command, cwd=cwd, check=True, text=True)


def find_cargo() -> str:
    cargo = shutil.which("cargo")
    if cargo is None:
        raise RuntimeError("cargo executable was not found in PATH")
    return cargo


def read_target_directory(cargo: str, project_root: Path) -> Path:
    result = subprocess.run(
        [cargo, "metadata", "--format-version", "1", "--no-deps"],
        cwd=project_root,
        check=True,
        capture_output=True,
        text=True,
    )
    metadata = json.loads(result.stdout)
    return Path(metadata["target_directory"])


def open_directory(path: Path) -> None:
    if sys.platform.startswith("win"):
        os.startfile(str(path))  # type: ignore[attr-defined]
        return

    if sys.platform == "darwin":
        opener = shutil.which("open")
        if opener is not None:
            subprocess.Popen([opener, str(path)])
            return

    for name in ("xdg-open", "gio", "kde-open", "exo-open"):
        opener = shutil.which(name)
        if opener is None:
            continue
        if name == "gio":
            subprocess.Popen([opener, "open", str(path)])
        else:
            subprocess.Popen([opener, str(path)])
        return

    raise RuntimeError(
        "no file manager opener was found; install xdg-open/gio or open the directory manually"
    )


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Build this Rust project in release mode and open the binary directory."
    )
    parser.add_argument(
        "--no-open",
        action="store_true",
        help="skip opening the release binary directory after a successful build",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    project_root = Path(__file__).resolve().parent

    try:
        cargo = find_cargo()
        target_directory = read_target_directory(cargo, project_root)
        release_directory = target_directory / "release"

        run_checked([cargo, "build", "--release"], project_root)

        print(f"release binary directory: {release_directory}", flush=True)
        if not args.no_open:
            open_directory(release_directory)
            print("opened release binary directory", flush=True)
    except subprocess.CalledProcessError as error:
        return error.returncode or 1
    except (OSError, RuntimeError, KeyError, json.JSONDecodeError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
