# 実行した検証

2026-10-07のワークスペースで以下を実行しました。
ホストはaarch64 macOS、current compilerは `rustc 1.99.0 (b940084d7 2026-09-28)`、MSRV検証は実際のRust1.85.1です。
CPU製品名はsandboxのsysctl制限により取得していません。

## 復号と画像テスト

| 検査 | 結果 |
| --- | --- |
| `cargo test --workspace`、release、Rust1.85.1 | 全テスト成功 |
| 独立Nayuki生成のV1–40×4 ECC×8 mask | 1280/1280復号、version/ECC/mask/payload照合 |
| 第2独立生成器Segno1.6.6 | V1/9/10/26/27/40×Numeric/Alphanumeric/Byte/Kanjiの24/24成功 |
| 同じ1280行列を独立quircで復号 | 1280/1280照合 |
| 最大payload | Numeric7089桁、Byte2953bytes、Kanji1817文字成功 |
| mixed mode | SA→FNC1第二→ECI26→Numeric→Alphanumeric→Byte→Kanjiとmetadata成功 |
| 直接payload検査 | ECI1/2/3byte、FNC1第一、% escape、範囲外値、重複/順序、truncated、容量不足 |
| BCH | 両copy3bit訂正、formatcopy競合の拒否 |
| RS | V1-Hで0–8codeword訂正、V7-H不均等ブロックで65codeword訂正、既知の過大破損拒否 |
| 画像 | V1/2/7/14/32/40×四方向回転×通常/反転の48ケース |
| 追加画像 | 17°/45°、V1強射影（retry3）、V7射影、local閾値必須の照明勾配、軽いノイズ |
| 境界 | 640×240/stride1920、最終行padding不要、overflow、幅上限、候補/試行/finder予算 |
| 動的確保 | allocator監視付きmatrix/image呼出しで0回 |
| C ABI | C11コンパイル/実行、サイズ/alignment、null/長さ/重複/不整列/short、matrixとGray8 |
| 品質ゲート | fmt、clippy全target `-D warnings`、rustdoc `-D warnings` 成功 |

`fixtures/all-model2.bin` のSHA-256は `86ab198358424eb3b64c7acc22fe24cab1f8d3ccde9a82f7662ab66131a7fcee` です。
fixtureには固定メッセージだけでなく、version/ECC/maskごとに変わる4bytesを入れて期待値を検査しています。
Nayuki生成コードと独立quircデコーダはライセンス通知付きでvendorしてあり、それらの再生成と照合にネットワークは不要です。
第2生成器Segnoの出力は `fixtures/segno-boundaries.bin` に固定し、バージョン、元プロジェクト、各モードと期待payloadを隣のJSONに記録しています。
Segno corpusのSHA-256は `11c1a960e673b133b796c0b1f23d7dc5e505ac1c78468f02ab101598da9e1ce6` です。
通常のRustテストはSegnoのインストールを必要としません。
第2corpusを再生成する場合は、次のようにテスト専用環境へ固定版を導入します。

```sh
python3 -m venv /tmp/tessipher-segno-env
/tmp/tessipher-segno-env/bin/pip install segno==1.6.6
/tmp/tessipher-segno-env/bin/python tools/generate_segno_fixtures.py
```

RSの上限を超えた誤りが常に拒否されるという主張はしていません。

## no_std と独立性

```sh
cargo build -p qr-core --target thumbv6m-none-eabi --release --offline
cargo build -p qr-ffi --no-default-features --target thumbv7em-none-eabi --release --offline
python3 tools/check_symbols.py
cargo check -p qr --target wasm32-unknown-unknown --offline
```

Cortex-M0のcoreとCortex-M4の全no-host C libraryを実際にコンパイルしました。
`thumbv7em-none-eabi` はsoft-floatターゲットで、FPUを必須にしません。
生成static archiveのsymbol検査ではmalloc/calloc/realloc/free、Rust allocation API、unwind runtime/personalityのimportがありません。
Rustのpanic入口のsymbol名 `rust_begin_unwind` は残りますが、この構成ではFFIクレートの無限ループpanic handlerを呼びます。
MCUでリンクして起動するボード固有のstartup、linker script、実行試験は未実施です。

`qr-core` は依存クレート0で単独ビルドでき、`qr-detect` から独立しています。
`qr-core`、`qr-detect`、`qr` はunsafeを禁止しています。
unsafeはCポインタ境界に限定し、Cストレージを初期化した後でsliceを構築します。

## fuzz

決定的mutation smokeと、coverageを使うlibFuzzerをそれぞれ実行しました。
前者はseed `0x738a9015` でpayload20,000、matrix10,000、image200ケースを実行し、panicなしでした。
記録時のreleaseホスト実行時間は0.141秒です。

libFuzzerはnightly、AddressSanitizer、inline 8bit coverage counterを有効にした `cargo fuzz run` で実行しました。
短時間の実行結果は次のとおりです。
これは継続的なfuzzや全入力に対する安全性の証明を代替しません。

| target | 指定秒数 | 実行件数 | coverage counters到達数 | 結果 |
| --- | ---: | ---: | ---: | --- |
| payload | 5 | 1,193,340 | 360 | crashなし |
| matrix | 5 | 2,469 | 388 | crashなし |
| image | 5 | 1,197 | 747 | crashなし |

```sh
python3 tools/fuzz_seeds.py
cargo +nightly fuzz run payload -- -max_total_time=5 -max_len=256
cargo +nightly fuzz run matrix -- -max_total_time=5 -max_len=4000
cargo +nightly fuzz run image -- -max_total_time=5 -max_len=65536
```

`fuzz/seeds` は不変の開始corpus、`fuzz/corpus` と `fuzz/artifacts` はgit管理外です。
fuzz用依存は独立workspace内だけに置いています。
CIは3targetを15秒ずつ実行する設定です。

## ホストベンチマーク

`cargo run -p qr --example bench --release` は各ケース15回の最小、中央値、観測最大を表示します。
release設定は `panic=abort`、LTO有効、codegen-units=1です。
以下は上記ホストでの一回の測定です。
観測最大は試験集合内の最大で、WCETではありません。

| ケース | 寸法 | 最小μs | 中央μs | 観測最大μs | 結果 |
| --- | --- | ---: | ---: | ---: | --- |
| matrix V7-H mask3 | 45×45 modules | 107.83 | 124.62 | 151.71 | 成功 |
| 通常QR V7-H mask3 | Gray8 640×640 stride640 | 2015.87 | 2574.12 | 4434.62 | 成功、candidate1/retry1 |
| QRなし白画像 | Gray8 640×640 stride640 | 11.50 | 11.54 | 12.25 | NotFound |
| 一様乱数ノイズ | Gray8 640×640 stride640 | 11670.12 | 11945.50 | 15941.67 | CandidateBudget、candidate4096 |
| finderを残した復号不能QR | Gray8 640×640 stride640 | 30816.38 | 30902.00 | 31192.88 | Decode(Version)、candidate2/retry60 |

設定はfinder32、candidate4096、retry256、row_step1、local_radius24です。
現在のscratchはcore7077+detector4557=11634bytes、alignment1です。
上の表は二方向pitch保存、鏡像再試行、RS推奨p採用の修正前の履歴です。
その後の変更を含む測定は末尾の「内部整理後の性能比較」に記録しています。
検出の失敗時にも `decode_with_stats` で予算消費を確認できます。
MCU実測値、実画像の認識率、flash/RAM配置後のスタック上限、規格適合認証は未取得です。

## FNC1ヘッダ順序の修正検証

2026-10-07、JIS X 0510:2026（ISO/IEC 18004:2024対応）の7.4.9.2と7.4.9.3（本文31–32頁）を公式[JISCビューア](https://www.jisc.go.jp/pdf2/Viewer/2eb0dc1a-09e1-4e4f-9800-34e4f6e789e3/4409b1b1-faa3-4580-aabc-d901356eb0f1)で照合しました。
FNC1第一位置・第二位置はECIや先頭のStructured Appendヘッダの後、最初のデータモードの前に指定できます。
ECIをデータ開始と扱っていた判定を修正しました。
この照合はヘッダ順序に限り、規格全体への適合を示すものではありません。

追加した合法順序のpayload回帰テストは修正前に `MalformedPayload` で失敗し、修正後に成功しました。
ECI→FNC1第一/第二→データ、SA→ECI→FNC1第一/第二→データの4通りで、出力、ECI適用、セグメント順序、第二位置のapplication indicator、SA metadataを確認しています。
FNC1後のECIも受理し、FNC1重複、データ後のFNC1、長さ0の各データモード後のFNC1、先頭以外のSAを拒否する境界テストを追加しました。
Alphanumericの `%` 変換とByteの生値維持を確認しています。
独立Nayuki生成のV1-L行列4個でも同じ合法順序をmatrix入口から復号しました。

| コマンド | 結果 |
| --- | --- |
| `cargo test -p qr-core --test payload` | 6/6成功 |
| `cargo test --workspace` | 全テスト成功、新規matrix順序テストを含む |
| `python3 tools/generate_fixtures.py` | 既存fixtureのSHA-256は全て不変、新規4行列を生成 |
| `cargo fmt --all -- --check` | 成功 |
| Rust1.85.1 `cargo check -p qr-core --lib` | 成功 |

## 規格照合と今回の修正

JIS X 0510:2026（ISO/IEC 18004:2024対応）を公式[JISCビューア](https://www.jisc.go.jp/pdf2/Viewer/2eb0dc1a-09e1-4e4f-9800-34e4f6e789e3/4409b1b1-faa3-4580-aabc-d901356eb0f1)で確認した範囲を記録します。
これは規格適合認証や規格全体の適合宣言ではありません。

| 分類 | 根拠と現状 |
| --- | --- |
| 確認済み・修正済み | 7.4.9.2/.3のFNC1ヘッダ位置。ECI/先頭SA後、最初のデータモード前の第一/第二位置を受理 |
| 今回修正 | 7.4.9.3の第二位置application indicator。0–99、165–190、197–222以外を拒否 |
| 今回説明修正 | 7.4.3.3で文字コードECIと変換ECIは共存可能。`Segment.eci` は最後の番号であり、集合ではない。全ECI履歴を保持するが変換は行わない |
| 今回検出修正 | 水平・垂直pitchを分離し、V7の2×4/4×2pixels/module画像を復号。参照算法の実装そのものや任意変形の保証ではない |
| 未実装の送信層 | 第14章のAIM識別子、ECI送信escape、FNC1第二位置prefix、SA結合はraw payload/metadata APIの上位層に委ねる |
| 推奨を採用 | 7.5.1のRS訂正上限pを採用。詳細と訂正能力への影響は後述の鏡像とRS検証を参照 |
| 参照手順との差・挙動維持 | 第12章の二重BCH情報の選択手順。現実装は両copyから最小Hamming距離の候補を選び、異なる候補が同じ最小距離を持つ場合は拒否する。参照手順との差を直ちに必須要件違反とはしない |
| 未確定・挙動維持 | FNC1 Alphanumericの `%` escapeをセグメント境界で継続すべきかは未確定。現実装は各セグメント内で処理 |

修正前にはindicator100の誤受理とV7の2×4pixels/module画像の `NotFound` を新規回帰テストで再現しました。
FNC1第二位置の全256値を検査し、許容値では生metadataと後続Byte出力、不許容値では `MalformedPayload` を確認します。
複数ECIの履歴は制御順序・番号、後続データの最後のECI番号、offset/len、生Byte出力を確認します。
検出は4×4/2×4/4×2pixels/moduleを同一描画helperで検証し、既存の回転・反転・射影・照明・ノイズ・stride・各予算の試験も維持しています。

今回の修正後に実行した検証は次のとおりです。

| コマンド | 結果 |
| --- | --- |
| `cargo test -p qr-core --test payload` | 8/8成功、第二位置全256値と複数ECI履歴を含む |
| `cargo test -p qr-detect --test images` | 7/7成功、2×4/4×2画像と既存回転・射影を含む |
| `cargo test --workspace` | 全成功、model2 9/9、payload 8/8、images 7/7、allocation監視1/1 |
| `cargo fmt --all -- --check` | 成功 |
| `cargo clippy --workspace --all-targets -- -D warnings` | 成功 |
| Rust1.85.1 `cargo check -p qr-core -p qr-detect --lib --offline` | 成功 |
| `cargo build -p qr-core -p qr-detect --target thumbv6m-none-eabi --release --offline` | 成功 |
| `cargo build -p qr-ffi --no-default-features --target thumbv7em-none-eabi --release --offline` | 成功 |
| `python3 tools/check_header.py` / `python3 tools/c_smoke.py` | 成功、照会core7077/detector4557/alignment1 |
| `python3 tools/check_symbols.py` | 更新したCortex archiveにheap/unwind runtime importなし |

公開のRust/FFI型とABIは変更していません。
Finder保存領域は16から20bytesへ増えたため、呼出側は更新した `scratch_bytes()` / `qr_memory()` の値を使用します。
画像面積に比例する領域、新規依存、動的確保、safeクレートのunsafeは追加していません。


## 鏡像とRS推奨pの検証

2026-10-07、上記JISCビューアの11 b)（本文64頁）と12 k)（本文71頁）を照合し、通常方向に続く転置復号を追加しました。
行列は入力を変更せず内部の座標参照で転置し、Gray8でも同じcore入口を使います。
鏡像のformatが偶然有効になるケースを独立corpusで確認したため、format以降の内容エラーでも一度だけ転置します。
通常方向を優先し、両方向が内容エラーの場合は通常方向の診断を保持します。
容量不足の扱い、失敗時の出力が未規定という契約、公開Rust/C ABI、scratch量は維持しています。

RSでは7.5.1（本文38頁）の推奨pを採用しました。
これは誤訂正抑制の推奨を採用した変更であり、以前の挙動を必須要件違反と断定するものではありません。
ブロックごとに `2t <= ECC codewords - p` を確認し、syndromeは訂正前後とも全ECC codewordsで計算します。
V1-Lの許容訂正数は3語から2語、V1-MとV2-Lは5語から4語になります。
p=1のV1-Q、V1-H、V3-Lはそれぞれ6語、8語、7語で変わらず、その他はp=0です。
この制限でも過大な破損の誤読を完全には防げません。

BCHは第12章の参照手順との差を記録したうえで、既存の最小距離選択を維持しました。
各方向で同距離の競合を拒否し、一方が訂正不能でも他方から回復することを確認しています。
全alignmentを使った局所グリッド補正は追加していません。
Micro QR、Model 1、rMQR、送信層、SA結合、セグメント間FNC1 escapeの扱いも今回の範囲外です。

| 検証 | 結果 |
| --- | --- |
| Nayuki V1–40×4 ECC×8 maskの通常行列と転置行列 | 2560/2560でpayload/version/ECC/mask一致 |
| 鏡像でもformatが有効となるV1入力 | 通常解釈のRS失敗後に転置して成功 |
| V1-L、V1-M、V2-LのRS境界 | 0語から新上限まで成功、上限+1語の既知破損を拒否 |
| p=1のV1-Q/V1-H/V3-L、p=0のV2-M | 訂正境界まで成功、境界+1語の既知破損を拒否 |
| 不均等ブロック | 既存V7-Hの各block13語、合計65語の訂正成功 |
| Gray8 V1/2/7/14/32/40×鏡像有無×四方向回転×通常/反転 | 96/96成功 |
| 既存の17°/45°、射影V1/強射影V1/V7、照明勾配 | 通常画像に加え左右鏡像6件も成功 |
| 容量、診断、BCH | 両方向の容量不足、通常エラー保持、format/versionの片copy回復、競合拒否を確認 |
| 予算と幾何 | 鏡像をmax_retries=1で復号、cornersのサンプリング順序を確認。既存stride、非等方pitch、予算境界も成功 |
| allocation監視 | 通常と鏡像のmatrix/Gray8呼出しで0回 |

| コマンド | 結果 |
| --- | --- |
| `cargo test --workspace --locked` | 全成功、core単体4、model2 10、payload 8、images 8、allocation監視1 |
| `cargo test --workspace --locked --release` | 全成功 |
| `cargo fmt --all -- --check` | 成功 |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | 成功 |
| `cargo +1.85 check -p qr-core -p qr-detect --lib --offline` | 成功、登録済みtoolchainの実体はrustc/cargo 1.85.1 |
| `cargo build -p qr-core -p qr-detect --target thumbv6m-none-eabi --release --offline` | 成功 |
| `cargo build -p qr-ffi --no-default-features --target thumbv7em-none-eabi --release --offline` | 成功 |
| `python3 tools/check_header.py` / `python3 tools/c_smoke.py` | 成功、core7077/detector4557/alignment1を維持 |
| `python3 tools/check_symbols.py` | 更新したCortex archiveにheap/unwind runtime importなし |

通常方向が内容エラーになる場合のcore処理は最大2回になります。
サンプリングのStats.retriesとmax_retriesは増やさず、scratchも増やしていませんが、CPU時間への影響は未測定です。
この修正後の性能保証として以前のベンチマーク値を使用しません。


## 幾何キャッシュと探索状態の整理

2026-10-07、行列復号の各moduleで再計算していたalignment中心を、呼出しごとの固定長geometryにまとめました。
通常方向と転置方向で同じgeometryを使い、alignmentの判定は中心の全組合せ探索から、xとyの帯を個別に調べる探索へ変更しています。
中心数をkとすると各moduleの判定はO(k²)からO(k)になります。
全V1–40の全座標を変更前のpairwise判定と比較し、data module数と `raw_codewords` の整合、V32の特殊間隔、範囲外座標と不正versionの扱いを確認しました。

検出では `SearchContext` に作業領域、出力、Stats、予算、最後の復号エラーを集約し、三点候補、version候補、射影のsamplingを別のメソッドに整理しました。
探索順と浮動小数点の式を維持し、画像面積に比例する領域を追加していません。
失敗時Stats、pass間のcandidateとretry予算共有、幾何的にsamplingを破棄した場合のretry計上、容量エラーでの即時停止、FinderCapacityの最終優先順位を回帰テストで固定しました。
Statsの期待値は変更前のHEADでも照合しています。
強射影V1はcandidate1/retry3/pass1、local閾値が必要な画像はcandidate1/retry1/pass3のままです。

訂正済みdata codewordには名前付き結果 `PayloadInfo` を返す `parse_payload_info` を追加しました。
既存のtuple APIは新入口へ委譲し、内部の解析結果も同じ型を使います。
合法な制御セグメントの混在と不正な制御順序を含む既存テストで両入口を比較し、空入力、data長の上限、versionの長さ区分、容量不足、不正version、truncatedの境界も確認しました。

| コマンド | 結果 |
| --- | --- |
| `cargo test --workspace --locked` | 単体と統合37件、APIのdoc test1件成功 |
| `cargo test --workspace --locked --release` | 同じ全テスト成功、allocator監視で動的確保0回 |
| `cargo fmt --all -- --check` | 成功 |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | 成功 |
| `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked` | 成功 |
| `cargo +1.85 check -p qr-core -p qr-detect -p qr --lib --locked --offline` | Rust1.85.1で成功 |
| `cargo build -p qr-core -p qr-detect --target thumbv6m-none-eabi --release --locked --offline` | 成功 |
| `cargo build -p qr-ffi --no-default-features --target thumbv7em-none-eabi --release --locked --offline` | 成功 |
| `cargo check -p qr --target wasm32-unknown-unknown --locked --offline` | 成功 |
| `cargo build -p qr-ffi --release --locked --offline` | 成功 |
| `python3 tools/check_header.py` / `python3 tools/c_smoke.py` | header一致、C ABI成功、core7077/detector4557/alignment1 |
| `python3 tools/check_symbols.py` | Cortex archiveにheapとunwind runtimeのimportなし |
| `cargo run -p qr --example fuzz_smoke --release --locked` | payload20,000/matrix10,000/image200件、panicなし、0.156秒 |

通常と転置の2560行列、Segnoの24行列、最大payload、RSとBCH、回転、鏡像、非等方pitch、射影、照明、strideの既存検査も成功しています。
公開の既存Rust APIとC ABI、scratch量、alignment、新規依存なしという条件を維持しています。
MCUの実測スタック上限と実画像の認識率は引き続き未測定です。

## 内部整理後の性能比較

`bench` にV32とV40のmatrix測定を追加し、変更前にも同じbenchソースを渡しました。
比較した3構成は変更前、coreだけ変更、coreとdetectorを変更した最終構成です。
同じ一時ディレクトリで同じcompilerとrelease設定を使って順次ビルドし、各実行バイナリを保存しました。
以降はビルドを挟まず、構成の実行順を変えて3巡測定しました。
各巡は各ケース15回で、表は3巡の中央値の中央値です。
ホスト、profile、画像、設定は前掲と同じです。

| ケース | 変更前μs | coreのみ変更μs | 最終構成μs |
| --- | ---: | ---: | ---: |
| matrix V7-H mask3 | 54.92 | 31.42 | 32.83 |
| matrix V32 | 783.88 | 471.83 | 472.62 |
| matrix V40 | 1564.42 | 721.00 | 707.88 |
| 通常QR | 1341.21 | 1395.12 | 1303.88 |
| QRなし白画像 | 8.12 | 8.62 | 8.12 |
| 一様乱数ノイズ | 6555.04 | 6797.71 | 6502.67 |
| 復号不能QR | 32525.88 | 31964.54 | 32074.79 |

全9実行のdetectorの結果とStatsは一致しました。
通常はcandidate1/retry1/pass1、白画像はNotFoundで全counter0、ノイズはCandidateBudgetでcandidate4096/retry0/pass1、復号不能QRはDecode(Version)でcandidate2/retry60/pass4です。

プロセス間の時間変動は大きく、通常QRの巡ごとの中央値は変更前1340.83–1410.83μs、coreのみ1318.79–1731.00μs、最終構成1300.33–1829.25μsでした。
ビルド直後の1巡目ではcoreだけの変更でも悪化し、最終構成の2巡目と3巡目は変更前より短い値になりました。
別ディレクトリで行った先の3組の交互比較では、通常QRに約17%の悪化も観測しています。
これらからdetectorの恒常的な速度改善や悪化は断定せず、matrixの探索量削減とホストでの観測値を区別します。
この測定はWCET、全入力の性能、MCUの速度を保証するものではありません。
