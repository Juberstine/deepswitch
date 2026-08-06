from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path


SCRIPT = Path(__file__).parents[1] / "scripts" / "render-homebrew-formula.py"
SPEC = importlib.util.spec_from_file_location("render_homebrew_formula", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
formula = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(formula)


def checksum_manifest() -> str:
    return "\n".join(
        f"{index:064x}  ./{asset}"
        for index, asset in enumerate(formula.FORMULA_ASSETS.values(), start=1)
    )


class ParseChecksumsTests(unittest.TestCase):
    def test_parses_release_manifest(self) -> None:
        checksums = formula.parse_checksums(checksum_manifest())

        self.assertEqual(
            checksums["deepswitch-linux-aarch64.tar.gz"],
            f"{1:064x}",
        )

    def test_rejects_missing_formula_asset(self) -> None:
        with self.assertRaisesRegex(ValueError, "missing checksums"):
            formula.parse_checksums(checksum_manifest().splitlines()[0])

    def test_rejects_invalid_checksum(self) -> None:
        manifest = checksum_manifest().replace(f"{1:064x}", "not-a-checksum", 1)

        with self.assertRaisesRegex(ValueError, "invalid SHA-256"):
            formula.parse_checksums(manifest)


class RenderFormulaTests(unittest.TestCase):
    def test_renders_all_supported_brew_platforms(self) -> None:
        checksums = formula.parse_checksums(checksum_manifest())

        rendered = formula.render_formula("1.2.3", checksums)

        self.assertIn('version "1.2.3"', rendered)
        self.assertIn("on_macos do", rendered)
        self.assertIn("on_linux do", rendered)
        for asset in formula.FORMULA_ASSETS.values():
            self.assertIn(
                f"releases/download/v1.2.3/{asset}",
                rendered,
            )
        self.assertIn(
                'shell_output("#{bin}/deepswitch --version")',
            rendered,
        )

    def test_rejects_invalid_version(self) -> None:
        checksums = formula.parse_checksums(checksum_manifest())

        with self.assertRaisesRegex(ValueError, "invalid release version"):
            formula.render_formula("../../latest", checksums)


if __name__ == "__main__":
    unittest.main()
