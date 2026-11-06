"""Deployment/architecture gate regressions; no Mac executable is run."""
import unittest
from pathlib import Path
from unittest.mock import patch
from check_macos_compat import check, minimum_for_arch


class CompatibilityTests(unittest.TestCase):
    def output(self, arch, load):
        return patch('check_macos_compat.subprocess.check_output', side_effect=[arch, load])

    def test_package_minimum_matches_architecture(self):
        root = Path(__file__).resolve().parent.parent
        self.assertEqual(minimum_for_arch(root, 'arm64'), '12.0')
        self.assertEqual(minimum_for_arch(root, 'x86_64'), '11.0')
        with self.output('arm64', 'LC_BUILD_VERSION\n minos 12.0'):
            check(Path('ffmpeg'), 'arm64', minimum_for_arch(root, 'arm64'))
        with self.output('arm64', 'LC_BUILD_VERSION\n minos 13.0'):
            with self.assertRaisesRegex(ValueError, '13.0'):
                check(Path('ffmpeg'), 'arm64', minimum_for_arch(root, 'arm64'))

    def test_big_sur_and_older_targets_pass(self):
        for arch, command in [('arm64', 'LC_BUILD_VERSION\n minos 11.0'),
                              ('x86_64', 'LC_VERSION_MIN_MACOSX\n version 10.13')]:
            with self.output(arch, command):
                check(Path('ffmpeg'), arch)

    def test_newer_target_is_rejected(self):
        with self.output('arm64', 'LC_BUILD_VERSION\n minos 12.0'):
            with self.assertRaisesRegex(ValueError, '12.0'):
                check(Path('ffmpeg'), 'arm64')

    def test_linker_version_is_not_mistaken_for_os_minimum(self):
        with self.output('arm64', 'LC_BUILD_VERSION\n minos 11.0\n tool 3\n version 764.0'):
            check(Path('ffmpeg'), 'arm64')

    def test_missing_deployment_command_is_rejected(self):
        with self.output('arm64', 'LC_UUID'):
            with self.assertRaisesRegex(ValueError, 'unknown'):
                check(Path('ffmpeg'), 'arm64')

    def test_wrong_architecture_is_rejected(self):
        with self.output('arm64', ''):
            with self.assertRaisesRegex(ValueError, 'expected x86_64'):
                check(Path('ffmpeg'), 'x86_64')


if __name__ == '__main__':
    unittest.main()
