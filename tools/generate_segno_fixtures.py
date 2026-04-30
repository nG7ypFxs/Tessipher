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
