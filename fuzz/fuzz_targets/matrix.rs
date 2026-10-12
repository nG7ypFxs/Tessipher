#![no_main]
use libfuzzer_sys::fuzz_target;
fuzz_target!(|data:&[u8]|{if data.is_empty(){return;}let side=21+(data[0]as usize%40)*4;if let Ok(m)=qr::core::Matrix::new(&data[1..],side){let mut scratch=vec![0;qr::core::SCRATCH_BYTES];let mut out=vec![0;qr::core::MAX_PAYLOAD_BYTES];let mut seg=vec![qr::Segment::EMPTY;128];let _=qr::decode_matrix(m,&mut scratch,&mut out,&mut seg);}});
