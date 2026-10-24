"""No real credentials: construct scanner probes only in memory."""
import importlib.util
from pathlib import Path
import unittest
import subprocess
import tempfile
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('public_scan', Path(__file__).resolve().parents[1] / 'scripts/check-public.py')
scan = importlib.util.module_from_spec(spec)
spec.loader.exec_module(scan)


class PublicExportTests(unittest.TestCase):
    def test_runtime_files_are_rejected_even_without_secret_text(self):
        for name in ('auth.json', 'private/operator.key', 'workboard.sqlite', '.env',
                     '.env.local', 'action/result.jsonl', 'runtime/plain.txt', 'state/data.json',
                     'auth-reference.json', 'native-receipt.json', '.venv/metadata.txt'):
            with self.subTest(name=name):
                self.assertIn('private-runtime-file', scan.inspect(name, b'{}'))

    def test_safe_configuration_examples_are_allowed(self):
        self.assertEqual(scan.inspect('.env.example', b'GH_TOKEN=YOUR_TOKEN_HERE\n'), [])
        self.assertEqual(scan.inspect('src/operator_auth.rs', b'let key = env("GH_TOKEN");'), [])

    def test_known_token_families_are_detected_without_echoing_values(self):
        probes = [('github-token', 'gh' + 'p_' + 'A' * 36),
                  ('provider-key', 'sk' + '-' + 'a' * 40),
                  ('slack-token', 'xo' + 'xb-' + '1' * 32),
                  ('private-key', '-----BEGIN ' + 'PRIVATE KEY-----')]
        for rule, value in probes:
            hits = scan.inspect('example.txt', value.encode())
            self.assertIn(rule, hits)
            self.assertNotIn(value, str(hits))

    def test_symlinks_cannot_smuggle_external_files(self):
        self.assertIn('symlink-not-exportable', scan.inspect('source.rs', b'', True))

    def test_credentials_in_urls_are_rejected(self):
        value = 'https://' + 'name:pass' + '@example.invalid/repo'
        self.assertIn('credential-url', scan.inspect('file.txt', value.encode()))

    def test_binary_requires_explicit_review(self):
        self.assertIn('binary-file-review-required', scan.inspect('file.bin', bytes([255])))


    def test_forced_staged_ignored_credentials_are_still_candidates(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            subprocess.run(['git', 'init', '-q'], cwd=root, check=True)
            (root / '.gitignore').write_text('auth.json\n')
            (root / 'auth.json').write_text('{}')
            subprocess.run(['git', 'add', '-f', 'auth.json'], cwd=root, check=True)
            with patch.object(scan, 'ROOT', root):
                files = list(scan.candidates())
            self.assertTrue(any(path == 'auth.json' and 'private-runtime-file' in scan.inspect(path, data, link)
                                for path, data, link in files))

    def test_history_detects_a_removed_credential_without_echoing_value(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            subprocess.run(['git', 'init', '-q'], cwd=root, check=True)
            fake = 'gh' + 'p_' + 'Z' * 36
            (root / 'sample.txt').write_text(fake)
            subprocess.run(['git', 'add', 'sample.txt'], cwd=root, check=True)
            commit = ['git', '-c', 'user.name=Synthetic', '-c', 'user.email=fixture@example.invalid',
                      '-c', 'core.hooksPath=/dev/null', 'commit', '-qm']
            subprocess.run([*commit, 'synthetic initial'], cwd=root, check=True)
            (root / 'sample.txt').write_text('safe example')
            subprocess.run(['git', 'add', 'sample.txt'], cwd=root, check=True)
            subprocess.run([*commit, 'remove synthetic probe'], cwd=root, check=True)
            with patch.object(scan, 'ROOT', root):
                findings = list(scan.history())
            self.assertTrue(any('github-token' in hits for _, hits in findings))
            self.assertNotIn(fake, str(findings))


if __name__ == '__main__':
    unittest.main()
