"""Exact Git-source projection controls; no application or binary substitution."""
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('source_package', Path(__file__).with_name('package-portable-view-source.py'))
owner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(owner)


class SourcePackage(unittest.TestCase):
    def fixture(self, root, forbidden=False):
        for name in owner.REQUIRED:
            path = root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text('source ' + name)
        (root / 'src/lib.rs').write_text('include_bytes!("../apps/model-hub/web/public/payload");' if forbidden else '// library')
        asset = root / 'apps/model-hub/web/public/payload'
        asset.parent.mkdir(parents=True)
        asset.write_bytes(b'unrelated website payload')
        subprocess.run(['git', 'init', '-q', '--initial-branch=main', str(root)], check=True)
        subprocess.run(['git', '-C', str(root), 'add', '.'], check=True)
        subprocess.run(['git', '-C', str(root), '-c', 'user.name=Source Package Test', '-c', 'user.email=source-package@example.com', 'commit', '-qm', 'fixture'], check=True)
        return subprocess.check_output(['git', '-C', str(root), 'rev-parse', 'HEAD'], text=True).strip()

    def test_exact_members_modes_bytes_and_reproduction(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / 'source'; root.mkdir()
            revision = self.fixture(root)
            output, manifest = Path(temporary) / 'source.tar', Path(temporary) / 'manifest.json'
            record = owner.package(root, revision, output, manifest)
            first = output.read_bytes()
            owner.package(root, revision, output, manifest)
            self.assertEqual(first, output.read_bytes())
            self.assertEqual(set(row['path'] for row in record['entries']), owner.REQUIRED)
            self.assertFalse(any(row['path'].startswith(owner.EXCLUDED) for row in record['entries']))
            self.assertEqual(json.loads(manifest.read_text())['source_revision'], revision)

    def test_excluded_compile_time_include_is_refused(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / 'source'; root.mkdir()
            revision = self.fixture(root, forbidden=True)
            with self.assertRaisesRegex(ValueError, 'compile-time include is absent'):
                owner.package(root, revision, Path(temporary) / 'source.tar', Path(temporary) / 'manifest.json')

    def test_excluded_direct_include_is_refused(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / 'source'; root.mkdir()
            revision = self.fixture(root)
            (root / 'src/lib.rs').write_text('include!("../apps/model-hub/web/public/payload");')
            subprocess.run(['git', '-C', str(root), 'add', 'src/lib.rs'], check=True)
            subprocess.run(['git', '-C', str(root), '-c', 'user.name=Source Package Test', '-c', 'user.email=source-package@example.com', 'commit', '-qm', 'direct include'], check=True)
            revision = subprocess.check_output(['git', '-C', str(root), 'rev-parse', 'HEAD'], text=True).strip()
            with self.assertRaisesRegex(ValueError, 'compile-time include is absent'):
                owner.package(root, revision, Path(temporary) / 'source.tar', Path(temporary) / 'manifest.json')

    def test_missing_lock_and_floating_revision_are_refused(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / 'source'; root.mkdir()
            revision = self.fixture(root)
            with self.assertRaisesRegex(ValueError, 'immutable'):
                owner.package(root, 'main', Path(temporary) / 'source.tar', Path(temporary) / 'manifest.json')
            subprocess.run(['git', '-C', str(root), 'rm', '-q', 'Cargo.lock'], check=True)
            subprocess.run(['git', '-C', str(root), '-c', 'user.name=Source Package Test', '-c', 'user.email=source-package@example.com', 'commit', '-qm', 'remove lock'], check=True)
            revision = subprocess.check_output(['git', '-C', str(root), 'rev-parse', 'HEAD'], text=True).strip()
            with self.assertRaisesRegex(ValueError, 'lock source is missing'):
                owner.package(root, revision, Path(temporary) / 'source.tar', Path(temporary) / 'manifest.json')


if __name__ == '__main__':
    unittest.main()
