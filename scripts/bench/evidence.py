#!/usr/bin/env python3
"""Evidence helpers: sha256 sidecars and manifests, tar archives, layout names.

Ported in spirit from the E74 harness (evidence_paths.py and the archive
handling of its runner). The RFC7502_BENCHMARK.md §5 layout:

    <lane>/<row>/t<k>-r<rate>/dut.tar.gz + dut.tar.gz.sha256
    <lane>/<row>/t<k>-r<rate>/caller-<i>.tar.gz + .sha256

usage: evidence.py sha256 <file>...              write <file>.sha256 sidecars
       evidence.py verify <file>...              check the sidecars (exit 1 on mismatch)
       evidence.py manifest <dir> [--out PATH]   sha256 manifest of a directory tree
       evidence.py pack <dir> <out.tar.gz>       deterministic archive (+ .sha256)
       evidence.py unpack <in.tar.gz> <dir>      safe extraction (sidecar verified when present)
Standard library only.
"""
import argparse
import gzip
import hashlib
import json
from pathlib import Path
import re
import sys
import tarfile

CHUNK = 1024 * 1024
TRIAL_DIR = re.compile(r'^t(\d+)-r(\d+)$')


# ---- layout

def trial_dir_name(index, rate):
    """`t<k>-r<rate>` (k zero-padded to two digits)."""
    return 't%02d-r%d' % (int(index), int(rate))


def parse_trial_dir_name(name):
    """(index, rate) of a trial directory name, or None."""
    match = TRIAL_DIR.match(Path(name).name)
    return (int(match.group(1)), int(match.group(2))) if match else None


def trial_path(row, index, rate):
    """`<row>/t<k>-r<rate>` relative to the lane evidence root."""
    return '%s/%s' % (row, trial_dir_name(index, rate))


# ---- sha256

def sha256_file(path):
    digest = hashlib.sha256()
    with open(path, 'rb') as stream:
        for chunk in iter(lambda: stream.read(CHUNK), b''):
            digest.update(chunk)
    return digest.hexdigest()


def write_sha256(path):
    """Write `<path>.sha256` in sha256sum format; returns the digest."""
    path = Path(path)
    digest = sha256_file(path)
    Path(str(path) + '.sha256').write_text('%s  %s\n' % (digest, path.name))
    return digest


def read_sha256(sidecar):
    text = Path(sidecar).read_text().split()
    if not text or not re.fullmatch(r'[0-9a-f]{64}', text[0]):
        raise ValueError('not a sha256 sidecar: %s' % sidecar)
    return text[0]


def verify_sha256(path):
    """True when `<path>.sha256` exists and matches `path`."""
    sidecar = Path(str(path) + '.sha256')
    if not sidecar.exists():
        return False
    return read_sha256(sidecar) == sha256_file(path)


def manifest(directory):
    """{relative path: {sha256, bytes}} of every regular file under `directory` (symlinks skipped)."""
    directory = Path(directory)
    out = {}
    for path in sorted(directory.rglob('*')):
        if path.is_symlink() or not path.is_file():
            continue
        out[path.relative_to(directory).as_posix()] = {'sha256': sha256_file(path), 'bytes': path.stat().st_size}
    return out


def write_manifest(directory, out=None):
    out = Path(out) if out else Path(directory) / 'manifest.json'
    rows = {k: v for k, v in manifest(directory).items() if Path(directory) / k != out}
    out.write_text(json.dumps(rows, indent=1, sort_keys=True) + '\n')
    return out


def verify_manifest(directory, rows):
    """Paths whose content differs from (or is missing against) a manifest."""
    directory = Path(directory)
    bad = []
    for name, row in rows.items():
        path = directory / name
        if not path.is_file() or sha256_file(path) != row['sha256']:
            bad.append(name)
    return bad


# ---- archives

def _normalize(info):
    info.uid = info.gid = 0
    info.uname = info.gname = ''
    return info


def pack(directory, out, arcname=None):
    """Deterministic tar.gz of `directory` (sorted, owner-free, gzip mtime 0) plus its .sha256; returns the digest."""
    directory, out = Path(directory), Path(out)
    arcname = arcname if arcname is not None else directory.name
    buffer_path = Path(str(out) + '.partial')
    with open(buffer_path, 'wb') as raw:
        with gzip.GzipFile(filename='', mode='wb', fileobj=raw, mtime=0) as gz:
            with tarfile.open(fileobj=gz, mode='w') as tar:
                paths = [directory] + sorted(directory.rglob('*'))
                for path in paths:
                    name = arcname if path == directory else '%s/%s' % (arcname, path.relative_to(directory).as_posix())
                    name = name.lstrip('/') or '.'
                    tar.add(str(path), arcname=name, recursive=False, filter=_normalize)
    buffer_path.replace(out)
    return write_sha256(out)


def _safe_members(tar, dest):
    dest = Path(dest).resolve()
    for member in tar.getmembers():
        target = (dest / member.name).resolve()
        if member.name.startswith('/') or (target != dest and dest not in target.parents):
            raise ValueError('unsafe path in archive: %s' % member.name)
        if member.issym() or member.islnk():
            link = (target.parent / member.linkname).resolve()
            if member.linkname.startswith('/') or (link != dest and dest not in link.parents):
                raise ValueError('unsafe link in archive: %s -> %s' % (member.name, member.linkname))
        if member.isdev():
            raise ValueError('device file in archive: %s' % member.name)
        yield member


def unpack(archive, dest, require_sidecar=False):
    """Extract `archive` under `dest` refusing absolute paths, '..' and links that escape; verifies the
    .sha256 sidecar when present (raises ValueError on a mismatch, or when required and absent)."""
    archive, dest = Path(archive), Path(dest)
    sidecar = Path(str(archive) + '.sha256')
    if sidecar.exists():
        if read_sha256(sidecar) != sha256_file(archive):
            raise ValueError('sha256 mismatch: %s' % archive)
    elif require_sidecar:
        raise ValueError('missing sidecar: %s' % sidecar)
    dest.mkdir(parents=True, exist_ok=True)
    with tarfile.open(archive, 'r:*') as tar:
        members = list(_safe_members(tar, dest))
        if hasattr(tarfile, 'data_filter'):
            tar.extractall(dest, members=members, filter='data')
        else:   # pragma: no cover - Python < 3.10.12 without extraction filters
            tar.extractall(dest, members=members)
    return dest


def main(argv=None):
    parser = argparse.ArgumentParser(description='Evidence sha256 sidecars, manifests and archives.')
    sub = parser.add_subparsers(dest='command', required=True)
    p = sub.add_parser('sha256'); p.add_argument('files', nargs='+')
    p = sub.add_parser('verify'); p.add_argument('files', nargs='+')
    p = sub.add_parser('manifest'); p.add_argument('directory'); p.add_argument('--out')
    p = sub.add_parser('pack'); p.add_argument('directory'); p.add_argument('out'); p.add_argument('--arcname')
    p = sub.add_parser('unpack'); p.add_argument('archive'); p.add_argument('dest'); p.add_argument('--require-sidecar', action='store_true')
    args = parser.parse_args(argv)
    if args.command == 'sha256':
        for name in args.files:
            print(write_sha256(name), name)
        return 0
    if args.command == 'verify':
        bad = [name for name in args.files if not verify_sha256(name)]
        print(json.dumps({'verified': len(args.files) - len(bad), 'failed': bad}))
        return 1 if bad else 0
    if args.command == 'manifest':
        print(write_manifest(args.directory, args.out))
        return 0
    if args.command == 'pack':
        print(pack(args.directory, args.out, args.arcname), args.out)
        return 0
    unpack(args.archive, args.dest, args.require_sidecar)
    print(args.dest)
    return 0


if __name__ == '__main__':
    sys.exit(main())
