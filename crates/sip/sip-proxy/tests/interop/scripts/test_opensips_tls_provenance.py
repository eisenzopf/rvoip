#!/usr/bin/env python3
"""Fail-closed tests for OpenSIPS TLS derived-image provenance."""

from __future__ import annotations

import importlib.util
import shutil
import tempfile
import unittest
from pathlib import Path


SCRIPT_DIR = Path(__file__).resolve().parent
SOURCE_DOCKERFILE = (
    SCRIPT_DIR.parent / "images/opensips-tls/Dockerfile"
)
MODULE_SPEC = importlib.util.spec_from_file_location(
    "opensips_tls_provenance",
    SCRIPT_DIR / "opensips_tls_provenance.py",
)
assert MODULE_SPEC and MODULE_SPEC.loader
provenance = importlib.util.module_from_spec(MODULE_SPEC)
MODULE_SPEC.loader.exec_module(provenance)


class OpenSipsTlsProvenanceTests(unittest.TestCase):
    def setUp(self) -> None:
        self._temporary = tempfile.TemporaryDirectory()
        self.root = Path(self._temporary.name)
        self.dockerfile = self.root / provenance.DOCKERFILE_RELATIVE_PATH
        self.dockerfile.parent.mkdir(parents=True)
        shutil.copyfile(SOURCE_DOCKERFILE, self.dockerfile)
        self.state = {
            "image_id": f"sha256:{'1' * 64}",
            "reference": provenance.IMAGE_REFERENCE,
            "architecture": "amd64",
            "base": provenance.BASE_DIGEST,
            "packages": dict(provenance.PACKAGES),
            "modules": dict(provenance.MODULES),
        }
        self.original_docker = provenance.docker
        provenance.docker = self.fake_docker

    def tearDown(self) -> None:
        provenance.docker = self.original_docker
        self._temporary.cleanup()

    def fake_docker(
        self, _executable: str, arguments: list[str], description: str
    ) -> str:
        if description == "running-container image lookup":
            return self.state["image_id"]
        if description == "running-container image reference lookup":
            return self.state["reference"]
        if description == "derived-image metadata lookup":
            return f"{self.state['architecture']}\t{self.state['base']}"
        if description == "installed OpenSIPS package query":
            return "\n".join(
                f"{package}\t{version}"
                for package, version in self.state["packages"].items()
            )
        if description == "installed OpenSIPS module hash query":
            return "\n".join(
                f"{digest}  {path}"
                for path, digest in self.state["modules"].items()
            )
        raise AssertionError(f"unexpected Docker command: {arguments!r}")

    def capture(self) -> dict:
        return provenance.capture("a" * 64, self.dockerfile, "docker")

    def test_valid_mocked_runtime_is_captured(self) -> None:
        result = self.capture()
        self.assertEqual(result["schema"], provenance.SCHEMA)
        self.assertEqual(result["result"], "PASS")
        self.assertEqual(
            result["image"]["base_digest"], provenance.BASE_DIGEST
        )
        self.assertEqual(set(result["modules"]), {
            "proto_tls.so",
            "tls_mgm.so",
            "tls_openssl.so",
        })

    def test_wrong_reference_base_or_architecture_fails(self) -> None:
        mutations = (
            ("reference", "unreviewed/image:latest"),
            ("base", f"sha256:{'2' * 64}"),
            ("architecture", "arm64"),
        )
        for key, value in mutations:
            with self.subTest(key=key):
                original = self.state[key]
                self.state[key] = value
                with self.assertRaises(provenance.ProvenanceError):
                    self.capture()
                self.state[key] = original

    def test_wrong_package_version_fails(self) -> None:
        self.state["packages"]["opensips-tls-module"] = "3.6.8-1"
        with self.assertRaisesRegex(
            provenance.ProvenanceError, "package versions"
        ):
            self.capture()

    def test_wrong_installed_module_hash_fails(self) -> None:
        module = next(iter(self.state["modules"]))
        self.state["modules"][module] = "f" * 64
        with self.assertRaisesRegex(
            provenance.ProvenanceError, "module hashes"
        ):
            self.capture()

    def test_wrong_reviewed_deb_hash_fails(self) -> None:
        text = self.dockerfile.read_text()
        digest = next(iter(provenance.REVIEWED_DEBS.values()))
        self.dockerfile.write_text(text.replace(digest, "0" * 64))
        with self.assertRaisesRegex(
            provenance.ProvenanceError, "omits reviewed package"
        ):
            self.capture()

    def test_invalid_container_id_fails(self) -> None:
        with self.assertRaisesRegex(
            provenance.ProvenanceError, "container ID"
        ):
            provenance.capture("not-a-container", self.dockerfile, "docker")

    def test_symlink_dockerfile_fails(self) -> None:
        real = self.root / "real-Dockerfile"
        self.dockerfile.replace(real)
        self.dockerfile.symlink_to(real)
        with self.assertRaisesRegex(
            provenance.ProvenanceError, "Dockerfile is missing"
        ):
            self.capture()


if __name__ == "__main__":
    unittest.main()
