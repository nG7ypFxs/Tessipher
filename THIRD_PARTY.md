# Third-party notices

The Tessipher Rust implementation is licensed under MIT OR Apache-2.0.
The following separately licensed files are included for reproducible tests or parameter tables.
None of the C or Python reference implementations are linked into the Rust runtime crates.

## Project Nayuki QR Code generator (MIT)

Source: <https://github.com/nayuki/QR-Code-generator/blob/master/python/qrcodegen.py>
Retrieved 2026-10-07; the exact vendored bytes are recorded in `tools/vendor/SHA256SUMS`.
`tools/vendor/qrcodegen.py` is unmodified and retains its complete copyright and permission notice.
The ECC codewords-per-block and block-count tables in `crates/qr/src/core/tables.rs` were transcribed from this source.
Fixtures are generated with this independent encoder; `tools/generate_fixtures.py` records the payload and geometry choices.

Copyright (c) Project Nayuki. (MIT License)
https://www.nayuki.io/page/qr-code-generator-library

Permission is hereby granted, free of charge, to any person obtaining a copy of
this software and associated documentation files (the "Software"), to deal in
the Software without restriction, including without limitation the rights to
use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of
the Software, and to permit persons to whom the Software is furnished to do so,
subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

The Software is provided "as is", without warranty of any kind, express or
implied, including but not limited to the warranties of merchantability,
fitness for a particular purpose and noninfringement. In no event shall the
authors or copyright holders be liable for any claim, damages or other
liability, whether in an action of contract, tort or otherwise, arising from,
out of or in connection with the Software or the use or other dealings in the
Software.

## quirc (ISC)

Source: <https://github.com/dlbeer/quirc/tree/master/lib>
Retrieved 2026-10-07; hashes are in `tools/vendor/SHA256SUMS`.
`tools/vendor/quirc/{decode.c,version_db.c,quirc.h,quirc_internal.h}` are unmodified.
They are compiled only by `tools/cross_decode.py` for an independent decoder cross-check.
Each source file retains its original notice; the common license is reproduced below.

Copyright (C) 2010-2012 Daniel Beer <dlbeer@gmail.com>

Permission to use, copy, modify, and/or distribute this software for any
purpose with or without fee is hereby granted, provided that the above
copyright notice and this permission notice appear in all copies.

THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN
ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF
OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.

## Segno 1.6.6 (BSD-3-Clause)

Source: <https://github.com/heuer/segno>
Package: <https://pypi.org/project/segno/1.6.6/>
Segno is a second independent QR and Micro QR encoder used only by `tools/generate_segno_fixtures.py`.
Its source or runtime package is not bundled or linked. The generated 24-vector
Model 2 corpus and 125-vector Micro QR corpus, with versioned provenance, are
checked in under `fixtures/segno-boundaries.*` and `fixtures/micro-segno.*`.
The regeneration environment pins `segno==1.6.6`; its upstream license is reproduced here for reference.

Copyright (c) 2016 - 2025, Lars Heuer
All rights reserved.

Redistribution and use in source and binary forms, with or without modification,
are permitted provided that the following conditions are met:

1. Redistributions of source code must retain the above copyright notice,
   this list of conditions and the following disclaimer.

2. Redistributions in binary form must reproduce the above copyright notice,
   this list of conditions and the following disclaimer in the documentation
   and/or other materials provided with the distribution.

3. Neither the name of the copyright holder nor the names of its contributors
   may be used to endorse or promote products derived from this software
   without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND
ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED
WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE
FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL
DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER
CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY,
OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
