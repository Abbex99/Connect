//! Bounded media packets. Capture stays in the UI; the server authenticates and routes.
use crate::protocol::*;
use anyhow::{Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use std::{
	collections::HashMap,
	sync::Mutex,
	time::{Duration, Instant},
};
use tokio::sync::{broadcast, watch};

pub const MAX_VIDEO: usize = 1024 * 1024;
pub const MAX_AUDIO_BYTES: usize = 1000; // 20 ms mono Opus at 48 kHz.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
	Audio,
	Video,
	ScreenAudio,
	VideoH264,
	ScreenAac,
}
impl Kind {
	pub fn is_video(self) -> bool {
		matches!(self, Self::Video | Self::VideoH264)
	}
	pub fn is_audio(self) -> bool {
		!self.is_video()
	}
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Frame {
	pub kind: Kind,
	pub epoch: u32,
	pub sequence: u32,
	#[serde(with = "data64")]
	pub data: Vec<u8>,
}
mod data64 {
	use super::*;
	pub fn serialize<S: serde::Serializer>(v: &[u8], s: S) -> Result<S::Ok, S::Error> {
		s.serialize_str(&STANDARD.encode(v))
	}
	pub fn deserialize<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
		let s = String::deserialize(d)?;
		if s.len() > MAX_VIDEO * 4 / 3 + 8 {
			return Err(serde::de::Error::custom("För stor mediabild."));
		}
		STANDARD.decode(s).map_err(serde::de::Error::custom)
	}
}
impl Frame {
	pub fn validate(&self) -> Result<()> {
		ensure!(self.epoch != 0, "Ogiltigt samtal.");
		match self.kind {
			Kind::VideoH264 => crate::obs_codec::validate_video(&self.data)?,
			Kind::ScreenAac => crate::obs_codec::validate_audio(&self.data)?,
			Kind::Audio => ensure!(
				(1..=MAX_AUDIO_BYTES).contains(&self.data.len()),
				"Ogiltigt ljudpaket."
			),
			Kind::ScreenAudio => {
				ensure!((1..=1000).contains(&self.data.len()), "Ogiltigt skärmljud.")
			}
			Kind::Video => {
				ensure!(
					self.data.len() >= 3 && self.data.len() <= MAX_VIDEO,
					"Ogiltig videostorlek."
				);
				if self.data[0] & 1 == 0 {
					ensure!(
						self.data.len() >= 10 && self.data[3..6] == [0x9d, 0x01, 0x2a],
						"Ogiltig VP8-nyckelbild."
					);
					let w = u16::from_le_bytes([self.data[6], self.data[7]]) & 0x3fff;
					let h = u16::from_le_bytes([self.data[8], self.data[9]]) & 0x3fff;
					ensure!(
						w > 0 && h > 0 && w <= 1920 && h <= 1080,
						"Videon får vara högst 1920 × 1080."
					);
				}
			}
		}
		Ok(())
	}
	pub fn encode(&self) -> Result<Vec<u8>> {
		self.validate()?;
		let mut b = Vec::with_capacity(9 + self.data.len());
		b.push(match self.kind {
			Kind::Audio => 0,
			Kind::Video => 1,
			Kind::ScreenAudio => 2,
			Kind::VideoH264 => 3,
			Kind::ScreenAac => 4,
		});
		b.extend(self.epoch.to_be_bytes());
		b.extend(self.sequence.to_be_bytes());
		b.extend(&self.data);
		Ok(b)
	}
	pub fn decode(b: &[u8]) -> Result<Self> {
		ensure!(
			b.len() >= 9 && b.len() <= MAX_VIDEO + 9,
			"Ogiltigt mediapaket."
		);
		let kind = match b[0] {
			0 => Kind::Audio,
			1 => Kind::Video,
			2 => Kind::ScreenAudio,
			3 => Kind::VideoH264,
			4 => Kind::ScreenAac,
			_ => anyhow::bail!("Okänd media."),
		};
		let frame = Self {
			kind,
			epoch: u32::from_be_bytes(b[1..5].try_into()?),
			sequence: u32::from_be_bytes(b[5..9].try_into()?),
			data: b[9..].to_vec(),
		};
		frame.validate()?;
		Ok(frame)
	}
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Delivery {
	pub sender: String,
	pub channel: String,
	pub epoch: u32,
	pub frame: Frame,
}
impl Delivery {
	pub fn encode(&self) -> Result<Vec<u8>> {
		ensure!(
			self.sender.len() <= 128 && self.channel.len() <= 32,
			"Ogiltig avsändare."
		);
		let mut b = vec![self.sender.len() as u8, self.channel.len() as u8];
		b.extend(self.epoch.to_be_bytes());
		b.extend(self.sender.as_bytes());
		b.extend(self.channel.as_bytes());
		b.extend(self.frame.encode()?);
		Ok(b)
	}
	pub fn decode(b: &[u8]) -> Result<Self> {
		ensure!(b.len() >= 6, "Ofullständig media.");
		let s = b[0] as usize;
		let c = b[1] as usize;
		ensure!(
			s <= 128 && c <= 32 && b.len() >= 6 + s + c + 9,
			"Ogiltig mediarubrik."
		);
		let channel = std::str::from_utf8(&b[6 + s..6 + s + c])?.to_string();
		validate_slug(&channel)?;
		Ok(Self {
			sender: std::str::from_utf8(&b[6..6 + s])?.into(),
			channel,
			epoch: u32::from_be_bytes(b[2..6].try_into()?),
			frame: Frame::decode(&b[6 + s + c..])?,
		})
	}
}
#[derive(Clone)]
pub struct Routed {
	at: Instant,
	sender: String,
	channel: String,
	frame: Frame,
	recipients: Vec<(String, u32)>,
}
struct Rate {
	start: Instant,
	frames: u32,
	bytes: usize,
	epoch: u32,
	last: u32,
}
pub struct Hub {
	snapshot: watch::Receiver<Snapshot>,
	pub audio: broadcast::Sender<Routed>,
	pub screen_audio: broadcast::Sender<Routed>,
	pub video: broadcast::Sender<Routed>,
	rates: Mutex<HashMap<(String, Kind), Rate>>,
}
impl Hub {
	pub fn new(snapshot: watch::Receiver<Snapshot>) -> Self {
		let (audio, _) = broadcast::channel(128);
		let (screen_audio, _) = broadcast::channel(64);
		let (video, _) = broadcast::channel(32);
		Self {
			snapshot,
			audio,
			screen_audio,
			video,
			rates: Mutex::new(HashMap::new()),
		}
	}
	pub fn publish(&self, sender: &str, frame: Frame) -> Result<()> {
		frame.validate()?;
		let snapshot = self.snapshot.borrow();
		let voice = snapshot
			.members
			.iter()
			.find(|m| m.id == sender)
			.and_then(|m| m.voice.as_ref())
			.ok_or_else(|| anyhow::anyhow!("Ingen röstanslutning."))?;
		ensure!(
			if frame.kind == Kind::Audio {
				voice.epoch
			} else {
				voice.screen_epoch
			} == frame.epoch,
			"Inaktuellt samtal."
		);
		ensure!(
			match frame.kind {
				Kind::Audio => !voice.muted && !voice.deafened,
				Kind::Video | Kind::VideoH264 => voice.screen,
				Kind::ScreenAudio | Kind::ScreenAac => voice.screen,
			},
			"Media är avstängd."
		);
		let mut rates = self
			.rates
			.lock()
			.map_err(|_| anyhow::anyhow!("Media upptagen."))?;
		rates.retain(|(id, _), _| snapshot.members.iter().any(|m| &m.id == id));
		let r = rates.entry((sender.into(), frame.kind)).or_insert(Rate {
			start: Instant::now(),
			frames: 0,
			bytes: 0,
			epoch: frame.epoch,
			last: frame.sequence.wrapping_sub(1),
		});
		if r.epoch != frame.epoch {
			r.epoch = frame.epoch;
			r.last = frame.sequence.wrapping_sub(1);
		}
		ensure!(
			frame.sequence.wrapping_sub(r.last) > 0
				&& frame.sequence.wrapping_sub(r.last) < u32::MAX / 2,
			"Inaktuellt mediapaket."
		);
		if r.start.elapsed() >= Duration::from_secs(1) {
			r.start = Instant::now();
			r.frames = 0;
			r.bytes = 0;
		}
		ensure!(
			r.frames < if frame.kind.is_audio() { 75 } else { 45 }
				&& r.bytes + frame.data.len() <= 8 * 1024 * 1024,
			"För hög mediatakt."
		);
		r.frames += 1;
		r.bytes += frame.data.len();
		r.last = frame.sequence;
		let event = Routed {
			at: Instant::now(),
			recipients: snapshot
				.members
				.iter()
				.filter_map(|m| {
					m.voice
						.as_ref()
						.filter(|v| v.channel == voice.channel)
						.map(|v| (m.id.clone(), v.epoch))
				})
				.collect(),
			sender: sender.into(),
			channel: voice.channel.clone(),
			frame,
		};
		let _ = if event.frame.kind == Kind::Audio {
			self.audio.send(event)
		} else if event.frame.kind.is_audio() {
			self.screen_audio.send(event)
		} else {
			self.video.send(event)
		};
		Ok(())
	}
	pub fn delivery(&self, receiver: &str, event: &Routed) -> Option<Delivery> {
		let ttl = if event.frame.kind.is_audio() {
			200
		} else {
			500
		};
		if receiver == event.sender || event.at.elapsed() > Duration::from_millis(ttl) {
			return None;
		}
		let snapshot = self.snapshot.borrow();
		let source = snapshot
			.members
			.iter()
			.find(|m| m.id == event.sender)?
			.voice
			.as_ref()?;
		let target = snapshot
			.members
			.iter()
			.find(|m| m.id == receiver)?
			.voice
			.as_ref()?;
		if (if event.frame.kind == Kind::Audio {
			source.epoch
		} else {
			source.screen_epoch
		}) != event.frame.epoch
			|| source.channel != event.channel
			|| target.channel != event.channel
			|| !event
				.recipients
				.iter()
				.any(|(id, epoch)| id == receiver && *epoch == target.epoch)
		{
			return None;
		}
		if event.frame.kind == Kind::Audio && (source.muted || source.deafened || target.deafened) {
			return None;
		}
		if event.frame.kind.is_video() && !source.screen {
			return None;
		}
		if matches!(event.frame.kind, Kind::ScreenAudio | Kind::ScreenAac)
			&& (!source.screen || target.deafened)
		{
			return None;
		}
		Some(Delivery {
			sender: event.sender.clone(),
			channel: event.channel.clone(),
			epoch: target.epoch,
			frame: event.frame.clone(),
		})
	}
}
pub struct Received {
	pub delivery: Delivery,
	pub at: Instant,
}
// Speech has its own capacity: neither screen audio nor video can evict it.
// Selective consumers never drain the other lane's packets.
#[derive(Clone)]
pub struct MediaSender {
	audio: tokio::sync::mpsc::Sender<Received>,
	screen_audio: tokio::sync::mpsc::Sender<Received>,
	video: tokio::sync::mpsc::Sender<Received>,
}
pub struct MediaInbox {
	audio: tokio::sync::mpsc::Receiver<Received>,
	screen_audio: tokio::sync::mpsc::Receiver<Received>,
	video: tokio::sync::mpsc::Receiver<Received>,
}
pub fn channel() -> (MediaSender, MediaInbox) {
	let (audio_tx, audio) = tokio::sync::mpsc::channel(64);
	let (screen_audio_tx, screen_audio) = tokio::sync::mpsc::channel(32);
	let (video_tx, video) = tokio::sync::mpsc::channel(8);
	(
		MediaSender {
			audio: audio_tx,
			screen_audio: screen_audio_tx,
			video: video_tx,
		},
		MediaInbox {
			audio,
			screen_audio,
			video,
		},
	)
}
impl MediaSender {
	pub fn try_send(
		&self,
		packet: Received,
	) -> Result<(), tokio::sync::mpsc::error::TrySendError<Received>> {
		if packet.delivery.frame.kind.is_video() {
			self.video.try_send(packet)
		} else if packet.delivery.frame.kind == Kind::Audio {
			self.audio.try_send(packet)
		} else {
			self.screen_audio.try_send(packet)
		}
	}
}
impl MediaInbox {
	pub fn try_recv_audio(&mut self) -> Result<Received, tokio::sync::mpsc::error::TryRecvError> {
		self.audio
			.try_recv()
			.or_else(|_| self.screen_audio.try_recv())
	}
	pub fn try_recv_video(&mut self) -> Result<Received, tokio::sync::mpsc::error::TryRecvError> {
		self.video.try_recv()
	}
	pub fn try_recv(&mut self) -> Result<Received, tokio::sync::mpsc::error::TryRecvError> {
		self.try_recv_audio().or_else(|_| self.try_recv_video())
	}
	pub async fn recv(&mut self) -> Option<Received> {
		loop {
			tokio::select! {biased;
			 value=self.audio.recv(), if !self.audio.is_closed() || !self.audio.is_empty()=>if value.is_some() {return value},
			 value=self.screen_audio.recv(), if !self.screen_audio.is_closed() || !self.screen_audio.is_empty()=>if value.is_some() {return value},
			 value=self.video.recv(), if !self.video.is_closed() || !self.video.is_empty()=>if value.is_some() {return value},
			 else=>return None,
			}
		}
	}
}

// Reset on cancellation as well as timeout. Dropping a bare SendStream would
// implicitly finish it and retain its buffered bytes for retransmission.
struct ExpiringVideoStream(iroh::endpoint::SendStream);
impl Drop for ExpiringVideoStream {
	fn drop(&mut self) {
		let _ = self.0.reset(0u32.into());
	}
}
async fn write_video_stream(
	mut stream: ExpiringVideoStream,
	bytes: Vec<u8>,
	deadline: tokio::time::Instant,
) -> Result<()> {
	let result = tokio::time::timeout_at(deadline, async {
		stream.0.set_priority(-10)?;
		stream
			.0
			.write_all(&(bytes.len() as u32).to_be_bytes())
			.await?;
		stream.0.write_all(&bytes).await?;
		stream.0.finish()?;
		ensure!(
			stream.0.stopped().await?.is_none(),
			"Mottagaren avbröt videon."
		);
		Ok::<_, anyhow::Error>(())
	})
	.await;
	result?
}
pub async fn write_blob(connection: &iroh::endpoint::Connection, bytes: Vec<u8>) -> Result<()> {
	let deadline = tokio::time::Instant::now() + Duration::from_millis(600);
	let stream = tokio::time::timeout_at(deadline, connection.open_uni()).await??;
	write_video_stream(ExpiringVideoStream(stream), bytes, deadline).await
}

#[derive(Default)]
struct VideoWindow {
	writes: tokio::task::JoinSet<Result<()>>,
}
impl VideoWindow {
	async fn send(
		&mut self,
		connection: &iroh::endpoint::Connection,
		bytes: Vec<u8>,
	) -> Result<()> {
		let deadline = tokio::time::Instant::now() + Duration::from_millis(600);
		while self.writes.try_join_next().is_some() {}
		if self.writes.len() >= 8 {
			let _ = tokio::time::timeout_at(deadline, self.writes.join_next()).await?;
		}
		ensure!(
			tokio::time::Instant::now() < deadline,
			"Videobilden har löpt ut."
		);
		// Open in source order before spawning: stream IDs retain frame order,
		// while up to eight acknowledgements can be outstanding concurrently.
		let stream = tokio::time::timeout_at(deadline, connection.open_uni()).await??;
		self.writes.spawn(write_video_stream(
			ExpiringVideoStream(stream),
			bytes,
			deadline,
		));
		Ok(())
	}
}

async fn local_forward(
	hub: std::sync::Arc<Hub>,
	peer: String,
	mut events: broadcast::Receiver<Routed>,
	tx: MediaSender,
) {
	loop {
		match events.recv().await {
			Ok(event) => {
				if let Some(delivery) = hub.delivery(&peer, &event) {
					let _ = tx.try_send(Received {
						delivery,
						at: event.at,
					});
				}
			}
			Err(broadcast::error::RecvError::Lagged(_)) => continue,
			Err(_) => break,
		}
	}
}
pub async fn local(hub: std::sync::Arc<Hub>, peer: String, tx: MediaSender) {
	tokio::join!(
		local_forward(hub.clone(), peer.clone(), hub.audio.subscribe(), tx.clone()),
		local_forward(
			hub.clone(),
			peer.clone(),
			hub.screen_audio.subscribe(),
			tx.clone()
		),
		local_forward(hub.clone(), peer, hub.video.subscribe(), tx)
	);
}
pub async fn serve(
	connection: iroh::endpoint::Connection,
	hub: std::sync::Arc<Hub>,
	peer: String,
) -> Result<()> {
	let mut audio = hub.audio.subscribe();
	let mut screen_audio = hub.screen_audio.subscribe();
	let mut video = hub.video.subscribe();
	let audio_in = async {
		loop {
			let b = connection.read_datagram().await?;
			if let Ok(frame) = Frame::decode(&b)
				&& frame.kind.is_audio()
			{
				let _ = hub.publish(&peer, frame);
			}
		}
		#[allow(unreachable_code)]
		Ok::<(), anyhow::Error>(())
	};
	let video_in = async {
		loop {
			let stream = connection.accept_uni().await?;
			if let Ok(b) = read_blob(stream).await
				&& let Ok(frame) = Frame::decode(&b)
				&& frame.kind.is_video()
			{
				let _ = hub.publish(&peer, frame);
			}
		}
		#[allow(unreachable_code)]
		Ok::<(), anyhow::Error>(())
	};
	let audio_out = async {
		loop {
			match audio.recv().await {
				Ok(event) => {
					if let Some(delivery) = hub.delivery(&peer, &event) {
						let _ = connection.send_datagram(delivery.encode()?.into());
					}
				}
				Err(broadcast::error::RecvError::Lagged(_)) => continue,
				Err(_) => break,
			}
		}
		Ok::<(), anyhow::Error>(())
	};
	let video_out = async {
		let mut window = VideoWindow::default();
		loop {
			match video.recv().await {
				Ok(event) => {
					if let Some(delivery) = hub.delivery(&peer, &event) {
						let _ = window.send(&connection, delivery.encode()?).await;
					}
				}
				Err(broadcast::error::RecvError::Lagged(_)) => continue,
				Err(_) => break,
			}
		}
		Ok::<(), anyhow::Error>(())
	};
	let screen_audio_out = async {
		loop {
			match screen_audio.recv().await {
				Ok(event) => {
					if let Some(delivery) = hub.delivery(&peer, &event) {
						// Unlike send_datagram, this never evicts already buffered voice.
						let _ = tokio::time::timeout(
							Duration::from_millis(20),
							connection.send_datagram_wait(delivery.encode()?.into()),
						)
						.await;
					}
				}
				Err(broadcast::error::RecvError::Lagged(_)) => continue,
				Err(_) => break,
			}
		}
		Ok::<(), anyhow::Error>(())
	};
	tokio::select! {r=audio_in=>r,r=video_in=>r,r=audio_out=>r,r=screen_audio_out=>r,r=video_out=>r}
}
pub async fn client(
	connection: iroh::endpoint::Connection,
	mut audio: tokio::sync::mpsc::Receiver<Frame>,
	mut screen_audio: tokio::sync::mpsc::Receiver<Frame>,
	mut video: tokio::sync::mpsc::Receiver<Frame>,
	tx: MediaSender,
) -> Result<()> {
	let audio_in = async {
		loop {
			let b = connection.read_datagram().await?;
			if let Ok(delivery) = Delivery::decode(&b)
				&& delivery.frame.kind.is_audio()
			{
				let _ = tx.try_send(Received {
					delivery,
					at: Instant::now(),
				});
			}
		}
		#[allow(unreachable_code)]
		Ok::<(), anyhow::Error>(())
	};
	let video_in = async {
		loop {
			let stream = connection.accept_uni().await?;
			if let Ok(b) = read_blob(stream).await
				&& let Ok(delivery) = Delivery::decode(&b)
				&& delivery.frame.kind.is_video()
			{
				let _ = tx.try_send(Received {
					delivery,
					at: Instant::now(),
				});
			}
		}
		#[allow(unreachable_code)]
		Ok::<(), anyhow::Error>(())
	};
	let audio_out = async {
		while let Some(frame) = audio.recv().await {
			let _ = connection.send_datagram(frame.encode()?.into());
		}
		Ok::<(), anyhow::Error>(())
	};
	let video_out = async {
		let mut window = VideoWindow::default();
		while let Some(frame) = video.recv().await {
			let _ = window.send(&connection, frame.encode()?).await;
		}
		Ok::<(), anyhow::Error>(())
	};
	let screen_audio_out = async {
		while let Some(frame) = screen_audio.recv().await {
			let _ = tokio::time::timeout(
				Duration::from_millis(20),
				connection.send_datagram_wait(frame.encode()?.into()),
			)
			.await;
		}
		Ok::<(), anyhow::Error>(())
	};
	tokio::select! {r=audio_in=>r,r=video_in=>r,r=audio_out=>r,r=screen_audio_out=>r,r=video_out=>r}
}
pub async fn read_blob(mut stream: iroh::endpoint::RecvStream) -> Result<Vec<u8>> {
	tokio::time::timeout(Duration::from_millis(800), async {
		let mut len = [0; 4];
		stream.read_exact(&mut len).await?;
		let len = u32::from_be_bytes(len) as usize;
		ensure!(
			(9..=MAX_VIDEO + 180).contains(&len),
			"Ogiltig videostorlek."
		);
		let mut b = vec![0; len];
		stream.read_exact(&mut b).await?;
		Ok::<_, anyhow::Error>(b)
	})
	.await?
}

#[cfg(test)]
mod inbox_tests {
	use super::*;
	#[tokio::test]
	async fn two_screen_audio_publishers_cannot_evict_routed_voice_or_keep_it_past_ttl() {
		let snapshot = Snapshot {
			members: ["a", "b"]
				.into_iter()
				.map(|id| Member {
					id: id.into(),
					host: false,
					profile: Profile {
						name: id.into(),
						accent: [1, 2, 3],
						card: None,
					},
					voice: Some(VoiceState {
						channel: "call".into(),
						epoch: 1,
						muted: false,
						deafened: false,
						screen: true,
						screen_epoch: 2,
					}),
				})
				.collect(),
			..Default::default()
		};
		let (_updates, snapshot) = watch::channel(snapshot);
		let hub = Hub::new(snapshot);
		let mut voice = hub.audio.subscribe();
		hub.publish(
			"a",
			Frame {
				kind: Kind::Audio,
				epoch: 1,
				sequence: 0,
				data: vec![1],
			},
		)
		.unwrap();
		for id in ["a", "b"] {
			for sequence in 0..70 {
				hub.publish(
					id,
					Frame {
						kind: Kind::ScreenAudio,
						epoch: 2,
						sequence,
						data: vec![1],
					},
				)
				.unwrap();
			}
		}
		let event = voice
			.try_recv()
			.expect("screen audio must not evict queued speech in the routing hub");
		assert_eq!(hub.delivery("b", &event).unwrap().frame.kind, Kind::Audio);
		tokio::time::sleep(Duration::from_millis(220)).await;
		assert!(
			hub.delivery("b", &event).is_none(),
			"buffered speech must expire at the routing hub"
		);
	}
	#[tokio::test]
	async fn video_sender_keeps_a_bounded_window_in_flight_without_waiting_per_frame() {
		use iroh::{
			Endpoint,
			endpoint::{QuicTransportConfig, presets},
		};
		let receiver = Endpoint::builder(presets::Minimal)
			.alpns(vec![b"window-test".to_vec()])
			.clear_ip_transports()
			.bind_addr("127.0.0.1:0")
			.unwrap()
			.transport_config(
				QuicTransportConfig::builder()
					.stream_receive_window(1024u32.into())
					.build(),
			)
			.bind()
			.await
			.unwrap();
		let sender = Endpoint::builder(presets::Minimal)
			.clear_ip_transports()
			.bind_addr("127.0.0.1:0")
			.unwrap()
			.bind()
			.await
			.unwrap();
		let (outbound, inbound) =
			tokio::join!(sender.connect(receiver.addr(), b"window-test"), async {
				receiver.accept().await.unwrap().await
			});
		let inbound = inbound.unwrap();
		let (_audio_tx, audio) = tokio::sync::mpsc::channel(1);
		let (_screen_tx, screen) = tokio::sync::mpsc::channel(1);
		let (video_tx, video) = tokio::sync::mpsc::channel(16);
		let (media_tx, _inbox) = channel();
		let task = tokio::spawn(client(outbound.unwrap(), audio, screen, video, media_tx));
		for sequence in 0..9 {
			video_tx
				.send(Frame {
					kind: Kind::Video,
					epoch: 1,
					sequence,
					data: vec![1; 64 * 1024],
				})
				.await
				.unwrap();
		}
		let mut streams = Vec::new();
		let window = tokio::time::timeout(Duration::from_millis(300), async {
			for _ in 0..8 {
				streams.push(inbound.accept_uni().await.unwrap());
			}
		})
		.await;
		assert!(
			window.is_ok(),
			"video throughput serialized behind one acknowledgement"
		);
		assert!(
			tokio::time::timeout(Duration::from_millis(50), inbound.accept_uni())
				.await
				.is_err(),
			"more than eight video streams in flight"
		);
		if let Ok(Ok(mut ninth)) =
			tokio::time::timeout(Duration::from_millis(900), inbound.accept_uni()).await
		{
			// Its lifetime began while waiting behind the first eight, so this
			// stream cannot get another full lifetime after obtaining capacity.
			tokio::time::sleep(Duration::from_millis(100)).await;
			assert!(
				tokio::time::timeout(Duration::from_secs(2), ninth.read_to_end(MAX_VIDEO))
					.await
					.unwrap()
					.is_err(),
				"capacity wait was excluded from video lifetime"
			);
		}
		task.abort();
		let _ = task.await;
		for mut stream in streams {
			assert!(
				tokio::time::timeout(Duration::from_secs(2), stream.read_to_end(MAX_VIDEO))
					.await
					.unwrap()
					.is_err(),
				"cancelling a video writer must reset its in-flight streams"
			);
		}
		sender.close().await;
		receiver.close().await;
	}
	#[tokio::test]
	async fn screen_audio_overload_cannot_evict_or_delay_voice() {
		let (tx, mut inbox) = channel();
		for sequence in 0..256 {
			let _ = tx.try_send(received(Kind::ScreenAac, sequence));
		}
		assert!(
			tx.try_send(received(Kind::Audio, 999)).is_ok(),
			"screen audio consumed the voice capacity"
		);
		assert_eq!(inbox.recv().await.unwrap().delivery.frame.kind, Kind::Audio);
	}
	#[tokio::test]
	async fn stalled_video_must_expire_and_reset_unacknowledged_bytes() {
		use iroh::{
			Endpoint,
			endpoint::{QuicTransportConfig, presets},
		};
		let receiver = Endpoint::builder(presets::Minimal)
			.alpns(vec![b"media-test".to_vec()])
			.clear_ip_transports()
			.bind_addr("127.0.0.1:0")
			.unwrap()
			.transport_config(
				QuicTransportConfig::builder()
					.stream_receive_window(1024u32.into())
					.build(),
			)
			.bind()
			.await
			.unwrap();
		let sender = Endpoint::builder(presets::Minimal)
			.clear_ip_transports()
			.bind_addr("127.0.0.1:0")
			.unwrap()
			.bind()
			.await
			.unwrap();
		let (outbound, inbound) =
			tokio::join!(sender.connect(receiver.addr(), b"media-test"), async {
				receiver.accept().await.unwrap().await
			});
		let outbound = outbound.unwrap();
		let inbound = inbound.unwrap();
		let result = write_blob(&outbound, vec![1; 64 * 1024]).await;
		assert!(
			result.is_err(),
			"a buffered write is not an acknowledged video delivery"
		);
		let mut stream = inbound.accept_uni().await.unwrap();
		let result = tokio::time::timeout(Duration::from_secs(2), stream.read_to_end(MAX_VIDEO))
			.await
			.unwrap();
		assert!(
			result.is_err(),
			"expired video must reset rather than remain queued for retransmission"
		);
		outbound.close(0u32.into(), b"done");
		sender.close().await;
		receiver.close().await;
	}
	fn received(kind: Kind, sequence: u32) -> Received {
		Received {
			delivery: Delivery {
				sender: "peer".into(),
				channel: "call".into(),
				epoch: 1,
				frame: Frame {
					kind,
					epoch: 2,
					sequence,
					data: vec![1],
				},
			},
			at: Instant::now(),
		}
	}
	#[tokio::test]
	async fn saturated_video_inbox_preserves_and_prioritizes_voice() {
		let (tx, mut inbox) = channel();
		let mut queued = 0;
		for sequence in 0..100 {
			if tx.try_send(received(Kind::Video, sequence)).is_ok() {
				queued += 1;
			}
		}
		assert_eq!(queued, 8);
		tx.try_send(received(Kind::Audio, 100)).unwrap();
		assert_eq!(inbox.try_recv().unwrap().delivery.frame.kind, Kind::Audio);
		tx.try_send(received(Kind::Audio, 101)).unwrap();
		assert_eq!(inbox.recv().await.unwrap().delivery.frame.kind, Kind::Audio);
		drop(tx);
		for _ in 0..8 {
			assert_eq!(inbox.recv().await.unwrap().delivery.frame.kind, Kind::Video);
		}
		assert!(inbox.recv().await.is_none());
	}
	#[tokio::test]
	async fn saturated_h264_inbox_cannot_evict_voice_or_aac() {
		let (tx, mut inbox) = channel();
		for sequence in 0..8 {
			tx.try_send(received(Kind::VideoH264, sequence)).unwrap();
		}
		assert!(tx.try_send(received(Kind::VideoH264, 9)).is_err());
		tx.try_send(received(Kind::Audio, 10)).unwrap();
		tx.try_send(received(Kind::ScreenAac, 11)).unwrap();
		assert_eq!(inbox.recv().await.unwrap().delivery.frame.kind, Kind::Audio);
		assert_eq!(
			inbox.recv().await.unwrap().delivery.frame.kind,
			Kind::ScreenAac
		);
		assert_eq!(
			inbox.recv().await.unwrap().delivery.frame.kind,
			Kind::VideoH264
		);
	}
}
