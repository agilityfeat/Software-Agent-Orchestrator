#!/usr/bin/env python3
"""Review candidate files/history for accidental credentials; never echo values."""
import argparse
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
RULES = {
    'private-key': re.compile(rb'-----BEGIN (?:RSA |EC |OPENSSH |DSA )?PRIVATE KEY'),
    'github-token': re.compile(rb'\b(?:gh[pousr]_[A-Za-z0-9_]{20,}|github_pat_[A-Za-z0-9_]{20,})'),
    'provider-key': re.compile(rb'\bsk-[A-Za-z0-9_-]{20,}'),
    'slack-token': re.compile(rb'\bxox[baprs]-[A-Za-z0-9-]{15,}'),
    'aws-key': re.compile(rb'\bAKIA[A-Z0-9]{16}\b'),
    'jwt': re.compile(rb'\beyJ[A-Za-z0-9_-]{15,}\.[A-Za-z0-9_-]{15,}\.[A-Za-z0-9_-]{15,}'),
    'credential-url': re.compile(rb'https?://[^\s/:]+:[^\s/@]+@'),
    'personal-home': re.compile(rb'/(?:Users|home)/[A-Za-z0-9._-]+/'),
    'private-network': re.compile(rb'\b(?:100\.(?:6[4-9]|[789][0-9]|1[01][0-9]|12[0-7])\.\d{1,3}\.\d{1,3})\b'),
}
PRIVATE_NAMES = {
    'auth.json', 'launch.json', 'setup.json', 'workspace-catalog.json',
    'build-receipt.json', '.env', '.ds_store', 'auth-reference.json',
    'auth-recovery.json', 'native-job.json', 'native-receipt.json',
    'native-failure.json', 'validation-status.json', 'native-validation.json',
}
PRIVATE_SUFFIXES = ('.sqlite', '.sqlite-wal', '.sqlite-shm', '.db', '.db-wal',
                    '.db-shm', '.key', '.pem', '.log', '.jsonl', '.tgz', '.zip')
PRIVATE_DIRS = {'.local', '.state', 'runtime', 'state', 'node_modules', 'target',
                '.codex', '.openclaw', '.aws', '.venv', '.pytest_cache'}


def inspect(path, data, symlink=False):
    """Return finite rule names only. Synthetic test credentials remain obvious text."""
    p = pathlib.PurePosixPath(path)
    findings = []
    name = p.name.lower()
    if (name in PRIVATE_NAMES or name.endswith(PRIVATE_SUFFIXES)
            or (name.startswith('.env.') and name != '.env.example')
            or any(part.lower() in PRIVATE_DIRS for part in p.parts)):
        findings.append('private-runtime-file')
    if symlink:
        findings.append('symlink-not-exportable')
    if len(data) > 5 * 1024 * 1024:
        findings.append('oversized-file-review-required')
    try:
        data.decode('utf-8')
    except UnicodeDecodeError:
        findings.append('binary-file-review-required')
    findings.extend(name for name, pattern in RULES.items() if pattern.search(data))
    return findings


def git(*args):
    return subprocess.check_output(['git', *args], cwd=ROOT, stderr=subprocess.DEVNULL)


def candidates():
    paths = git('ls-files', '--cached', '--others', '--exclude-standard', '-z')
    for raw in sorted(set(paths.split(b'\0')) - {b''}):
        path = raw.decode('utf-8')
        p = ROOT / path
        # A tracked deleted file is absent from the next commit.
        if p.is_symlink():
            yield path, b'', True
        elif p.exists():
            yield path, p.read_bytes(), False


def history():
    seen = set()
    for commit in git('rev-list', '--all').decode().splitlines():
        for entry in git('ls-tree', '-r', '-z', commit).split(b'\0'):
            if not entry:
                continue
            meta, raw = entry.split(b'\t', 1)
            mode, kind, oid = meta.split()
            path = raw.decode('utf-8')
            key = (path, oid)
            if key in seen:
                continue
            seen.add(key)
            data = git('cat-file', 'blob', oid.decode()) if kind == b'blob' else b''
            yield f'{commit[:12]}:{path}', inspect(path, data, mode == b'120000' or kind != b'blob')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--history', action='store_true', help='also inspect every local Git ref')
    args = parser.parse_args()
    checked = 0
    failures = []
    try:
        for path, data, link in candidates():
            checked += 1
            hits = inspect(path, data, link)
            if hits:
                failures.append((path, hits))
        if args.history:
            for path, hits in history():
                checked += 1
                if hits:
                    failures.append((path, hits))
    except (OSError, ValueError, subprocess.CalledProcessError):
        print('Public export scan could not complete; publication must wait.', file=sys.stderr)
        return 2
    for path, hits in failures:
        print(f'{path}: {", ".join(hits)}', file=sys.stderr)
    print(f'Public export scan: {checked} file versions checked; {len(failures)} findings.')
    if not failures:
        print('No known credential/private-context patterns found. Review the full diff too.')
    return int(bool(failures))


if __name__ == '__main__':
    raise SystemExit(main())
