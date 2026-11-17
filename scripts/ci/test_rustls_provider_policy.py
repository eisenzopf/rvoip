#!/usr/bin/env python3
"""Policy: library code names its rustls CryptoProvider explicitly.

rustls cannot infer a process-level provider when a dependency graph compiles
both of its providers (`ring` and `aws-lc-rs`), so library `src/` code must:

- build every `ClientConfig`/`ServerConfig` and WebPKI verifier with an explicit
  provider (`builder_with_provider`), never the implicit `builder()`;
- never call `install_default()`, except inside an
  `ensure_process_crypto_provider` helper, which installs ring only when no
  provider is installed. Those helpers guard third-party paths (dtls, redis,
  sqlx) that can only use the process default.

Test code (`#[cfg(test)]` items and files) is exempt.
"""

from __future__ import annotations

import re
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
HELPER = "ensure_process_crypto_provider"
INSTALL = re.compile(r"\.install_default\(\)")
IMPLICIT = re.compile(
    r"\b(?:ClientConfig|ServerConfig)::builder\(\)"
    r"|\b(?:WebPkiServerVerifier|WebPkiClientVerifier)::builder\("
)
CFG_TEST = re.compile(r"#\[cfg\(test\)\]")
TEST_MOD_DECL = re.compile(r"#\[cfg\(test\)\]\s*(?:#\[[^\]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?mod\s+(\w+)\s*;")
HELPER_FN = re.compile(r"\bfn\s+" + HELPER + r"\s*\(")


def mask(source: str) -> str:
    """Blank out comments and string/char literal contents, keeping offsets."""
    out = list(source)
    i, n = 0, len(source)

    def blank(a: int, b: int) -> None:
        for k in range(a, b):
            if out[k] != "\n":
                out[k] = " "

    while i < n:
        c = source[i]
        if source.startswith("//", i):
            j = source.find("\n", i)
            j = n if j < 0 else j
            blank(i, j)
            i = j
        elif source.startswith("/*", i):
            depth, j = 1, i + 2
            while j < n and depth:
                if source.startswith("/*", j):
                    depth, j = depth + 1, j + 2
                elif source.startswith("*/", j):
                    depth, j = depth - 1, j + 2
                else:
                    j += 1
            blank(i, j)
            i = j
        elif re.match(r'(?:b|c)?r#*"', source[i:i + 12]) and (i == 0 or not (source[i - 1].isalnum() or source[i - 1] == "_")):
            m = re.match(r'(?:b|c)?r(#*)"', source[i:])
            end = '"' + m.group(1)
            j = source.find(end, i + m.end())
            j = n if j < 0 else j + len(end)
            blank(i + m.end(), j - len(end))
            i = j
        elif c == '"':
            j = i + 1
            while j < n and source[j] != '"':
                j += 2 if source[j] == "\\" else 1
            blank(i + 1, j)
            i = j + 1
        elif c == "'":
            m = re.match(r"'(?:\\.|\\u\{[0-9a-fA-F]+\}|[^'\\])'", source[i:])
            if m:
                blank(i + 1, i + m.end() - 1)
                i += m.end()
            else:
                i += 1  # lifetime
        else:
            i += 1
    return "".join(out)


def block_end(masked: str, start: int) -> int:
    """Offset just past the `{...}` block whose `{` is the first at/after start,
    or the end of the item if a `;` comes first."""
    i = start
    while i < len(masked) and masked[i] not in "{;":
        i += 1
    if i >= len(masked) or masked[i] == ";":
        return i + 1
    depth = 0
    while i < len(masked):
        if masked[i] == "{":
            depth += 1
        elif masked[i] == "}":
            depth -= 1
            if depth == 0:
                return i + 1
        i += 1
    return len(masked)


def exempt_ranges(masked: str) -> list[tuple[int, int]]:
    ranges = [(m.start(), block_end(masked, m.end())) for m in CFG_TEST.finditer(masked)]
    ranges += [(m.start(), block_end(masked, m.end())) for m in HELPER_FN.finditer(masked)]
    return ranges


def violations(source: str) -> list[tuple[int, str]]:
    masked = mask(source)
    test_ranges = [(m.start(), block_end(masked, m.end())) for m in CFG_TEST.finditer(masked)]
    helper_ranges = [(m.start(), block_end(masked, m.end())) for m in HELPER_FN.finditer(masked)]
    found = []
    for pattern, allowed, what in (
        (INSTALL, test_ranges + helper_ranges, "install_default() outside ensure_process_crypto_provider"),
        (IMPLICIT, test_ranges, "implicit-provider rustls builder; use builder_with_provider"),
    ):
        for m in pattern.finditer(masked):
            if not any(a <= m.start() < b for a, b in allowed):
                found.append((masked.count("\n", 0, m.start()) + 1, what))
    for a, b in helper_ranges:
        body = masked[a:b]
        if "get_default().is_none()" not in source[a:b].replace("\n", "").replace(" ", "") and "get_default().is_none()" not in body.replace(" ", ""):
            found.append((masked.count("\n", 0, a) + 1, "ensure_process_crypto_provider must install only when get_default() is None"))
    return found


def test_files(paths: list[Path]) -> set[Path]:
    """Files that are whole `#[cfg(test)] mod x;` modules."""
    exempt = set()
    for path in paths:
        masked = mask(path.read_text())
        for m in TEST_MOD_DECL.finditer(masked):
            name = m.group(1)
            base = path.parent if path.name in ("lib.rs", "main.rs", "mod.rs") else path.with_suffix("")
            exempt.update({base / f"{name}.rs", base / name / "mod.rs"})
    return exempt


def library_sources() -> list[Path]:
    return sorted(p for p in (ROOT / "crates").glob("**/src/**/*.rs") if "/target/" not in str(p))


def scan() -> list[str]:
    paths = library_sources()
    skip = test_files(paths)
    report = []
    for path in paths:
        if path in skip:
            continue
        for line, what in violations(path.read_text()):
            report.append(f"{path.relative_to(ROOT)}:{line}: {what}")
    return report


class ScannerTests(unittest.TestCase):
    def test_flags_install_default_in_library_code(self):
        src = "fn setup() {\n    let _ = rustls::crypto::ring::default_provider().install_default();\n}\n"
        self.assertEqual([line for line, _ in violations(src)], [2])

    def test_allows_install_default_inside_the_guarded_helper(self):
        src = ("pub(crate) fn ensure_process_crypto_provider() {\n"
               "    if rustls::crypto::CryptoProvider::get_default().is_none() {\n"
               "        let _ = rustls::crypto::ring::default_provider().install_default();\n"
               "    }\n}\n")
        self.assertEqual(violations(src), [])

    def test_rejects_an_unguarded_helper(self):
        src = ("fn ensure_process_crypto_provider() {\n"
               "    let _ = rustls::crypto::ring::default_provider().install_default();\n}\n")
        self.assertEqual(len(violations(src)), 1)

    def test_flags_implicit_builders(self):
        src = ("fn a() { let c = rustls::ClientConfig::builder(); }\n"
               "fn b() { let s = ServerConfig::builder(); }\n"
               "fn c() { WebPkiClientVerifier::builder(roots); }\n"
               "fn d() { ClientConfig::builder_with_provider(p); }\n")
        self.assertEqual([line for line, _ in violations(src)], [1, 2, 3])

    def test_ignores_test_modules_comments_and_strings(self):
        src = ("// ClientConfig::builder()\n"
               "const S: &str = \"x.install_default() {\";\n"
               "#[cfg(test)]\nmod tests {\n    fn t() { let _ = p.install_default(); let c = ClientConfig::builder(); }\n}\n"
               "fn after() { let c = ClientConfig::builder(); }\n")
        self.assertEqual([line for line, _ in violations(src)], [7])


class WorkspacePolicyTests(unittest.TestCase):
    def test_library_sources_name_their_rustls_provider(self):
        report = scan()
        self.assertEqual(report, [], "\n" + "\n".join(report))


if __name__ == "__main__":
    unittest.main()
