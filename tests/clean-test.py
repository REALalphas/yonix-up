"""Exercise the generated cleaner using fakes; tmpfiles only touches a fixture root."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest

CLEAN, MINIMAL, CUSTOM, TMPFILES = sys.argv[1:]
sys.argv[1:] = []
VERIFY = 'nix <--extra-experimental-features> <nix-command> <store> <verify> <--all> <--no-trust>'
TMP = ('systemd-tmpfiles <--clean> <--prefix=/tmp> <--prefix=/var/tmp> '
       '<--prefix=/var/cache> <--prefix=/var/lib/systemd/coredump>')
EXPECTED = [
    'df <-h> <-l>', 'df <-i> <-l>', 'maintenance <report>', TMP,
    'journalctl <--rotate>', 'journalctl <--vacuum-time=14d> <--vacuum-size=512M>',
    'journalctl <--disk-usage>', 'nix-collect-garbage <--delete-older-than> <14d>',
    VERIFY, 'nix-store <--optimise>', 'maintenance <clean>',
    'fstrim <--fstab> <--verbose> <--quiet-unsupported>',
    'systemctl <--failed> <--no-legend> <--plain> <--no-pager>',
    'df <-h> <-l>', 'df <-i> <-l>',
]


class CleanerTest(unittest.TestCase):
    def invoke(self, command=CLEAN, **settings):
        with tempfile.TemporaryDirectory() as directory:
            calls = Path(directory) / 'calls'
            result = subprocess.run([command], env={**os.environ, 'TEST_CALLS': str(calls),
                                    **settings}, text=True, capture_output=True)
            return result, calls.read_text().splitlines()

    def test_complete_order_and_fixed_arguments(self):
        result, calls = self.invoke()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, EXPECTED)
        self.assertIn('13 completed, 0 failed, 0 skipped', result.stdout)

    def test_each_failure_is_visible_and_independent_steps_continue(self):
        for command in dict.fromkeys(EXPECTED):
            if command.startswith('maintenance'):
                settings = {'TEST_REPORT_STATUS' if command.endswith('<report>')
                            else 'TEST_MAINTENANCE_STATUS': '7'}
            else:
                settings = {'TEST_FAIL_COMMAND': command.replace(' <', ' ').replace('>', '')}
            with self.subTest(command=command):
                result, calls = self.invoke(**settings)
                self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
                self.assertIn('FAILED:', result.stderr)
                self.assertIn('Yonix clean summary:', result.stdout)
                self.assertEqual(calls[-1], 'df <-h> <-l>' if command == 'df <-h> <-l>'
                                 else 'df <-i> <-l>')
                self.assertIn('maintenance <clean>', calls)
                self.assertIn('fstrim <--fstab> <--verbose> <--quiet-unsupported>', calls)
                if command == 'journalctl <--rotate>':
                    self.assertNotIn(EXPECTED[5], calls)
                if command == VERIFY:
                    self.assertNotIn('nix-store <--optimise>', calls)
                    self.assertIn('integrity check failed', result.stdout)

    def test_failed_services_are_not_reset(self):
        result, calls = self.invoke(TEST_FAILED_UNITS='example.service loaded failed failed\n')
        self.assertEqual(result.returncode, 1)
        self.assertIn('example.service', result.stdout)
        self.assertEqual([c for c in calls if c.startswith('systemctl')], [EXPECTED[12]])

    def test_optional_expensive_operations(self):
        result, calls = self.invoke(MINIMAL)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn(VERIFY, calls)
        self.assertNotIn('nix-store <--optimise>', calls)
        self.assertFalse(any(c.startswith('fstrim') for c in calls))
        self.assertIn('3 skipped', result.stdout)

    def test_retention_configuration(self):
        result, calls = self.invoke(CUSTOM)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('journalctl <--vacuum-time=30d> <--vacuum-size=128M>', calls)
        self.assertIn('nix-collect-garbage <--delete-older-than> <7d>', calls)

    def test_real_tmpfiles_respects_age_exclusions_and_personal_data(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            config = root / 'etc/tmpfiles.d/test.conf'
            config.parent.mkdir(parents=True)
            paths = ['tmp/expired', 'var/tmp/expired', 'var/cache/expired',
                     'var/lib/systemd/coredump', 'home/user', 'var/lib/app', 'tmp/preserved']
            for path in paths:
                folder = root / path
                folder.mkdir(parents=True)
                for name in ['old', 'recent']:
                    file = folder / name
                    file.write_text('fixture')
                    if name == 'old':
                        os.utime(file, (time.time() - 3 * 86400,) * 2)
            (root / 'tmp/expired/link').symlink_to(root / 'home/user', target_is_directory=True)
            config.write_text('d /tmp 0700 - - m:1d\n'
                              + ''.join(f'd /{p} 0700 - - m:1d\n' for p in paths[:-1])
                              + 'x /tmp/preserved - - - -\n')
            result = subprocess.run([TMPFILES, '--root=' + directory, '--clean',
                                     '--prefix=/tmp', '--prefix=/var/tmp', '--prefix=/var/cache',
                                     '--prefix=/var/lib/systemd/coredump', 'test.conf'],
                                    text=True, capture_output=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            for path in paths:
                self.assertTrue((root / path / 'recent').is_file(), path)
                self.assertEqual((root / path / 'old').exists(), path in paths[4:], path)


if __name__ == '__main__':
    unittest.main()
