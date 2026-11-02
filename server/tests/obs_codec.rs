use serein_connect::{
	media::{Frame, Kind},
	obs_codec::ObsDemuxer,
};
const SPS: &[u8] = &[
	103, 66, 192, 11, 140, 104, 71, 38, 224, 33, 160, 32, 240, 136, 70, 160,
];
const PPS: &[u8] = &[104, 206, 60, 128];
const IDR: &[u8] = &[
	101, 184, 0, 4, 9, 255, 255, 248, 122, 40, 0, 8, 47, 249, 57, 57, 58, 235, 174, 186, 235, 174,
	188,
];
fn config(sps: &[u8]) -> Vec<u8> {
	let mut b = vec![0x17, 0, 0, 0, 0, 1, sps[1], sps[2], sps[3], 255, 225];
	b.extend((sps.len() as u16).to_be_bytes());
	b.extend(sps);
	b.extend([1, 0, 4]);
	b.extend(PPS);
	b
}
fn picture(nal: &[u8]) -> Vec<u8> {
	let mut b = vec![0x17, 1, 0, 0, 0];
	b.extend((nal.len() as u32).to_be_bytes());
	b.extend(nal);
	b
}
fn frame(kind: Kind, data: Vec<u8>) -> Frame {
	Frame {
		kind,
		epoch: 1,
		sequence: 2,
		data,
	}
}
#[test]
fn obs_aac_wire_packet_roundtrips_as_screen_audio() {
	let mut wire = vec![4, 0, 0, 0, 1, 0, 0, 0, 2];
	wire.extend([1, 0, 0, 0, 42, 0x11, 0x90, 0x21, 0x10]);
	let frame = Frame::decode(&wire).expect("bounded AAC-LC packet must be supported");
	assert_eq!(frame.encode().unwrap(), wire);
}
#[test]
fn obs_h264_config_and_idr_produce_self_contained_valid_wire_packet() {
	let mut d = ObsDemuxer::default();
	assert!(d.video(0, &config(SPS)).unwrap().is_none());
	let p = d.video(42, &picture(IDR)).unwrap().unwrap();
	assert_eq!(p.kind, Kind::VideoH264);
	assert_eq!(&p.data[..9], &[1, 1, 0, 0, 0, 42, 66, 192, 11]);
	assert!(p.data.windows(SPS.len()).any(|v| v == SPS));
	assert!(p.data.windows(PPS.len()).any(|v| v == PPS));
	let f = frame(p.kind, p.data);
	let wire = f.encode().unwrap();
	assert_eq!(Frame::decode(&wire).unwrap().data, f.data);
	let next = d.video(1042, &picture(IDR)).unwrap().unwrap();
	assert!(next.data.windows(SPS.len()).any(|v| v == SPS));
}
#[test]
fn obs_rejects_truncated_and_unsupported_flv_without_panics() {
	for input in [
		vec![],
		vec![0x17],
		vec![0x1c, 0, 0, 0, 0],
		vec![0x90, 0, 0, 0, 0],
		vec![0x17, 3, 0, 0, 0],
	] {
		assert!(ObsDemuxer::default().video(0, &input).is_err());
	}
	for end in 5..config(SPS).len() {
		assert!(ObsDemuxer::default().video(0, &config(SPS)[..end]).is_err());
	}
	let mut d = ObsDemuxer::default();
	assert!(d.video(0, &picture(IDR)).is_err());
	d.video(0, &config(SPS)).unwrap();
	let mut b = picture(IDR);
	b[8] = 255;
	assert!(d.video(0, &b).is_err());
	let mut b = picture(IDR);
	b[4] = 1;
	assert!(
		d.video(0, &b)
			.err()
			.unwrap()
			.to_string()
			.contains("B-bilder")
	);
	let mut b = picture(IDR);
	b[9] = 0xe5;
	assert!(d.video(0, &b).is_err());
	let mut b = picture(IDR);
	b[0] = 0x27;
	assert!(d.video(0, &b).is_err());
	d.video(0, &[0x17, 2, 0, 0, 0]).unwrap();
	assert!(d.video(0, &picture(IDR)).is_err());
}
#[test]
fn obs_sps_dimensions_bit_depth_and_untrusted_wire_headers_are_checked() {
	use h264_reader::{
		nal::WritableNal,
		nal::{Nal, RefNal, sps::SeqParameterSet},
	};
	let nal = RefNal::new(SPS, &[], true);
	let parsed = SeqParameterSet::from_bits(nal.rbsp_bits()).unwrap();
	let mut oversized = parsed.clone();
	oversized.pic_width_in_mbs_minus1 = 120;
	let oversized = oversized.to_vec_with_header(nal.header().unwrap());
	assert!(ObsDemuxer::default().video(0, &config(&oversized)).is_err());
	let mut deep = parsed;
	deep.profile_idc = 100.into();
	deep.chroma_info.bit_depth_luma_minus8 = 2;
	let deep = deep.to_vec_with_header(nal.header().unwrap());
	assert!(ObsDemuxer::default().video(0, &config(&deep)).is_err());
	let mut d = ObsDemuxer::default();
	d.video(0, &config(SPS)).unwrap();
	let p = d.video(0, &picture(IDR)).unwrap().unwrap();
	let mut bad = p.data.clone();
	bad[6] = 100;
	assert!(frame(Kind::VideoH264, bad).validate().is_err());
	let mut bad = p.data.clone();
	bad[1] = 0;
	assert!(frame(Kind::VideoH264, bad).validate().is_err());
	let mut bad = p.data;
	bad.truncate(20);
	assert!(frame(Kind::VideoH264, bad).validate().is_err());
	let mut no_parameters = vec![1, 1, 0, 0, 0, 1, 66, 192, 11, 0, 0, 0, 1];
	no_parameters.extend(IDR);
	assert!(frame(Kind::VideoH264, no_parameters).validate().is_err());
}
#[test]
fn obs_aac_configuration_late_join_timestamp_and_limits() {
	let mut d = ObsDemuxer::default();
	assert!(d.audio(0, &[0xaf, 1, 1]).is_err());
	assert!(d.audio(0, &[0xaf, 0, 0x12, 0x10]).is_err());
	assert!(
		d.audio(0, &[0xaf, 0, 0x11, 0x90, 0x56, 0xe5, 0])
			.unwrap()
			.is_none()
	);
	let p = d.audio(1234, &[0xaf, 1, 0x21, 0x10]).unwrap().unwrap();
	assert_eq!(p.kind, Kind::ScreenAac);
	assert_eq!(p.data, [1, 0, 0, 4, 210, 0x11, 0x90, 0x21, 0x10]);
	assert!(frame(p.kind, p.data).validate().is_ok());
	let mut large = vec![0xaf, 1];
	large.extend(vec![1; 994]);
	assert!(d.audio(0, &large).is_err());
	assert!(d.audio(0, &[0xaf, 1]).is_err());
	assert!(d.audio(0, &[0x2f, 1, 1]).is_err());
	assert!(d.audio(0, &[0xaf, 0, 0x11, 0x91]).is_err());
	assert!(d.audio(0, &[0xaf, 1, 1]).is_err());
	assert!(
		frame(Kind::ScreenAac, vec![1, 0, 0, 0, 0, 0x11, 0x91, 1])
			.validate()
			.is_err()
	);
}

#[test]
fn actual_high_profile_1080p_synthetic_frame_is_accepted() {
	// Chromium WebCodecs encoded a synthetic black canvas; no captured content.
	let annex = include_bytes!("fixtures/obs-high-1080p.h264");
	let starts: Vec<_> = annex
		.windows(4)
		.enumerate()
		.filter_map(|(i, b)| (b == [0, 0, 0, 1]).then_some(i))
		.collect();
	let nals: Vec<_> = starts
		.iter()
		.enumerate()
		.map(|(i, start)| &annex[start + 4..starts.get(i + 1).copied().unwrap_or(annex.len())])
		.collect();
	assert_eq!(nals[0][1], 100);
	let mut demux = ObsDemuxer::default();
	demux.video(0, &config(nals[0])).unwrap();
	let mut flv = vec![0x17, 1, 0, 0, 0];
	for nal in nals.iter().filter(|nal| nal[0] & 31 == 5) {
		flv.extend((nal.len() as u32).to_be_bytes());
		flv.extend(*nal);
	}
	let encoded = demux.video(1000, &flv).unwrap().unwrap();
	assert!(frame(encoded.kind, encoded.data).validate().is_ok());
}

#[test]
fn empty_parameter_sets_are_rejected_without_panicking() {
	let mut empty_pps = config(SPS);
	empty_pps.truncate(empty_pps.len() - PPS.len());
	let end = empty_pps.len();
	empty_pps[end - 2..].copy_from_slice(&[0, 0]);
	assert!(ObsDemuxer::default().video(0, &empty_pps).is_err());
	assert!(
		ObsDemuxer::default()
			.video(0, &[0x17, 0, 0, 0, 0, 1, 66, 192, 11, 255, 225, 0, 0, 0])
			.is_err()
	);
}
