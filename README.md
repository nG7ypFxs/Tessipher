# Tessipher 0.1

組込み向けの QR Model 2 リーダーです。
Rust の実行時クレートには外部依存がなく、`no_std`、ヒープ不使用で動きます。
画像、作業領域、復号出力は呼出側が所有します。
MSRV は Rust 1.85、ライセンスは MIT OR Apache-2.0 です（参照テストコードの個別ライセンスは [THIRD_PARTY.md](THIRD_PARTY.md)）。

| クレート | 役割 |
| --- | --- |
| `qr-core` | サンプリング済みモジュール行列から復号。画像処理に依存しない |
| `qr-detect` | Gray8 の走査、finder 検出、射影補正、サンプリング、最大1件の復号 |
| `qr` | 両 API の再公開とメモリ量の照会 |
| `qr-ffi` | アロケータを使わない C ABI、サイズ照会、生成 C ヘッダ |

## 対応範囲

QR Model 2 の Version 1–40、L/M/Q/H、mask 0–7 に対応します。
format/version BCH、機能モジュール除外、アンマスク、ブロック分離、Reed–Solomon 訂正を実装しています。
Numeric、Alphanumeric、Byte、Kanji、ECI、FNC1 の第一位置と第二位置、Structured Append を解釈します。
Byte は生バイト、Kanji は Shift_JIS の2バイト列を出力し、文字コード変換は行いません。
各セグメントにはモード、出力範囲、最後に出現した ECI 番号（初期値3） を記録します。
FNC1 の `%` 処理は Alphanumeric に適用し、Structured Append は index/total/parity を返します（複数シンボルの結合は呼出側の責任です）。

Gray8 は矩形の借用ビューです。
幅640、stride1920の ROI を画像コピーなしで渡せます。
幅と高さのコンパイル時上限は別々に指定でき、既定値はそれぞれ640です。
走査候補数、詳細候補数、再試行数は独立した予算です。
鏡像、回転、射影変形、白黒反転、照明勾配を含む合成画像テストを同梱しています。
行列は通常方向の復号に失敗すると転置して再試行します。
RS訂正は規格7.5.1の推奨pを採用し、V1-Lは最大2語、V1-MとV2-Lは最大4語の訂正に制限します。
検出条件と有界探索の内容は [検出仕様](docs/detection.md) を参照してください。

Micro QR、rMQR、Model 1、複数コードの同時返却、文字コード変換、カメラ制御、画像ファイル形式の読込みは公開ライブラリの対象外です。

## Rust API

```rust
use qr::{Config, GrayImage, Segment, Detection};

fn decode_roi(
    frame_roi: &[u8],
    detector_scratch: &mut [u8],
    core_scratch: &mut [u8],
    payload: &mut [u8],
    segments: &mut [Segment],
) -> Result<Detection, qr::detect::Error> {
    let config = Config::<640, 240>::default();
    let image = GrayImage::new(frame_roi, 640, 240, 1920)?;
    qr::decode_gray(image, &config, detector_scratch, core_scratch, payload, segments)
}
```

必要な作業領域は `qr::memory_requirements(&config)` で照会できます。
既定設定では core が **7077 bytes**、detector が **4557 bytes**、合計 **11634 bytes**、alignment は1です。
これらの領域をタスク用 arena やファームウェアの専用 RAM に確保し、呼出しをまたいで再利用してください。
大きな所有型の Scratch を構築してスタックへコピーする API はありません。

行列だけを復号する場合は `qr_core::Matrix::new(bits, side)` と `qr_core::decode(...)` を使います。
ビット列は row-major、MSB first、行ごとのパディングなしです。
side は21から177までの `17 + 4 × version` です。
詳細な出力契約とエラーは [メモリと復号 API](docs/api.md) に記載しています。

## ビルドと検証

```sh
cargo test --workspace
cargo test --workspace --release
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
python3 tools/c_smoke.py
python3 tools/cross_decode.py
cargo run -p qr --example fuzz_smoke --release
cargo run -p qr --example bench --release
cargo run -p qr --example read_pgm --release -- fixtures/perspective-strong-v1.pgm
```

検証用にNayukiの1280行列と、独立したSegno 1.6.6によるモード/版境界24行列を同梱しています。

C ヘッダは [include/tessipher.h](include/tessipher.h) に同梱しています。
`tools/check_header.py` が ABI schema から生成されたヘッダとの差分を検査します。
Cortex-M 向けコマンドと C のポインタ契約は [C ABI](docs/ffi.md)、実行した検証結果は [検証記録](docs/validation.md) にあります。

標準の版は ISO/IEC 18004:2024（第4版）、対応する JIS X 0510:2026 として確認しました。
規格全文に基づく認証試験は実施しておらず、規格適合認証を表明しません。
参照先は [ISO](https://www.iso.org/standard/83389.html) と [日本規格協会](https://webdesk.jsa.or.jp/books/W11M0090/index/?bunsyo_id=JIS+X+0510%3A2026) です。
