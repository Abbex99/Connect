//! Bounded FLV/RTMP elementary stream demux; never decodes or re-encodes media.
//!
//! VideoH264 v1: [version=1, IDR flag, PTS milliseconds u32 BE, profile, compatibility,
//! level, Annex-B access unit]. Every IDR includes its SPS/PPS. ScreenAac v1:
//! [version=1, PTS milliseconds u32 BE, ASC=11 90, raw AAC-LC frame]. All packets
//! carry configuration needed by late joiners. AAC is 48 kHz stereo, 1024 samples.
use crate::media::{Kind, MAX_AUDIO_BYTES, MAX_VIDEO};
use anyhow::{Result, bail, ensure};
use h264_reader::{
	Context,
	avcc::AvcDecoderConfigurationRecord,
	nal::{Nal, RefNal, pps::PicParameterSet, sps::SeqParameterSet},
	rbsp::BitRead,
};

pub struct Encoded {
	pub kind: Kind,
	pub data: Vec<u8>,
}
struct Avc {
	length_size: usize,
	profile: [u8; 3],
	parameters: Vec<Vec<u8>>,
}
#[derive(Default)]
pub struct ObsDemuxer {
	avc: Option<Avc>,
	aac: bool,
}

fn nal_type(nal: &[u8]) -> Result<u8> {
	ensure!(
		nal.len() >= 2 && nal[0] & 0x80 == 0,
		"Ogiltig H.264-NAL från OBS."
	);
	let kind = nal[0] & 31;
	ensure!(
		matches!(kind, 1 | 5 | 6 | 7 | 8 | 9 | 12),
		"OBS måste använda vanlig H.264, inte HEVC eller AV1."
	);
	Ok(kind)
}
fn sps(nal: &[u8]) -> Result<SeqParameterSet> {
	ensure!(
		nal.len() <= 4096 && nal_type(nal)? == 7,
		"Ogiltig H.264-konfiguration."
	);
	let value = SeqParameterSet::from_bits(RefNal::new(nal, &[], true).rbsp_bits())
		.map_err(|_| anyhow::anyhow!("Ogiltig H.264-SPS från OBS."))?;
	ensure!(
		matches!(nal[1], 66 | 77 | 100)
			&& value.chroma_info.bit_depth_luma_minus8 == 0
			&& value.chroma_info.bit_depth_chroma_minus8 == 0,
		"Välj H.264 med 8-bitars färg i OBS."
	);
	ensure!(
		value.pic_width_in_mbs_minus1 < 120 && value.pic_height_in_map_units_minus1 < 68,
		"Ställ in OBS på högst 1920 × 1080."
	);
	let (w, h) = value
		.pixel_dimensions()
		.map_err(|_| anyhow::anyhow!("Ogiltiga OBS-bildmått."))?;
	ensure!(
		w > 0 && h > 0 && w <= 1920 && h <= 1080,
		"Ställ in OBS på högst 1920 × 1080."
	);
	ensure!(
		value.max_num_ref_frames <= 16,
		"För många H.264-referensbilder från OBS."
	);
	Ok(value)
}
fn validate_nals(nals: &[&[u8]], profile: &[u8], key: bool) -> Result<()> {
	ensure!(
		!nals.is_empty() && nals.len() <= 64,
		"För många H.264-delar i OBS-bilden."
	);
	let mut context = Context::new();
	let mut have_sps = false;
	let mut have_pps = false;
	let mut idr = false;
	let mut picture = false;
	for nal in nals {
		match nal_type(nal)? {
			7 => {
				let value = sps(nal)?;
				ensure!(
					nal[1..4] == *profile,
					"H.264-profilen stämmer inte med bilden."
				);
				context.put_seq_param_set(value);
				have_sps = true;
			}
			8 => {
				ensure!(
					nal.len() <= 4096 && have_sps,
					"H.264-bilden saknar giltig SPS före PPS."
				);
				let value =
					PicParameterSet::from_bits(&context, RefNal::new(nal, &[], true).rbsp_bits())
						.map_err(|_| anyhow::anyhow!("Ogiltig H.264-PPS från OBS."))?;
				context.put_pic_param_set(value);
				have_pps = true;
			}
			1 | 5 => {
				let kind = nal[0] & 31;
				let reference = RefNal::new(nal, &[], true);
				let mut bits = reference.rbsp_bits();
				let first = bits
					.read_ue("first_mb")
					.map_err(|_| anyhow::anyhow!("Ofullständig H.264-bild."))?;
				let slice = bits
					.read_ue("slice_type")
					.map_err(|_| anyhow::anyhow!("Ofullständig H.264-bild."))?;
				let pps = bits
					.read_ue("pps")
					.map_err(|_| anyhow::anyhow!("Ofullständig H.264-bild."))?;
				ensure!(
					first < 8160 && slice <= 9 && pps <= 255,
					"Ogiltigt H.264-bildhuvud."
				);
				ensure!(slice % 5 != 1, "Ställ in B-bilder (B-frames) på 0 i OBS.");
				ensure!(
					key == (kind == 5),
					"OBS-nyckelbildens NAL-typ stämmer inte."
				);
				idr |= kind == 5;
				picture = true;
			}
			_ => {}
		}
	}
	ensure!(
		picture && (!key || (idr && have_sps && have_pps)),
		"H.264-nyckelbilden måste innehålla SPS, PPS och IDR."
	);
	Ok(())
}
fn annex_nals(data: &[u8]) -> Result<Vec<&[u8]>> {
	ensure!(
		data.starts_with(&[0, 0, 0, 1]),
		"Ogiltig H.264 Annex-B-bild."
	);
	let mut result = Vec::new();
	let mut start = 4;
	let mut i = 4;
	while i + 3 <= data.len() {
		if data[i..].starts_with(&[0, 0, 0, 1]) || data[i..].starts_with(&[0, 0, 1]) {
			result.push(&data[start..i]);
			ensure!(result.len() < 64, "För många H.264-delar.");
			i += if data[i..].starts_with(&[0, 0, 0, 1]) {
				4
			} else {
				3
			};
			start = i;
		} else {
			i += 1;
		}
	}
	result.push(&data[start..]);
	Ok(result)
}
pub fn validate_video(data: &[u8]) -> Result<()> {
	ensure!(
		(14..=MAX_VIDEO).contains(&data.len()) && data[0] == 1 && data[1] <= 1,
		"Ogiltigt OBS-videopaket."
	);
	ensure!(
		matches!(data[6], 66 | 77 | 100) && data[8] > 0 && data[8] <= 52,
		"Välj H.264 baseline, main eller high i OBS."
	);
	validate_nals(&annex_nals(&data[9..])?, &data[6..9], data[1] == 1)
}
pub fn validate_audio(data: &[u8]) -> Result<()> {
	ensure!(
		(8..=MAX_AUDIO_BYTES).contains(&data.len()) && data[0] == 1 && data[5..7] == [0x11, 0x90],
		"Välj AAC-LC, 48 kHz, stereo och 128 kbit/s i OBS."
	);
	Ok(())
}
fn validate_parameter_lengths(bytes: &[u8]) -> Result<()> {
	// h264-reader 0.9 validates the AVCC container length but its parameter
	// iterator indexes the first NAL byte even for a declared zero-byte NAL.
	// Reject those records before invoking the iterator on untrusted input.
	fn consume(bytes: &[u8], cursor: &mut usize, count: usize) -> Result<()> {
		ensure!(
			(1..=4).contains(&count),
			"OBS måste skicka 1–4 SPS/PPS-poster."
		);
		for _ in 0..count {
			let length = bytes
				.get(*cursor..*cursor + 2)
				.ok_or_else(|| anyhow::anyhow!("Ofullständig OBS-konfiguration."))?;
			let length = usize::from(u16::from_be_bytes([length[0], length[1]]));
			*cursor += 2;
			ensure!(
				(2..=4096).contains(&length) && bytes.get(*cursor..*cursor + length).is_some(),
				"Tom eller ogiltig H.264-parameter från OBS."
			);
			*cursor += length;
		}
		Ok(())
	}
	let count = usize::from(
		*bytes
			.get(5)
			.ok_or_else(|| anyhow::anyhow!("Ofullständig OBS-konfiguration."))?
			& 31,
	);
	let mut cursor = 6;
	consume(bytes, &mut cursor, count)?;
	let count = usize::from(
		*bytes
			.get(cursor)
			.ok_or_else(|| anyhow::anyhow!("OBS saknar PPS."))?,
	);
	cursor += 1;
	consume(bytes, &mut cursor, count)
}
impl ObsDemuxer {
	pub fn video(&mut self, timestamp_ms: u32, flv: &[u8]) -> Result<Option<Encoded>> {
		ensure!(
			(5..=MAX_VIDEO).contains(&flv.len()),
			"OBS-bilden är för stor eller ofullständig. Sänk bithastigheten."
		);
		ensure!(
			flv[0] & 0x80 == 0 && flv[0] & 15 == 7,
			"Välj H.264 i OBS, inte HEVC eller AV1."
		);
		ensure!(
			flv[2..5] == [0, 0, 0],
			"Ställ in B-bilder (B-frames) på 0 i OBS."
		);
		match flv[1] {
			0 => {
				self.avc = None;
				let bytes = &flv[5..];
				ensure!(bytes.len() <= 16384, "För stor OBS-konfiguration.");
				let conf = AvcDecoderConfigurationRecord::try_from(bytes)
					.map_err(|_| anyhow::anyhow!("Ogiltig H.264-konfiguration från OBS."))?;
				validate_parameter_lengths(bytes)?;
				let mut parameters = Vec::new();
				let mut context = Context::new();
				for nal in conf.sequence_parameter_sets() {
					let nal = nal.map_err(|_| anyhow::anyhow!("Ogiltig SPS från OBS."))?;
					let value = sps(nal)?;
					ensure!(
						nal[1..4] == bytes[1..4],
						"OBS skickade olika H.264-profiler."
					);
					context.put_seq_param_set(value);
					parameters.push(nal.to_vec());
				}
				ensure!(
					!parameters.is_empty() && parameters.len() <= 4,
					"OBS saknar H.264-SPS."
				);
				let count = parameters.len();
				for nal in conf.picture_parameter_sets() {
					let nal = nal.map_err(|_| anyhow::anyhow!("Ogiltig PPS från OBS."))?;
					ensure!(
						nal.len() <= 4096 && nal_type(nal)? == 8,
						"Ogiltig H.264-PPS."
					);
					PicParameterSet::from_bits(&context, RefNal::new(nal, &[], true).rbsp_bits())
						.map_err(|_| anyhow::anyhow!("Ogiltig H.264-PPS från OBS."))?;
					parameters.push(nal.to_vec());
				}
				ensure!(
					parameters.len() > count && parameters.len() <= count + 4,
					"OBS saknar H.264-PPS."
				);
				let length_size = usize::from(conf.length_size_minus_one()) + 1;
				ensure!(length_size != 3, "Ogiltig H.264-NAL-längd.");
				self.avc = Some(Avc {
					length_size,
					profile: bytes[1..4].try_into()?,
					parameters,
				});
				Ok(None)
			}
			1 => {
				let conf = self.avc.as_ref().ok_or_else(|| {
					anyhow::anyhow!("Starta om strömningen i OBS: H.264-konfiguration saknas.")
				})?;
				let key = flv[0] >> 4 == 1;
				ensure!(matches!(flv[0] >> 4, 1 | 2), "Ogiltig OBS-bildtyp.");
				let mut data = vec![1, u8::from(key)];
				data.extend(timestamp_ms.to_be_bytes());
				data.extend(conf.profile);
				if key {
					for nal in &conf.parameters {
						data.extend([0, 0, 0, 1]);
						data.extend(nal);
					}
				}
				let mut rest = &flv[5..];
				let mut count = 0;
				while !rest.is_empty() {
					ensure!(
						rest.len() >= conf.length_size,
						"Ofullständig H.264-NAL-längd."
					);
					let len = rest[..conf.length_size]
						.iter()
						.fold(0usize, |v, b| (v << 8) | usize::from(*b));
					rest = &rest[conf.length_size..];
					ensure!(
						len > 0 && len <= rest.len(),
						"Ofullständig H.264-NAL från OBS."
					);
					let nal = &rest[..len];
					nal_type(nal)?;
					data.extend([0, 0, 0, 1]);
					data.extend(nal);
					rest = &rest[len..];
					count += 1;
					ensure!(
						count <= 56 && data.len() <= MAX_VIDEO,
						"OBS-bilden är för stor. Sänk bithastigheten."
					);
				}
				validate_video(&data)?;
				Ok(Some(Encoded {
					kind: Kind::VideoH264,
					data,
				}))
			}
			2 => {
				self.avc = None;
				Ok(None)
			}
			_ => bail!("Okänd H.264-pakettyp från OBS."),
		}
	}
	pub fn audio(&mut self, timestamp_ms: u32, flv: &[u8]) -> Result<Option<Encoded>> {
		ensure!(flv.len() >= 2 && flv[0] >> 4 == 10, "Välj AAC-ljud i OBS.");
		match flv[1] {
			0 => {
				self.aac = false;
				let asc = &flv[2..];
				ensure!(
					asc == [0x11, 0x90] || asc == [0x11, 0x90, 0x56, 0xe5, 0],
					"Välj AAC-LC, 48 kHz och stereo i OBS."
				);
				self.aac = true;
				Ok(None)
			}
			1 => {
				ensure!(
					self.aac,
					"Starta om OBS-strömmen: AAC-konfiguration saknas."
				);
				ensure!(
					(3..=MAX_AUDIO_BYTES - 5).contains(&flv.len()),
					"OBS-ljudpaketet är för stort. Välj 128 kbit/s AAC."
				);
				let mut data = vec![1];
				data.extend(timestamp_ms.to_be_bytes());
				data.extend([0x11, 0x90]);
				data.extend(&flv[2..]);
				validate_audio(&data)?;
				Ok(Some(Encoded {
					kind: Kind::ScreenAac,
					data,
				}))
			}
			_ => bail!("Okänd AAC-pakettyp från OBS."),
		}
	}
}
