#!/usr/bin/env python3
"""Reject agent-attribution trailers in commits added by a change."""

from __future__ import annotations

import re
import subprocess
import sys


DISALLOWED = re.compile(
    r"(?im)(cursoragent@cursor\.com|"
    r"co-authored-by:\s*cursor(?:\s|<|$)|"
    r"generated(?:-| )by:\s*cursor(?:\s|$))"
)


def git(*args: str, check: bool = True) -> str:
    result = subprocess.run(
        ["git", *args],
        check=check,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        text=True,
    )
    return result.stdout


def commit_range(base: str, head: str) -> list[str]:
    if base and set(base) != {"0"}:
        result = git("rev-list", f"{base}..{head}", check=False)
        if result:
            return result.splitlines()
    return git("rev-list", head).splitlines()


def main() -> int:
    base = sys.argv[1] if len(sys.argv) > 1 else ""
    head = sys.argv[2] if len(sys.argv) > 2 else "HEAD"
    failures: list[str] = []

    for commit in commit_range(base, head):
        metadata = git(
            "show",
            "--no-patch",
            "--format=%H%n%an <%ae>%n%cn <%ce>%n%B",
            commit,
        )
        if DISALLOWED.search(metadata):
            subject = git("show", "--no-patch", "--format=%h %s", commit).strip()
            failures.append(subject)

    if failures:
        print("Agent attribution is not allowed in commit metadata:", file=sys.stderr)
        for failure in failures:
            print(f"  - {failure}", file=sys.stderr)
        print(
            "Disable commit attribution in your client and recreate the commits.",
            file=sys.stderr,
        )
        return 1

    print("Commit attribution policy passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
