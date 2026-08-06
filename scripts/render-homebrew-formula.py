#!/usr/bin/env python3
"""Render the Homebrew formula from release checksums."""

from __future__ import annotations

import argparse
import re
from pathlib import Path


REPOSITORY = "Juberstine/codex-deepseek-switcher"
FORMULA_ASSETS = {
    "linux_aarch64": "codex-deepseek-switcher-linux-aarch64.tar.gz",
    "linux_x86_64": "codex-deepseek-switcher-linux-x86_64.tar.gz",
    "macos_aarch64": "codex-deepseek-switcher-macos-aarch64.tar.gz",
    "macos_x86_64": "codex-deepseek-switcher-macos-x86_64.tar.gz",
}
SHA256 = re.compile(r"^[0-9a-f]{64}$")
VERSION = re.compile(r"^[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?$")


def parse_checksums(contents: str) -> dict[str, str]:
    checksums: dict[str, str] = {}
    for line in contents.splitlines():
        if not line.strip():
            continue

        parts = line.split()
        if len(parts) != 2:
            raise ValueError(f"invalid checksum line: {line!r}")

        checksum, raw_name = parts
        name = raw_name.removeprefix("*").removeprefix("./")
        if not SHA256.fullmatch(checksum):
            raise ValueError(f"invalid SHA-256 checksum for {name!r}")
        if name in checksums:
            raise ValueError(f"duplicate checksum for {name!r}")
        checksums[name] = checksum

    missing = sorted(set(FORMULA_ASSETS.values()) - checksums.keys())
    if missing:
        raise ValueError(f"missing checksums for: {', '.join(missing)}")
    return checksums


def render_formula(version: str, checksums: dict[str, str]) -> str:
    if not VERSION.fullmatch(version):
        raise ValueError(f"invalid release version: {version!r}")

    values = {
        name: checksums[asset]
        for name, asset in FORMULA_ASSETS.items()
    }
    release_url = f"https://github.com/{REPOSITORY}/releases/download/v{version}"

    return f'''class CodexDeepseekSwitcher < Formula
  desc "Safely switch Codex between OpenAI and DeepSeek"
  homepage "https://github.com/{REPOSITORY}"
  version "{version}"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "{release_url}/{FORMULA_ASSETS["macos_aarch64"]}"
      sha256 "{values["macos_aarch64"]}"
    else
      url "{release_url}/{FORMULA_ASSETS["macos_x86_64"]}"
      sha256 "{values["macos_x86_64"]}"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "{release_url}/{FORMULA_ASSETS["linux_aarch64"]}"
      sha256 "{values["linux_aarch64"]}"
    else
      url "{release_url}/{FORMULA_ASSETS["linux_x86_64"]}"
      sha256 "{values["linux_x86_64"]}"
    end
  end

  def install
    bin.install "codex-deepseek-switcher"
  end

  test do
    assert_match version.to_s, shell_output("#{{bin}}/codex-deepseek-switcher --version")
  end
end
'''


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--version", required=True)
    parser.add_argument("--checksums", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()

    checksums = parse_checksums(args.checksums.read_text(encoding="utf-8"))
    formula = render_formula(args.version, checksums)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(formula, encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
