#!/usr/bin/env python3
"""Second independent generator. Install exactly segno==1.6.6 in a temporary venv."""
from pathlib import Path
import hashlib
import json
import struct
import segno

assert segno.__version__ == '1.6.6', 'Use pinned segno==1.6.6 for reproducibility'
root = Path(__file__).resolve().parents[1]
corpus = bytearray(b'TQS1')
records = []
modes = [
    ('numeric', 1, '0123456789012345', b'0123456789012345', None),
    ('alphanumeric', 2, 'AB12%+', b'AB12%+', None),
    ('byte', 4, bytes([0, 128, 255]), bytes([0, 128, 255]), None),
    ('kanji', 8, '漢字', '漢字'.encode('shift_jis'), 'shift_jis'),
]
for index, version in enumerate([1, 9, 10, 26, 27, 40]):
    for mode_index, (mode, indicator, content, expected, encoding) in enumerate(modes):
        ecc_index = (index + mode_index) % 4
        ecc = 'LMQH'[ecc_index]
        mask = (index * 3 + mode_index) % 8
        qr = segno.make(content, version=version, error=ecc, mode=mode,
                        encoding=encoding, mask=mask, micro=False, boost_error=False)
        assert (qr.version, qr.error, qr.mask, qr.mode) == (version, ecc, mask, mode)
        side = len(qr.matrix)
        bitmap = bytearray((side * side + 7) // 8)
        for y, row in enumerate(qr.matrix):
            for x, dark in enumerate(row):
                if dark:
                    bit = y * side + x
                    bitmap[bit // 8] |= 128 >> (bit % 8)
        corpus.extend(struct.pack('<BBBBHH', version, ecc_index, mask, indicator,
                                  len(expected), len(bitmap)))
        corpus.extend(expected)
        corpus.extend(bitmap)
        records.append(dict(version=version, ecc=ecc, mask=mask, mode=mode,
                            expected_hex=expected.hex()))
path = root / 'fixtures/segno-boundaries.bin'
path.write_bytes(corpus)
manifest = dict(generator='Segno', version=segno.__version__,
                source='https://github.com/heuer/segno',
                recipe='tools/generate_segno_fixtures.py',
                sha256=hashlib.sha256(corpus).hexdigest(), records=records)
(root / 'fixtures/segno-boundaries.json').write_text(json.dumps(manifest, indent=2) + '\n')
print(f'Segno {segno.__version__}: {len(records)} vectors, {len(corpus)} bytes, sha256 {manifest["sha256"]}')

# Micro QR exercises every legal version / ECC / mode / mask combination. The
# corpus is separate so default Model 2 builds do not compile it into tests.
micro = bytearray(b'TQM1')
micro_records = []
micro_modes = [
    ('numeric', 1, '123', b'123', None, ('M1', 'M2', 'M3', 'M4')),
    ('alphanumeric', 2, 'AB', b'AB', None, ('M2', 'M3', 'M4')),
    ('byte', 4, bytes([0, 128, 255]), bytes([0, 128, 255]), None, ('M3', 'M4')),
    ('kanji', 8, '漢', '漢'.encode('shift_jis'), 'shift_jis', ('M3', 'M4')),
]
micro_ecc = {
    'M1': ((255, None),),
    'M2': ((0, 'L'), (1, 'M')),
    'M3': ((0, 'L'), (1, 'M')),
    'M4': ((0, 'L'), (1, 'M'), (2, 'Q')),
}
for version_number, version in enumerate(('M1', 'M2', 'M3', 'M4'), 1):
    for ecc_index, ecc in micro_ecc[version]:
        for mode, indicator, content, expected, encoding, versions in micro_modes:
            if version not in versions:
                continue
            for mask in range(4):
                qr = segno.make(content, version=version, error=ecc, mode=mode,
                                encoding=encoding, mask=mask, micro=True,
                                boost_error=False)
                assert (qr.version, qr.error, qr.mask, qr.mode) == (version, ecc, mask, mode)
                side = len(qr.matrix)
                bitmap = bytearray((side * side + 7) // 8)
                for y, row in enumerate(qr.matrix):
                    for x, dark in enumerate(row):
                        if dark:
                            bit = y * side + x
                            bitmap[bit // 8] |= 128 >> (bit % 8)
                micro.extend(struct.pack('<BBBBHH', version_number, ecc_index, mask,
                                         indicator, len(expected), len(bitmap)))
                micro.extend(expected)
                micro.extend(bitmap)
                micro_records.append(dict(version=version, ecc=ecc, mask=mask,
                                          mode=mode, expected_hex=expected.hex()))
# DENSO WAVE's published maximum character capacities, one exact-boundary
# vector for every legal version / ECC / mode combination.
micro_capacity = {
    ('M1', None): (5, None, None, None),
    ('M2', 'L'): (10, 6, None, None), ('M2', 'M'): (8, 5, None, None),
    ('M3', 'L'): (23, 14, 9, 6), ('M3', 'M'): (18, 11, 7, 4),
    ('M4', 'L'): (35, 21, 15, 9), ('M4', 'M'): (30, 18, 13, 8),
    ('M4', 'Q'): (21, 13, 9, 5),
}
for version_number, version in enumerate(('M1', 'M2', 'M3', 'M4'), 1):
    for ecc_index, ecc in micro_ecc[version]:
        for mode_index, (mode, indicator, _, _, encoding, versions) in enumerate(micro_modes):
            capacity = micro_capacity[(version, ecc)][mode_index]
            if version not in versions or capacity is None:
                continue
            if mode == 'numeric':
                content = '7' * capacity
                expected = content.encode()
            elif mode == 'alphanumeric':
                content = 'A' * capacity
                expected = content.encode()
            elif mode == 'byte':
                content = bytes((i * 73) & 0xff for i in range(capacity))
                expected = content
            else:
                content = '漢' * capacity
                expected = content.encode('shift_jis')
            mask = (version_number + ecc_index + mode_index) % 4
            qr = segno.make(content, version=version, error=ecc, mode=mode,
                            encoding=encoding, mask=mask, micro=True,
                            boost_error=False)
            assert (qr.version, qr.error, qr.mask, qr.mode) == (version, ecc, mask, mode)
            side = len(qr.matrix)
            bitmap = bytearray((side * side + 7) // 8)
            for y, row in enumerate(qr.matrix):
                for x, dark in enumerate(row):
                    if dark:
                        bit = y * side + x
                        bitmap[bit // 8] |= 128 >> (bit % 8)
            micro.extend(struct.pack('<BBBBHH', version_number, ecc_index, mask,
                                     indicator, len(expected), len(bitmap)))
            micro.extend(expected)
            micro.extend(bitmap)
            micro_records.append(dict(version=version, ecc=ecc, mask=mask,
                                      mode=mode, capacity_boundary=True,
                                      expected_hex=expected.hex()))
micro_path = root / 'fixtures/micro-segno.bin'
micro_path.write_bytes(micro)
micro_manifest = dict(generator='Segno', version=segno.__version__,
                      source='https://github.com/heuer/segno',
                      recipe='tools/generate_segno_fixtures.py',
                      sha256=hashlib.sha256(micro).hexdigest(), records=micro_records)
(root / 'fixtures/micro-segno.json').write_text(json.dumps(micro_manifest, indent=2) + '\n')
print(f'Segno {segno.__version__}: {len(micro_records)} Micro QR vectors, '
      f'{len(micro)} bytes, sha256 {micro_manifest["sha256"]}')
