# C ABI と組込みビルド

## ビルドとヘッダ

```sh
# ホストのCテスト用static library
cargo build -p qr-ffi --release
python3 tools/check_header.py
python3 tools/c_smoke.py

# Cortex-M0 core と Cortex-M4 soft-float C library
rustup target add thumbv6m-none-eabi thumbv7em-none-eabi
cargo build -p qr-core --target thumbv6m-none-eabi --release
cargo build -p qr-ffi --no-default-features --target thumbv7em-none-eabi --release
```

`qr-ffi` の既定 `host` featureはホストのstaticlibテストのためにstdをリンクします。
ファームウェア用には `--no-default-features --release` を指定します。
no-host staticlibはpanic handlerを持ち、万一のpanic時には無限ループへ入り、Cへunwindしません。
このpanic handlerを含むrlibと、別のRust panic handlerを同時リンクする用途は想定していません。
Rustファームウェアから直接使う場合は `qr` または個別safeクレートをリンクします。

`QR_MAX_WIDTH` と `QR_MAX_HEIGHT` 環境変数でC ABIのコンパイル時画像上限を独立に変更できます。
例は `QR_MAX_WIDTH=640 QR_MAX_HEIGHT=240 cargo build -p qr-ffi --release` です。
ヘッダは寸法をハードコードせず、実際にリンクしたライブラリの `qr_memory` で寸法を返します。
C ABI versionは1、codeword sideの上限は177です。

ABI構造体は `crates/qr-ffi/abi_schema.rs` からRust型とC宣言を生成します。
`build.rs` がOUT_DIRへ `tessipher.h` を出力し、`tools/check_header.py` が同梱版を検査します。
意図的なABI更新時は `python3 tools/check_header.py --write` で同梱版を更新します。
Cでの宣言例と実行可能な検査は `examples/c_smoke.c` にあります。

## 呼出し手順

1. `qr_default_config` で既定予算を取得し、必要なら調整します。
2. `qr_memory` で寸法上限、作業サイズ、segment/resultのサイズとalignmentを照会します。
3. 呼出側がその領域を確保します。
4. `qr_decode_matrix` または `qr_decode_gray` を呼びます。
5. 戻り値0の場合に限り `qr_result` の長さでpayloadとsegmentを読みます。

C APIは領域を確保せず、ユーザーのmallocを要求することもありません。
static storage、arena、RTOSタスク固有メモリなどを使えます。
出力とscratchは未初期化のCストレージでも渡せます。
FFIは数値チェックと重複検査の後、byte領域を0で初期化し、segmentのenum値を有効値で初期化してからRust参照を構築します。
この初期化コストはsafe Rust APIにはありません。

## ポインタ契約

ポインタと長さは、宣言した型のalignmentを満たす、実在する単一allocation内の領域を示す必要があります。
入力byte領域とconfigは初期化済みで、呼出し中に変更されないことが条件です。
出力、scratch、segment、resultは呼出し中の排他的な書込み権限が必要です。
すべての非空領域は入力を含めて互いに重ならない契約です。
別スレッド、DMA、割込みハンドラから同じ領域を変更しないでください。

FFIはnull、alignment、長さの乗算、`isize::MAX`、アドレス加算、領域の重複を検査します。
長さ0のbyte/segment領域に限りnullを許可し、Rustの空sliceとして扱います。
config、result、memory照会出力は必須です。
segment capacityは保守的上限5912まで受け付けます。
任意のポインタのallocation有効性、寿命、実際のallocationサイズ、並行アクセスをライブラリから検証することはできません。

resultは成功時だけ書き込みます。
失敗後のscratch、payload、segmentの内容は未規定です。
戻り値の数値とメタデータの符号化は生成ヘッダのコメントを参照してください。
不正なsizeやポインタ条件は1、重複は2、画像系は10–17、core系は101–110です。

## MCUでの計測

ホストのベンチ結果をMCUの周期数へ換算していません。
Cortex-M4では、ボード初期化後にDWT CYCCNTを有効化し、decode呼出しの直前直後の差をuint32で記録できます。
Cortex-M0ではDWTがない構成があるため、ボードのfree-running timerまたはGPIOとロジックアナライザを使ってください。
割込みを含めた値と除外した値、クロック、flash wait states、FPU設定、cache、compiler flags、ROI寸法、予算、結果コードを一緒に記録します。

`examples/mcu_measure.c` は計測境界だけを示す雛形です。
`board_cycles` と `board_record` はボード側が提供し、画像、領域、初期化も呼出側が管理します。
実ボードの周期数、スタックwatermark、カメラ入力での成功率は未測定です。
