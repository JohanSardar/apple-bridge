import os
import base64
from types import SimpleNamespace
import subprocess
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch, Mock

import signing
import release
import testflight


class SigningTests(unittest.TestCase):
    def test_password_is_private_and_keychain_operations_never_target_login(self):
        with tempfile.TemporaryDirectory(prefix='ab-test-signing-') as temp:
            directory = Path(temp)
            password = directory / 'keychain.password'
            keychain = directory / 'ab-release.keychain-db'
            with patch.multiple(signing, DIRECTORY=directory, PASSWORD=password, KEYCHAIN=keychain), patch.object(signing, 'command') as command:
                first = signing.prepare_keychain()
                self.assertEqual(len(first), 64)
                self.assertEqual(password.stat().st_mode & 0o777, 0o600)
                self.assertEqual(signing.prepare_keychain(), first)
                for call in command.call_args_list:
                    self.assertEqual(call.args[-1], keychain)
                self.assertIn(('security', 'set-keychain-settings', keychain), [call.args for call in command.call_args_list])
                password.chmod(0o644)
                with self.assertRaises(ValueError):
                    signing.prepare_keychain()

    def test_never_overwrites_a_keychain_password_or_follows_a_password_link(self):
        with tempfile.TemporaryDirectory(prefix='ab-test-signing-') as temp:
            directory = Path(temp)
            password, keychain = directory / 'password', directory / 'keychain'
            keychain.touch()
            with patch.multiple(signing, DIRECTORY=directory, PASSWORD=password, KEYCHAIN=keychain), patch.object(signing, 'command') as command:
                with self.assertRaises(ValueError):
                    signing.prepare_keychain()
                command.assert_not_called()
                target = directory / 'keep'
                target.write_text('original')
                password.symlink_to(target)
                with self.assertRaises(ValueError):
                    signing.prepare_keychain()
                self.assertEqual(target.read_text(), 'original')

    def test_errors_do_not_expose_passwords(self):
        result = Mock(returncode=1, stdout='', stderr='private details')
        with patch.object(signing.subprocess, 'run', return_value=result):
            with self.assertRaises(ValueError) as raised:
                signing.command('security', 'unlock-keychain', '-p', 'secret-password', 'keychain')
            self.assertNotIn('secret-password', str(raised.exception))
            self.assertNotIn('private details', str(raised.exception))

    def test_timeouts_do_not_expose_passwords(self):
        with patch.object(signing.subprocess, 'run', side_effect=subprocess.TimeoutExpired(['security', '-p', 'secret-password'], 120)):
            with self.assertRaises(ValueError) as raised:
                signing.command('security', 'unlock-keychain', '-p', 'secret-password', 'keychain')
            self.assertNotIn('secret-password', str(raised.exception))

    def test_signed_release_never_falls_back_to_default_keychains(self):
        with tempfile.TemporaryDirectory(prefix='ab-test-signing-') as temp:
            root = Path(temp)
            seed = root / 'seed'
            seed.write_text(base64.b64encode(bytes(32)).decode())
            seed.chmod(0o600)
            args = SimpleNamespace(version='0.1.2', build='3', feed_url='https://example.invalid/feed', download_url='https://example.invalid/update', output=str(root / 'release'), prepare_unsigned=False)
            environment = {'SPARKLE_KEY_FILE': str(seed), 'DEVELOPER_ID_APPLICATION': 'A' * 40}
            with patch.dict(os.environ, environment, clear=True), patch.object(release, 'output') as output:
                with self.assertRaisesRegex(ValueError, 'RELEASE_KEYCHAIN'):
                    release.build(args)
                output.assert_not_called()
            keychain = root / 'ab-release.keychain-db'
            keychain.touch()
            environment.update(RELEASE_KEYCHAIN=str(keychain), ASC_KEY_PATH='/private/api.p8', ASC_KEY_ID='KEY', ASC_ISSUER_ID='ISSUER')
            with patch.dict(os.environ, environment, clear=True), patch.object(release, 'output', return_value='A' * 40 + ' "Developer ID Application: Test"') as output, patch.object(release, 'sparkle', side_effect=RuntimeError('preflight completed')):
                with self.assertRaisesRegex(RuntimeError, 'preflight completed'):
                    release.build(args)
                output.assert_called_once_with('security', 'find-identity', '-v', '-p', 'codesigning', keychain.resolve())

    def test_apple_issued_certificate_can_be_imported_without_an_api_request(self):
        with tempfile.TemporaryDirectory(prefix='ab-test-signing-') as temp:
            root = Path(temp)
            certificate = root / 'download.cer'
            certificate.write_bytes(b'public certificate')
            (root / 'developer-id.key').write_text('private fixture')
            (root / 'developer-id.csr').write_text('public fixture')
            def command(*args):
                if args[:2] == ('openssl', 'x509') and '-fingerprint' in args:
                    return 'sha1 Fingerprint=' + ':'.join(['AA'] * 20)
                if args[:2] == ('security', 'find-identity'):
                    return 'A' * 40
                return ''
            with patch.multiple(signing, DIRECTORY=root, PASSWORD=root / 'password', KEYCHAIN=root / 'ab-release.keychain-db'), patch.object(signing, 'command', side_effect=command) as calls, patch('builtins.print'):
                signing.provision('DEVELOPER_ID_APPLICATION', certificate)
            self.assertFalse(any(call.args[0] == 'asc' for call in calls.call_args_list))
            self.assertEqual((root / 'developer-id.der').read_bytes(), certificate.read_bytes())

    def test_every_nested_codesign_uses_selected_keychain(self):
        with tempfile.TemporaryDirectory(prefix='ab-test-signing-') as temp:
            app = Path(temp) / 'Example.app'
            binary = app / 'Contents/MacOS/AppleBridgeHelper'
            binary.parent.mkdir(parents=True)
            binary.touch()
            with patch.object(release, 'run') as run, patch.object(release, 'output', return_value='Mach-O'), patch.object(release.subprocess, 'run', return_value=Mock(stderr='Authority=Developer ID Application: Test\nflags=runtime')):
                release.sign_bundle(app, 'IDENTITY', '/private/release.keychain-db')
            signs = [call.args for call in run.call_args_list if '--sign' in call.args]
            self.assertGreaterEqual(len(signs), 2)
            for call in signs:
                self.assertEqual(call[call.index('--keychain') + 1], '/private/release.keychain-db')

    def test_xcode_export_uses_apple_tools_before_homebrew(self):
        with patch.dict(os.environ, {'PATH': '/opt/homebrew/bin:/usr/bin', 'SDKROOT': '/wrong-sdk'}):
            environment = testflight.xcode_environment()
        self.assertEqual(environment['PATH'].split(':')[0], '/usr/bin')
        self.assertNotIn('SDKROOT', environment)


if __name__ == '__main__':
    unittest.main()
