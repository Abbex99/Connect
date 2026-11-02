//! Bounded inline profile media. No remote URLs or active document formats are accepted.
use anyhow::{Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};

pub const AVATAR_ANIMATION_LIMIT: usize = 16 * 1024;
pub const COVER_ANIMATION_LIMIT: usize = 32 * 1024;

pub fn validate_media(
	value: &str,
	static_limit: usize,
	animation_limit: usize,
	dimension: u32,
	max_seconds: Option<f64>,
) -> Result<()> {
	if value.is_empty() {
		return Ok(());
	}
	if value.starts_with("serein-asset:") {
		crate::protocol::ProfileAssetRef::parse(value)?;
		return Ok(());
	}
	let (header, payload) = value
		.split_once(',')
		.ok_or_else(|| anyhow::anyhow!("Ogiltig mediefil."))?;
	let animated = matches!(header, "data:image/gif;base64" | "data:video/mp4;base64");
	ensure!(
		value.len()
			<= if animated {
				animation_limit
			} else {
				static_limit
			},
		"Mediefilen är för stor. Välj en mindre animation."
	);
	let expected = match header {
		"data:image/png;base64" => Some(image::ImageFormat::Png),
		"data:image/jpeg;base64" => Some(image::ImageFormat::Jpeg),
		"data:image/webp;base64" => Some(image::ImageFormat::WebP),
		"data:image/gif;base64" => Some(image::ImageFormat::Gif),
		"data:video/mp4;base64" => None,
		_ => anyhow::bail!("Välj PNG, JPEG, WebP, GIF eller MP4."),
	};
	let bytes = STANDARD.decode(payload)?;
	if let Some(format) = expected {
		let reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()?;
		ensure!(
			reader.format() == Some(format),
			"Filens innehåll stämmer inte med bildformatet."
		);
		let (width, height) = reader.into_dimensions()?;
		ensure!(
			width > 0 && height > 0 && width <= dimension && height <= dimension,
			"Bilden har för stora mått."
		);
	} else {
		validate_mp4(&bytes, dimension, max_seconds, None)?;
	}
	Ok(())
}

pub const ASSET_LIMIT: usize = 50 * 1024 * 1024;
pub const ASSET_DIMENSION: u32 = 2048;
/// Validate original media before publishing or accepting bytes into the cache.
pub fn validate_asset(mime: &str, bytes: &[u8]) -> Result<()> {
	use image::{AnimationDecoder, ImageDecoder};
	ensure!(
		!bytes.is_empty() && bytes.len() <= ASSET_LIMIT,
		"Profilmedia får vara högst 50 MB."
	);
	if mime == "video/mp4" {
		return validate_mp4(bytes, 4096, Some(300.0), Some(3840 * 2160));
	}
	let format = match mime {
		"image/png" => image::ImageFormat::Png,
		"image/jpeg" => image::ImageFormat::Jpeg,
		"image/webp" => image::ImageFormat::WebP,
		"image/gif" => image::ImageFormat::Gif,
		_ => anyhow::bail!("Välj PNG, JPEG, WebP, GIF eller MP4."),
	};
	let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()?;
	ensure!(
		reader.format() == Some(format),
		"Filens innehåll stämmer inte med bildformatet."
	);
	let mut limits = image::Limits::default();
	limits.max_image_width = Some(ASSET_DIMENSION);
	limits.max_image_height = Some(ASSET_DIMENSION);
	limits.max_alloc = Some(64 * 1024 * 1024);
	reader.limits(limits.clone());
	if format == image::ImageFormat::Gif {
		let mut decoder = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(bytes))?;
		decoder.set_limits(limits)?;
		let mut pixels = 0u64;
		let mut frames = 0usize;
		for frame in decoder.into_frames() {
			frames += 1;
			ensure!(frames <= 600, "GIF får ha högst 600 bildrutor.");
			let frame = frame?;
			pixels += u64::from(frame.buffer().width()) * u64::from(frame.buffer().height());
			ensure!(
				pixels <= 128 * 1024 * 1024,
				"GIF har för många eller för stora bildrutor."
			);
		}
		ensure!(frames > 0, "GIF saknar bildrutor.");
	} else {
		reader.decode()?;
	}
	Ok(())
}
fn u32_at(bytes: &[u8], offset: usize) -> Result<u32> {
	let part = bytes
		.get(offset..offset + 4)
		.ok_or_else(|| anyhow::anyhow!("Avkortad MP4-fil."))?;
	Ok(u32::from_be_bytes(part.try_into()?))
}
fn u64_at(bytes: &[u8], offset: usize) -> Result<u64> {
	let part = bytes
		.get(offset..offset + 8)
		.ok_or_else(|| anyhow::anyhow!("Avkortad MP4-fil."))?;
	Ok(u64::from_be_bytes(part.try_into()?))
}
fn boxes(mut bytes: &[u8]) -> Result<Vec<([u8; 4], &[u8])>> {
	let mut result = Vec::new();
	while !bytes.is_empty() {
		ensure!(bytes.len() >= 8, "Avkortad MP4-fil.");
		let size = u32_at(bytes, 0)?;
		let (length, header) = match size {
			0 => (bytes.len(), 8),
			1 => (usize::try_from(u64_at(bytes, 8)?)?, 16),
			_ => (size as usize, 8),
		};
		ensure!(
			length >= header && length <= bytes.len(),
			"Ogiltig MP4-längd."
		);
		result.push((bytes[4..8].try_into()?, &bytes[header..length]));
		ensure!(result.len() <= 4096, "MP4-filen har för många delar.");
		bytes = &bytes[length..];
	}
	Ok(result)
}
fn validate_mp4(
	bytes: &[u8],
	dimension: u32,
	max_seconds: Option<f64>,
	max_pixels: Option<u64>,
) -> Result<()> {
	let top = boxes(bytes)?;
	let brand = top
		.iter()
		.find(|(kind, _)| kind == b"ftyp")
		.map(|(_, body)| *body)
		.ok_or_else(|| anyhow::anyhow!("MP4-format saknas."))?;
	ensure!(
		brand.len() >= 8 && brand.len() % 4 == 0,
		"Ogiltigt MP4-format."
	);
	ensure!(
		brand.as_chunks::<4>().0.iter().any(|part| matches!(
			part,
			b"isom" | b"iso2" | b"iso4" | b"iso5" | b"iso6" | b"mp41" | b"mp42" | b"avc1" | b"M4V "
		)),
		"MP4-formatet stöds inte."
	);
	ensure!(
		top.iter()
			.any(|(kind, body)| kind == b"mdat" && !body.is_empty()),
		"MP4-videodata saknas."
	);
	let movie = top
		.iter()
		.find(|(kind, _)| kind == b"moov")
		.map(|(_, body)| *body)
		.ok_or_else(|| anyhow::anyhow!("MP4-beskrivning saknas."))?;
	let entries = boxes(movie)?;
	let mut video = false;
	let mut has_duration = false;
	for (kind, body) in entries {
		if &kind == b"mvhd" {
			let version = body.first().copied().unwrap_or(255);
			let (scale, duration) = match version {
				0 => (u32_at(body, 12)?, u32_at(body, 16)? as u64),
				1 => (u32_at(body, 20)?, u64_at(body, 24)?),
				_ => anyhow::bail!("Ogiltig MP4-tidsangivelse."),
			};
			ensure!(
				scale > 0 && (duration > 0 || max_pixels.is_none()),
				"Ogiltig videolängd."
			);
			has_duration = true;
			if let Some(max) = max_seconds {
				ensure!(
					(duration as f64 / scale as f64) <= max,
					"Videon får vara högst {max} sekunder."
				);
			}
		}
		if &kind == b"trak" {
			let track = boxes(body)?;
			let mut visual = false;
			for (child, contents) in &track {
				if child == b"mdia" {
					for (field, data) in boxes(contents)? {
						if &field == b"hdlr" && data.get(8..12) == Some(b"vide") {
							visual = true;
						}
					}
				}
			}
			if visual {
				let header = track
					.iter()
					.find(|(field, _)| field == b"tkhd")
					.map(|(_, data)| *data)
					.ok_or_else(|| anyhow::anyhow!("Videomått saknas."))?;
				ensure!(header.len() >= 84, "Ogiltiga videomått.");
				let width = u32_at(header, header.len() - 8)?;
				let height = u32_at(header, header.len() - 4)?;
				ensure!(
					width > 0
						&& height > 0 && width <= dimension * 65536
						&& height <= dimension * 65536,
					"Videon har för stora mått."
				);
				if let Some(pixels) = max_pixels {
					ensure!(
						u64::from(width) * u64::from(height) <= pixels * 65536 * 65536,
						"Videon får vara högst 4K (8,3 megapixlar)."
					);
				}
				video = true;
			}
		}
	}
	ensure!(video, "MP4-filen behöver innehålla video.");
	// Original-byte assets require a bounded duration; preserve legacy inline handling.
	ensure!(
		has_duration || max_pixels.is_none(),
		"MP4-videons längd saknas."
	);
	Ok(())
}
