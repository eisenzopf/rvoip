#!/usr/bin/env python3
"""Capture selected, non-secret provenance from the running OpenSIPS TLS peer."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
from pathlib import Path


SCHEMA = "rvoip-sip-proxy-interop-opensips-tls-image-v1"
OUTPUT_BASENAME = "opensips-tls-image-provenance.json"
IMAGE_REFERENCE = "rvoip/opensips-tls-interop:3.6.7-1"
DOCKERFILE_RELATIVE_PATH = (
    "crates/sip/sip-proxy/tests/interop/images/opensips-tls/Dockerfile"
)
BASE_DIGEST = (
    "sha256:eba1396b438a7f8a9d33c17017aae4670cb43361eb7130359240cf85fc3e6979"
)
PACKAGES = {
    "opensips": "3.6.7-1",
    "opensips-tls-module": "3.6.7-1",
    "opensips-tlsmgm-module": "3.6.7-1",
    "opensips-tls-openssl-module": "3.6.7-1",
}
REVIEWED_DEBS = {
    "opensips-tls-module_3.6.7-1_amd64.deb": (
        "685f704faf2ce6b9015a0c059a32333bf789cabd8a467c7068aa1cea363de799"
    ),
    "opensips-tlsmgm-module_3.6.7-1_amd64.deb": (
        "20d83193399a9ee02b8c3f1cc2fbe311231ec22c0b43656add63f77110a68545"
    ),
    "opensips-tls-openssl-module_3.6.7-1_amd64.deb": (
        "690c52e06b9d0f8d76483900a4185f3a3d7cc3ec30a752ff762bdca254750209"
    ),
}
MODULES = {
    "/usr/lib/x86_64-linux-gnu/opensips/modules/proto_tls.so": (
        "c0a3b92a2d64fc58c8d015338f8016d8fe59e9cc131b91f2774f07281349f7f3"
    ),
    "/usr/lib/x86_64-linux-gnu/opensips/modules/tls_mgm.so": (
        "f2726bc731dbdf840bd40dbc7209eded8f16899034177b8dabed481da60662d2"
    ),
    "/usr/lib/x86_64-linux-gnu/opensips/modules/tls_openssl.so": (
        "77714d25e26933b4a18408b3ef65bad75400d5840d1bcf58efde99292e21ba6f"
    ),
}


class ProvenanceError(RuntimeError):
    pass


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def docker(
    executable: str, arguments: list[str], description: str
) -> str:
    completed = subprocess.run(
        [executable, *arguments],
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if completed.returncode != 0:
        raise ProvenanceError(
            f"{description} failed: {completed.stderr.strip()}"
        )
    return completed.stdout.strip()


def capture(
    container_id: str, dockerfile: Path, docker_executable: str = "docker"
) -> dict:
    if re.fullmatch(r"[0-9a-f]{12,64}", container_id) is None:
        raise ProvenanceError("container ID must be an exact hexadecimal ID")
    if (
        not dockerfile.is_file()
        or dockerfile.is_symlink()
        or dockerfile.stat().st_size <= 0
    ):
        raise ProvenanceError(f"Dockerfile is missing: {dockerfile}")
    if not dockerfile.as_posix().endswith(DOCKERFILE_RELATIVE_PATH):
        raise ProvenanceError(
            "Dockerfile path does not name the reviewed interop derivation"
        )
    dockerfile_text = dockerfile.read_text(
        encoding="utf-8", errors="strict"
    )
    if f"FROM opensips/opensips:3.6@{BASE_DIGEST}" not in dockerfile_text:
        raise ProvenanceError("Dockerfile does not pin the reviewed base digest")
    for package, digest in REVIEWED_DEBS.items():
        package_template = package.replace(
            "3.6.7-1", "${OPENSIPS_VERSION}"
        )
        if (
            package_template not in dockerfile_text
            or digest not in dockerfile_text
        ):
            raise ProvenanceError(
                f"Dockerfile omits reviewed package input {package}"
            )

    image_id = docker(
        docker_executable,
        ["inspect", "--format", "{{.Image}}", container_id],
        "running-container image lookup",
    )
    if re.fullmatch(r"sha256:[0-9a-f]{64}", image_id) is None:
        raise ProvenanceError("running container returned an invalid image ID")
    image_reference = docker(
        docker_executable,
        ["inspect", "--format", "{{.Config.Image}}", container_id],
        "running-container image reference lookup",
    )
    if image_reference != IMAGE_REFERENCE:
        raise ProvenanceError(
            "running container does not use the reviewed derived image reference"
        )
    image_metadata = docker(
        docker_executable,
        [
            "image",
            "inspect",
            "--format",
            (
                '{{.Architecture}}\t{{index .Config.Labels '
                '"org.opencontainers.image.base.digest"}}'
            ),
            image_id,
        ],
        "derived-image metadata lookup",
    ).split("\t")
    if image_metadata != ["amd64", BASE_DIGEST]:
        raise ProvenanceError(
            "derived image architecture/base digest does not match review"
        )

    package_output = docker(
        docker_executable,
        [
            "exec",
            container_id,
            "dpkg-query",
            "-W",
            "-f=${Package}\t${Version}\n",
            *PACKAGES,
        ],
        "installed OpenSIPS package query",
    )
    installed_packages: dict[str, str] = {}
    for line in package_output.splitlines():
        fields = line.split("\t")
        if len(fields) != 2:
            raise ProvenanceError(
                f"unexpected dpkg-query record: {line!r}"
            )
        installed_packages[fields[0]] = fields[1]
    if installed_packages != PACKAGES:
        raise ProvenanceError(
            "running OpenSIPS package versions do not match review"
        )

    module_output = docker(
        docker_executable,
        ["exec", container_id, "sha256sum", *MODULES],
        "installed OpenSIPS module hash query",
    )
    installed_modules: dict[str, str] = {}
    for line in module_output.splitlines():
        fields = line.split()
        if len(fields) != 2 or re.fullmatch(r"[0-9a-f]{64}", fields[0]) is None:
            raise ProvenanceError(
                f"unexpected module hash record: {line!r}"
            )
        installed_modules[fields[1]] = fields[0]
    if installed_modules != MODULES:
        raise ProvenanceError(
            "running OpenSIPS module hashes do not match reviewed packages"
        )

    return {
        "schema": SCHEMA,
        "result": "PASS",
        "image": {
            "reference": image_reference,
            "id": image_id,
            "platform": "linux/amd64",
            "base_digest": BASE_DIGEST,
        },
        "dockerfile": {
            "relative_path": DOCKERFILE_RELATIVE_PATH,
            "sha256": sha256_file(dockerfile),
        },
        "packages": {
            package: {
                "version": version,
                **(
                    {
                        "deb_sha256": REVIEWED_DEBS[
                            {
                                "opensips-tls-module": (
                                    "opensips-tls-module_3.6.7-1_amd64.deb"
                                ),
                                "opensips-tlsmgm-module": (
                                    "opensips-tlsmgm-module_3.6.7-1_amd64.deb"
                                ),
                                "opensips-tls-openssl-module": (
                                    "opensips-tls-openssl-module_3.6.7-1_amd64.deb"
                                ),
                            }[package]
                        ]
                    }
                    if package != "opensips"
                    else {}
                ),
            }
            for package, version in installed_packages.items()
        },
        "modules": {
            Path(path).name: {"path": path, "sha256": digest}
            for path, digest in installed_modules.items()
        },
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--container-id", required=True)
    parser.add_argument("--dockerfile", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument(
        "--docker", default=os.environ.get("DOCKER", "docker")
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    if args.output.name != OUTPUT_BASENAME:
        print(
            f"provenance output must be named {OUTPUT_BASENAME}",
            file=sys.stderr,
        )
        return 2
    try:
        result = capture(
            args.container_id, args.dockerfile.resolve(), args.docker
        )
    except (OSError, ProvenanceError) as error:
        print(f"OpenSIPS TLS provenance failed: {error}", file=sys.stderr)
        return 1
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(
        json.dumps(result, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
