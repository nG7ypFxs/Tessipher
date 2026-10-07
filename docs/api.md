# メモリと復号 API

## 所有権とサイズ

safe Rust の3クレートには `#![forbid(unsafe_code)]` を指定しています。
入力は `&[u8]`、作業領域と出力は `&mut [u8]`、セグメントは `&mut [Segment]` です。
再帰、可変グローバル、動的確保は使いません。
複数タスクから使う場合は、タスクごとに別の作業領域と出力を用意します。

| 領域 | 必要サイズ | alignment |
| --- | ---: | ---: |
| core scratch | `qr_core::SCRATCH_BYTES` = 7077 bytes | 1 |
| detector scratch | `3917 + 20 × max_finders` bytes | 1 |
| payload | 最大7089 bytes。用途に合わせて小さくできる | 1 |
| segment | `Segment` の配列。保守的な全入力上限5912要素 | `align_of::<Segment>()` |

core scratch は raw codewords 3706、data codewords 2956、単一 RS block 255、RS workspace 160 bytes に分けて使います。
detector scratch は最大177×177 modules のpacked行列と finder records のみです。
画像面積に比例する binary image、integral image、追加フレームは作りません。
32 finder の既定設定では、幅上限を320から640へ変更しても detector scratch は4557 bytesです。

payload 上限7089は Numeric の最大桁数から定めています。
全セグメント容量の保守的上限は data codewords の全ビット数を最小モード長4で割った値です。
実用途でセグメントを8個などに制限しても安全に使えますが、容量超過は `SegmentsTooSmall` になります。
ホスト64bitでは `Segment` が32 bytes、32bitターゲットでは16 bytesです。
正確なサイズは各ターゲットの `size_of` または C ABI の照会を使ってください。

大きな配列はすべて呼出側領域です。
実装の局所配列の最大単体は射影方程式の `[[f32; 9]; 8]`（288 bytes）です。
コンパイラの spill、呼出階層、割込みを含む実測スタック上限は未測定です。
MCUへの統合時に使用するコンパイラと最適化設定で watermark を測定してください。

## 画像ビュー

`GrayImage::new(data, width, height, stride)` はゼロ寸法と `stride < width` を拒否します。
必要なバイト長は `(height - 1) * stride + width` で、乗算と加算のoverflowも検査します。
最終行の末尾パディングは不要です。
ROI の左上を `data[0]` に合わせ、親フレームのstrideをそのまま渡せます。

画像上限は `Config::<MAX_WIDTH, MAX_HEIGHT>` で指定します。
各上限は1から32768、finder容量は3から128、候補予算は1から1,000,000、retry予算は1から65536です。
不正な構成は `InvalidConfig`、構成の画像上限を超える入力は `ImageTooLarge` として返します。

## 行列の向きと鏡像

行列入口はfinderが左上、右上、左下に来る向きを前提とし、通常方向を先に復号します。
`Format`、`Version`、`ReedSolomon`、`Truncated`、`UnsupportedMode`、`MalformedPayload` で失敗した場合、座標参照を転置して一度だけ再試行します。
鏡像でもformatが有効候補になることがあるため、format以降の内容エラーも対象にしています。
入力の変更や行列コピーは行わず、同じscratchと出力領域を再利用します。
`InvalidMatrix` と容量エラーでは再試行しません。
転置側が容量エラーならそのエラーを返し、両方向が内容エラーなら通常方向のエラーを返します。

## 復号結果

`Decoded` はversion、ECC、mask、payload長、セグメント数、訂正codeword数、BCH距離、FNC1、Structured Appendを持ちます。
セグメントの `offset..offset+len` はpayload内の範囲で、制御セグメントの長さは0です。
`eci` は最後に出現したECI番号を記録し、初期値は3です。
文字コードECIと変換ECIが共存するときの有効ECI集合を表すフィールドではありません。
すべてのECI制御セグメントを順序と割当番号付きで保持するため、上位層はその履歴から解釈できます。
文字コード・暗号・圧縮の変換は行わず、Byteデータは生バイトを返します。
Kanjiモードの出力形式はECIに関係なくShift_JISです。

FNC1第一位置と第二位置では Alphanumeric 内の単独 `%` を GS（0x1D）、`%%` を文字 `%` に変換します。
Byteの0x25は変換しません。
第二位置の8bit application indicatorは0–99、165–190、197–222を受理し、元の値を返します。
それ以外は `MalformedPayload` を返します。
FNC1は最初のデータモードの前に一度だけ指定でき、先頭のStructured AppendやECIヘッダの後にも置けます。
長さ0のデータモードを含め、データモードの後や重複したFNC1は拒否します。
FNC1後にもECIを指定でき、以降のセグメントの `eci` に最後の番号を記録します。
Structured Appendは0始まりindex、1–16のtotal、parity byteを返し、`index >= total`、後置または重複した制御指定を拒否します。
ECIは最大999999までの1/2/3byte表現を受け付けます。

## 訂正済みcodewordの解析

別のdemodulatorで得た訂正済みdata codewordには `qr_core::parse_payload_info` を使えます。
返す `PayloadInfo` のフィールドは `Decoded` と同じ `payload_len`、`segment_count`、`fnc1`、`structured_append` です。
復号行列のversion、ECC、mask、訂正数、BCH距離はこの型に含みません。
versionは1から40、dataの長さは `MAX_DATA_CODEWORDS` 以下である必要があります。
終端子と4bit未満のゼロpaddingを受理し、容量不足と失敗時の部分出力は他の復号入口と同じ契約です。

```rust
use qr::core::{parse_payload_info, Segment};

let mut output = [0; 1];
let mut segments = [Segment::EMPTY; 1];
// V1のByteモード、長さ1、文字A、終端子。
let info = parse_payload_info(&[0x40, 0x14, 0x10], 1, &mut output, &mut segments)?;
assert_eq!(&output[..info.payload_len], b"A");
assert_eq!(info.segment_count, 1);
```

既存の `parse_payload` は `(payload_len, segment_count, fnc1, structured_append)` のtupleを返し続けます。
両入口は同じ解析実装を使います。

## エラーと部分出力

失敗した呼出しの作業領域、payload、セグメントの内容は未規定です。
成功した場合だけ返された長さを読んでください。
再試行のたびに同じ領域を再利用できます。
`decode_with_stats` は失敗時にも走査と予算の使用数を呼出側へ返します。

| core error | 段階 |
| --- | --- |
| `InvalidMatrix` | sideまたはpackedデータ長 |
| `ScratchTooSmall` | 作業領域 |
| `Format` | format BCH、同距離の競合 |
| `Version` | version BCH、寸法との不一致 |
| `ReedSolomon` | ブロックの訂正失敗 |
| `Truncated` | セグメントの必要ビット不足 |
| `UnsupportedMode` | 対応外のmode indicator |
| `MalformedPayload` | 数値範囲、Kanji値、ECI値、制御順序 |
| `OutputTooSmall` / `SegmentsTooSmall` | 呼出側出力容量 |

RSはlocatorの次数、根の個数、訂正後syndromeを検査します。
JIS X 0510:2026（ISO/IEC 18004:2024）7.5.1の誤訂正抑制の推奨pを採用し、ブロックごとに `2t <= ECC codewords - p` を満たす訂正数tだけを受理します。
消失訂正は行いません。

| version/ECC | p | 許容する訂正数の変更 |
| --- | ---: | --- |
| V1-L | 3 | 3語から2語 |
| V1-M、V2-L | 2 | 5語から4語 |
| V1-Q、V1-H、V3-L | 1 | それぞれ6語、8語、7語で不変 |
| その他 | 0 | 不変 |

syndrome計算と訂正後検査には全ECC codewordsを使います。
この推奨の採用により、従来成功したV1-Lの3語破損、V1-MとV2-Lの5語破損を拒否します。
訂正能力を超える破損は別の有効codewordへ化ける可能性があり、過大な破損を必ず検出する契約ではありません。
各方向のBCH処理では、両copyが同じ距離で異なる有効値を示す場合は任意に選択せず、その方向をエラーにします。
通常方向がこの競合で失敗しても、転置方向の復号が成功すれば成功を返します。
