pub mod sfx;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
pub const FRAME: usize = 960;
const MAGIC: &[u8; 4] = b"MPL1";
pub fn packet(room: u32, stream: u32, sequence: u16, payload: &[u8]) -> Vec<u8> {
    let mut b = Vec::with_capacity(payload.len() + 14);
    b.extend_from_slice(MAGIC);
    b.extend_from_slice(&room.to_be_bytes());
    b.extend_from_slice(&stream.to_be_bytes());
    b.extend_from_slice(&sequence.to_be_bytes());
    b.extend_from_slice(payload);
    b
}
pub fn unpack(b: &[u8]) -> Option<(u32, u32, u16, &[u8])> {
    if b.len() < 15 || b.len() > 1289 || &b[..4] != MAGIC {
        return None;
    }
    Some((
        u32::from_be_bytes(b[4..8].try_into().ok()?),
        u32::from_be_bytes(b[8..12].try_into().ok()?),
        u16::from_be_bytes([b[12], b[13]]),
        &b[14..],
    ))
}
#[derive(Default)]
pub struct JitterBuffer {
    packets: BTreeMap<u16, Vec<u8>>,
    next: Option<u16>,
}
impl JitterBuffer {
    pub fn push(&mut self, seq: u16, data: Vec<u8>) {
        if self.next.is_none() {
            self.next = Some(seq);
        }
        let distance = seq.wrapping_sub(self.next.unwrap());
        if (32..32768).contains(&distance) {
            self.packets.clear();
            self.next = Some(seq);
        }
        let distance = seq.wrapping_sub(self.next.unwrap());
        if distance < 32 && self.packets.len() < 32 {
            self.packets.entry(seq).or_insert(data);
        }
    }
    pub fn pop(&mut self) -> Option<Vec<u8>> {
        let n = self.next?;
        self.next = Some(n.wrapping_add(1));
        self.packets.remove(&n)
    }
}
struct Speaker {
    jitter: JitterBuffer,
    decoder: opus::Decoder,
    start: Instant,
    last: Instant,
}
pub struct VoiceSession {
    stop: Arc<AtomicBool>,
    active: Arc<AtomicBool>,
    input: Option<cpal::Stream>,
    output: Option<cpal::Stream>,
    worker: Option<thread::JoinHandle<()>>,
    receiver: tokio::task::JoinHandle<()>,
}
impl VoiceSession {
    pub fn speaking(&self) -> bool {
        self.active.load(Ordering::Relaxed)
    }
    /// Capture and mix audio on a server-authorized, TLS-protected QUIC room.
    pub fn connect(
        connection: quinn::Connection,
        room: u32,
        runtime: &tokio::runtime::Handle,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        if connection.max_datagram_size().is_none() {
            return Err("Encrypted voice datagrams unavailable".into());
        }
        let host = cpal::default_host();
        let input_device = host.default_input_device().ok_or("No microphone")?;
        let output_device = host.default_output_device().ok_or("No speaker")?;
        let supports = |c: &cpal::SupportedStreamConfigRange| {
            c.sample_format() == cpal::SampleFormat::F32
                && c.min_sample_rate().0 <= 48000
                && c.max_sample_rate().0 >= 48000
        };
        let input_config = input_device
            .supported_input_configs()?
            .find(supports)
            .ok_or("Microphone must support 48 kHz float samples")?
            .with_sample_rate(cpal::SampleRate(48000))
            .config();
        let output_config = output_device
            .supported_output_configs()?
            .find(supports)
            .ok_or("Speaker must support 48 kHz float samples")?
            .with_sample_rate(cpal::SampleRate(48000))
            .config();
        let input_channels = input_config.channels as usize;
        let output_channels = output_config.channels as usize;
        let (tx, rx) = mpsc::sync_channel::<[f32; FRAME]>(8);
        let playback = Arc::new(Mutex::new(VecDeque::<f32>::with_capacity(FRAME * 8)));
        let queue = playback.clone();
        let active = Arc::new(AtomicBool::new(false));
        let meter = active.clone();
        let mut capture = [0f32; FRAME];
        let mut captured = 0;
        let input = input_device.build_input_stream(
            &input_config,
            move |data: &[f32], _| {
                for channels in data.chunks_exact(input_channels) {
                    capture[captured] = channels.iter().sum::<f32>() / input_channels as f32;
                    captured += 1;
                    if captured == FRAME {
                        let energy = capture.iter().map(|s| s * s).sum::<f32>() / FRAME as f32;
                        meter.store(energy > 0.0004, Ordering::Relaxed);
                        let _ = tx.try_send(capture);
                        captured = 0;
                    }
                }
            },
            |e| eprintln!("Audio capture: {e}"),
            None,
        )?;
        let output = output_device.build_output_stream(
            &output_config,
            move |data: &mut [f32], _| {
                if let Ok(mut q) = queue.try_lock() {
                    for channels in data.chunks_mut(output_channels) {
                        channels.fill(q.pop_front().unwrap_or(0.0));
                    }
                } else {
                    data.fill(0.0);
                }
            },
            |e| eprintln!("Audio playback: {e}"),
            None,
        )?;
        let mut encoder = opus::Encoder::new(48000, opus::Channels::Mono, opus::Application::Voip)?;
        encoder.set_bitrate(opus::Bitrate::Bits(32000))?;
        input.play()?;
        output.play()?;
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let (media_tx, media_rx) = mpsc::sync_channel::<Vec<u8>>(32);
        let receiving = connection.clone();
        let receiver = runtime.spawn(async move {
            while let Ok(bytes) = receiving.read_datagram().await {
                let _ = media_tx.try_send(bytes.to_vec());
            }
        });
        let worker = thread::spawn(move || {
            let mut seq = 0u16;
            let mut bytes = [0u8; 1275];
            let mut pcm = [0f32; FRAME];
            let mut speakers: BTreeMap<u32, Speaker> = BTreeMap::new();
            let mut tick = Instant::now() + Duration::from_millis(20);
            while !stopping.load(Ordering::Relaxed) {
                while let Ok(frame) = rx.try_recv() {
                    if let Ok(n) = encoder.encode_float(&frame, &mut bytes) {
                        let bytes = packet(room, 0, seq, &bytes[..n]);
                        if connection
                            .max_datagram_size()
                            .is_some_and(|max| bytes.len() <= max)
                        {
                            let _ = connection.send_datagram(bytes.into());
                        }
                        seq = seq.wrapping_add(1);
                    }
                }
                // Bound the receive batch so a packet flood cannot starve shutdown or playback.
                for _ in 0..64 {
                    let Ok(incoming) = media_rx.try_recv() else {
                        break;
                    };
                    if let Some((incoming_room, id, seq, payload)) = unpack(&incoming) {
                        if incoming_room != room {
                            continue;
                        }
                        if !speakers.contains_key(&id) && speakers.len() < 10 {
                            if let Ok(decoder) = opus::Decoder::new(48000, opus::Channels::Mono) {
                                speakers.insert(
                                    id,
                                    Speaker {
                                        jitter: JitterBuffer::default(),
                                        decoder,
                                        start: Instant::now() + Duration::from_millis(60),
                                        last: Instant::now(),
                                    },
                                );
                            }
                        }
                        if let Some(s) = speakers.get_mut(&id) {
                            s.jitter.push(seq, payload.to_vec());
                            s.last = Instant::now();
                        }
                    }
                }
                if Instant::now() >= tick {
                    tick = Instant::now() + Duration::from_millis(20);
                    let mut mixed = [0f32; FRAME];
                    speakers.retain(|_, s| s.last.elapsed() < Duration::from_secs(2));
                    for s in speakers.values_mut() {
                        if Instant::now() < s.start {
                            continue;
                        }
                        let frame = s.jitter.pop();
                        if let Ok(n) =
                            s.decoder
                                .decode_float(frame.as_deref().unwrap_or(&[]), &mut pcm, false)
                        {
                            for i in 0..n {
                                mixed[i] += pcm[i];
                            }
                        }
                    }
                    if let Ok(mut q) = playback.lock() {
                        if q.len() + FRAME > FRAME * 8 {
                            q.clear();
                        }
                        q.extend(mixed.iter().map(|s| s.clamp(-1.0, 1.0)));
                    }
                }
                thread::sleep(Duration::from_millis(2));
            }
        });
        Ok(Self {
            stop,
            active,
            input: Some(input),
            output: Some(output),
            worker: Some(worker),
            receiver,
        })
    }
}
impl Drop for VoiceSession {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.receiver.abort();
        self.input.take();
        self.output.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn framing() {
        let p = packet(1, 12, 65535, &[1, 2, 3]);
        assert_eq!(unpack(&p), Some((1, 12, 65535, &[1, 2, 3][..])));
        assert!(unpack(b"bad").is_none());
        assert!(unpack(&vec![0; 1290]).is_none());
    }
    #[test]
    fn reorder_loss_wrap() {
        let mut j = JitterBuffer::default();
        j.push(65535, vec![1]);
        j.push(1, vec![3]);
        j.push(0, vec![2]);
        assert_eq!(j.pop(), Some(vec![1]));
        assert_eq!(j.pop(), Some(vec![2]));
        assert_eq!(j.pop(), Some(vec![3]));
        assert_eq!(j.pop(), None);
        j.push(2, vec![9]);
        assert_eq!(j.pop(), None);
    }
    #[test]
    fn opus_roundtrip() {
        let mut enc =
            opus::Encoder::new(48000, opus::Channels::Mono, opus::Application::Voip).unwrap();
        let pcm: Vec<f32> = (0..FRAME)
            .map(|i| (i as f32 * 0.0576).sin() * 0.1)
            .collect();
        let mut bytes = [0; 1275];
        let n = enc.encode_float(&pcm, &mut bytes).unwrap();
        let mut dec = opus::Decoder::new(48000, opus::Channels::Mono).unwrap();
        let mut out = [0.0; FRAME];
        assert_eq!(
            dec.decode_float(&bytes[..n], &mut out, false).unwrap(),
            FRAME
        );
        assert!(out.iter().all(|s| s.is_finite()));
    }
}
