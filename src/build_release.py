#!/usr/bin/env python3
"""Build the Rust project in release mode and prepare distribution files."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import zipfile
from pathlib import Path


EXCLUDED_SOURCE_DIRS = {
    ".git",
    ".idea",
    ".my",
    ".vscode",
    "__pycache__",
    "coverage",
    "criterion",
    "dist",
    "target",
}
EXCLUDED_SOURCE_NAMES = {
    ".DS_Store",
    "Desktop.ini",
    "Thumbs.db",
    "cargo-tarpaulin-report.xml",
    "flamegraph.svg",
    "tarpaulin-report.html",
}
EXCLUDED_SOURCE_SUFFIXES = (
    ".bak",
    ".ilk",
    ".log",
    ".pdb",
    ".profdata",
    ".profraw",
    ".rlib",
    ".rmeta",
    ".swo",
    ".swp",
    ".tmp",
)


def run_checked(command: list[str], cwd: Path) -> subprocess.CompletedProcess[str]:
    print(f"$ {' '.join(command)}", flush=True)
    return subprocess.run(command, cwd=cwd, check=True, text=True)


def copy_release_license_files(project_root: Path, release_directory: Path) -> None:
    for filename in ("LICENSE", "THIRD_PARTY_NOTICES.txt", "about.txt"):
        source = project_root / filename
        if source.is_file():
            shutil.copy2(source, release_directory / source.name)
            print(f"copied {source.name} to release directory", flush=True)


def find_cargo() -> str:
    cargo = shutil.which("cargo")
    if cargo is None:
        raise RuntimeError("cargo executable was not found in PATH")
    return cargo


def read_cargo_metadata(cargo: str, project_root: Path) -> dict[str, object]:
    result = subprocess.run(
        [cargo, "metadata", "--format-version", "1", "--no-deps"],
        cwd=project_root,
        check=True,
        capture_output=True,
        text=True,
    )
    return json.loads(result.stdout)


def read_target_directory(metadata: dict[str, object]) -> Path:
    target_directory = metadata["target_directory"]
    if not isinstance(target_directory, str):
        raise RuntimeError("cargo metadata target_directory was not a string")
    return Path(target_directory)


def read_root_package(metadata: dict[str, object]) -> tuple[str, str]:
    packages = metadata["packages"]
    if not isinstance(packages, list) or len(packages) != 1:
        raise RuntimeError("cargo metadata did not return exactly one root package")

    package = packages[0]
    if not isinstance(package, dict):
        raise RuntimeError("cargo metadata package entry was not an object")

    name = package.get("name")
    version = package.get("version")
    if not isinstance(name, str) or not isinstance(version, str):
        raise RuntimeError("cargo metadata package name/version was not a string")

    return name, version


def should_include_source_file(project_root: Path, path: Path) -> bool:
    relative = path.relative_to(project_root)
    if any(part in EXCLUDED_SOURCE_DIRS for part in relative.parts):
        return False

    name = path.name
    if name in EXCLUDED_SOURCE_NAMES or name.endswith("~"):
        return False

    return not name.endswith(EXCLUDED_SOURCE_SUFFIXES)


def iter_release_source_files(project_root: Path):
    for directory, dir_names, file_names in os.walk(project_root):
        dir_names[:] = sorted(
            dir_name for dir_name in dir_names if dir_name not in EXCLUDED_SOURCE_DIRS
        )

        current_directory = Path(directory)
        for file_name in sorted(file_names):
            path = current_directory / file_name
            if should_include_source_file(project_root, path):
                yield path


def create_release_source_archive(
    project_root: Path, release_directory: Path, package_name: str, package_version: str
) -> Path:
    archive_root = f"{package_name}-{package_version}-source"
    archive_path = release_directory / f"{archive_root}.zip"

    with zipfile.ZipFile(archive_path, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        for path in iter_release_source_files(project_root):
            relative = path.relative_to(project_root).as_posix()
            archive.write(path, f"{archive_root}/{relative}")

    print(f"created source archive {archive_path.name}", flush=True)
    return archive_path


def release_executable_name(package_name: str) -> str:
    if sys.platform.startswith("win"):
        return f"{package_name}.exe"
    return package_name


def create_release_binary_archive(
    project_root: Path,
    release_directory: Path,
    package_name: str,
    package_version: str,
    source_archive: Path,
) -> Path:
    archive_root = f"{package_name}-{package_version}-windows"
    archive_path = release_directory / f"{archive_root}.zip"
    executable_path = release_directory / release_executable_name(package_name)
    required_files = [
        executable_path,
        release_directory / "LICENSE",
        release_directory / "THIRD_PARTY_NOTICES.txt",
        release_directory / "about.txt",
        source_archive,
    ]

    missing = [path.name for path in required_files if not path.is_file()]
    if missing:
        raise RuntimeError(
            "cannot create binary archive; missing release files: " + ", ".join(missing)
        )

    with zipfile.ZipFile(archive_path, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        for path in required_files:
            archive.write(path, f"{archive_root}/{path.name}")

    print(f"created binary archive {archive_path.name}", flush=True)
    return archive_path


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
        metadata = read_cargo_metadata(cargo, project_root)
        target_directory = read_target_directory(metadata)
        package_name, package_version = read_root_package(metadata)
        release_directory = target_directory / "release"

        run_checked([cargo, "build", "--release"], project_root)
        copy_release_license_files(project_root, release_directory)
        source_archive = create_release_source_archive(
            project_root, release_directory, package_name, package_version
        )
        create_release_binary_archive(
            project_root,
            release_directory,
            package_name,
            package_version,
            source_archive,
        )

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
