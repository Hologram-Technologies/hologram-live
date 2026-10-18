#!/usr/bin/env python3
"""Package exact Git library/oracle source, excluding unrelated web payloads.

This is a source delivery projection, never a binary or application release.
Cargo manifests and lockfiles are not rewritten. All retained file bytes and
modes are independently checked against the requested immutable Git tree.
"""
import argparse
import hashlib
import io
import json
from pathlib import Path, PurePosixPath
import re
import subprocess
import tarfile

EXCLUDED = 'apps/model-hub/web/public/'
REQUIRED = {
    'Cargo.toml', 'Cargo.lock', 'build.rs', 'rust-toolchain.toml',
    'src/lib.rs', 'src/holo_provider.rs',
    'crates/hologram-view-surface/src/lib.rs',
    'proto/hologram/live/v1/live.proto',
    'proto/hologram/live/plugin/v1/plugin.proto',
    'specs/wit/hologram-application-v1.wit',
}


def git(root, *args):
    return subprocess.check_output(['git', '-C', str(root), *args])


def package(root, revision, output, manifest):
    if not re.fullmatch(r'[0-9a-f]{40}', revision):
        raise ValueError('an exact immutable forty-hex Git commit is required')
    if git(root, 'cat-file', '-t', revision).strip() != b'commit':
        raise ValueError('source revision must be a commit')
    entries = {}
    for row in git(root, 'ls-tree', '-rz', '--full-tree', revision).split(b'\0'):
        if not row:
            continue
        header, name = row.split(b'\t', 1)
        mode, kind, oid = header.decode('ascii').split()
        name = name.decode('utf-8')
        path = PurePosixPath(name)
        if path.is_absolute() or '..' in path.parts or kind != 'blob' or mode not in {'100644', '100755', '120000'}:
            raise ValueError('source tree contains an unsupported member')
        if not name.startswith(EXCLUDED):
            entries[name] = (mode, oid)
    if not REQUIRED.issubset(entries):
        raise ValueError('library, build, protocol, or lock source is missing')
    archive = git(root, 'archive', '--format=tar', revision, '--', '.', ':(exclude)' + EXCLUDED.rstrip('/'))
    observed = {}
    source = {}
    with tarfile.open(fileobj=io.BytesIO(archive), mode='r:') as handle:
        for member in handle:
            if member.isdir():
                continue
            if member.name not in entries or member.name in observed:
                raise ValueError('archive member inventory differs from the Git projection')
            mode, oid = entries[member.name]
            if mode == '120000' and member.issym():
                data = member.linkname.encode('utf-8')
            elif mode != '120000' and member.isfile():
                data = handle.extractfile(member).read()
                if bool(member.mode & 0o111) != (mode == '100755'):
                    raise ValueError('archive executable mode differs from Git')
            else:
                raise ValueError('archive member kind differs from Git')
            actual_oid = hashlib.sha1(b'blob ' + str(len(data)).encode() + b'\0' + data).hexdigest()
            if actual_oid != oid:
                raise ValueError('archive bytes differ from the immutable Git blob')
            observed[member.name] = {'path': member.name, 'mode': mode, 'git_blob': oid,
                                     'bytes': len(data), 'sha256': hashlib.sha256(data).hexdigest()}
            source[member.name] = data
    if set(observed) != set(entries):
        raise ValueError('archive omits a retained Git source member')
    # Audit literal source/test compile-time includes in the Cargo library
    # workspace. Generated OUT_DIR/protobuf includes retain their build inputs.
    includes = []
    for name, data in source.items():
        if not name.endswith('.rs') or not (name.startswith(('src/', 'crates/', 'tests/')) or name == 'build.rs'):
            continue
        for match in re.finditer(rb'include(?:_(?:bytes|str))?!\(\s*"([^"\r\n]+)"\s*\)', data):
            target = match[1].decode('utf-8')
            parts = list(PurePosixPath(name).parent.parts)
            for part in PurePosixPath(target).parts:
                if part == '..':
                    if not parts:
                        raise ValueError('compile-time include escapes the source package')
                    parts.pop()
                elif part != '.':
                    parts.append(part)
            resolved = '/'.join(parts)
            if resolved not in entries:
                raise ValueError('compile-time include is absent from the source package: ' + resolved)
            includes.append({'source': name, 'target': resolved})
    record = {'schema': 'hologram-live/portable-view-source-package/1',
              'qualification': False, 'source_revision': revision,
              'repository': 'https://github.com/Hologram-Technologies/hologram-live',
              'scope': 'Exact Cargo library/oracle source. Static model-hub website payloads are excluded; this does not package or qualify the model-hub application.',
              'excluded_prefix': EXCLUDED,
              'archive': {'bytes': len(archive), 'sha256': hashlib.sha256(archive).hexdigest()},
              'entries': [observed[name] for name in sorted(observed)],
              'literal_compile_time_includes': includes}
    output.write_bytes(archive)
    manifest.write_text(json.dumps(record, indent=2) + '\n')
    return record


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--revision', required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--manifest', type=Path, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    record = package(root, args.revision, args.output, args.manifest)
    print(json.dumps({'source_revision': args.revision, 'archive': record['archive'],
                      'retained_files': len(record['entries'])}))


if __name__ == '__main__':
    main()
