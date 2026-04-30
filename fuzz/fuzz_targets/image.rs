#![no_main]
use libfuzzer_sys::fuzz_target;
fuzz_target!(|data:&[u8]|{if data.len()<3{return;}let w=data[0]as usize+1;let h=data[1]as usize+1;let stride=w+data[2]as usize;if let Ok(image)=qr::GrayImage::new(&data[3..],w,h,stride){let config=qr::Config::<256,256>{max_finders:8,max_candidates:16,max_retries:8,..Default::default()};let mut ds=vec![0;config.scratch_bytes().unwrap()];let mut cs=vec![0;qr::core::SCRATCH_BYTES];let mut out=vec![0;qr::core::MAX_PAYLOAD_BYTES];let mut seg=vec![qr::Segment::EMPTY;128];let _=qr::decode_gray(image,&config,&mut ds,&mut cs,&mut out,&mut seg);}});
