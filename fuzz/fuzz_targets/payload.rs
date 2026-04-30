#![no_main]
use libfuzzer_sys::fuzz_target;
fuzz_target!(|data:&[u8]|{if data.is_empty(){return;}let mut out=vec![0;qr::core::MAX_PAYLOAD_BYTES];let mut seg=vec![qr::Segment::EMPTY;128];let _=qr::core::parse_payload(&data[1..],data[0],&mut out,&mut seg);});
