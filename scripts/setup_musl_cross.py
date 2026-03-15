#!/usr/bin/env python3
"""
Download and setup musl-cross toolchain for static linking.
Uses benjaminwan/musl-cross-builder (GCC 14.2.0) which includes full C++ runtime support.
"""

import argparse
import platform
import shutil
import subprocess
import sys
import tempfile
import urllib.request
from pathlib import Path

VERSION = "14.2.0"
BASE_URL = (
    f"https://github.com/benjaminwan/musl-cross-builder/releases/download/{VERSION}"
)

MUSL_CC_URLS = {
    "x86_64": f"{BASE_URL}/x86_64-linux-musl-{VERSION}.7z",
    "aarch64": f"{BASE_URL}/aarch64-linux-musl-{VERSION}.7z",
    "riscv64": f"{BASE_URL}/riscv64-linux-musl-{VERSION}.7z",
}


def detect_arch():
    machine = platform.machine().lower()
    if machine in MUSL_CC_URLS:
        return machine
    print(f"Unknown architecture: {machine}")
    sys.exit(1)


def check_7z():
    try:
        subprocess.run(["7z"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        return True
    except FileNotFoundError:
        return False


def download_file(url, dest):
    print(f"Downloading {url}...")
    urllib.request.urlretrieve(url, dest)


def extract_7z(archive_path, dest_dir):
    print(f"Extracting to {dest_dir}...")
    subprocess.run(["7z", "x", str(archive_path), f"-o{dest_dir}", "-y"], check=True)


def main():
    parser = argparse.ArgumentParser(description="Setup musl-cross toolchain")
    parser.add_argument(
        "--arch", help="Target architecture (auto-detect if not specified)"
    )
    args = parser.parse_args()

    arch = args.arch or detect_arch()
    print(f"Target architecture: {arch}")

    if arch not in MUSL_CC_URLS:
        print(f"No toolchain available for {arch}")
        sys.exit(1)

    if not check_7z():
        print("Error: 7z is required. Install with: sudo apt install p7zip-full")
        sys.exit(1)

    script_dir = Path(__file__).parent
    project_root = script_dir.parent
    install_dir = project_root / "lib" / "musl-cross"
    toolchain_dir = install_dir / f"{arch}-linux-musl"

    if toolchain_dir.exists():
        print(f"Toolchain already installed at {toolchain_dir}")
        print_build_info(arch, toolchain_dir)
        return

    url = MUSL_CC_URLS[arch]
    filename = f"{arch}-linux-musl-{VERSION}.7z"

    install_dir.mkdir(parents=True, exist_ok=True)

    with tempfile.TemporaryDirectory() as tmpdir:
        archive_path = Path(tmpdir) / filename
        download_file(url, archive_path)
        extract_7z(archive_path, tmpdir)

        extracted_dir = Path(tmpdir) / f"{arch}-linux-musl"
        if not extracted_dir.exists():
            for item in Path(tmpdir).iterdir():
                if item.is_dir() and "linux-musl" in item.name:
                    extracted_dir = item
                    break

        if extracted_dir.exists():
            shutil.move(str(extracted_dir), str(toolchain_dir))
            print(f"Installed to {toolchain_dir}")
            print_build_info(arch, toolchain_dir)
        else:
            print("Error: Could not find extracted directory")
            sys.exit(1)


def print_build_info(arch, toolchain_dir):
    gcc_path = toolchain_dir / "bin" / f"{arch}-linux-musl-gcc"
    lib_path = toolchain_dir / "lib"

    print("\nBuild with:")
    print(f"  export CC={gcc_path}")
    print(f"  export CXX={gcc_path}")
    print(
        f"  cargo build --target {arch}-unknown-linux-musl --release --features musl --no-default-features"
    )

    print("\nRun with LD_LIBRARY_PATH:")
    print(
        f"  LD_LIBRARY_PATH={lib_path} ./target/{arch}-unknown-linux-musl/release/ferrinx-api"
    )


if __name__ == "__main__":
    main()
