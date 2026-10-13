//! Quiet synthesized earcons. No assets, microphone access, or blocking UI calls.
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};

#[derive(Clone, Copy, Debug)]
pub enum Cue {
    Navigate,
    Send,
    Receive,
    Welcome,
    Goodbye,
    VoiceOn,
    VoiceOff,
    Error,
    Toggle,
    Type,
}
impl Cue {
    fn notes(self) -> &'static [f32] {
        match self {
            Self::Navigate => &[440.0],
            Self::Send => &[523.25, 783.99],
            Self::Receive => &[659.25, 523.25],
            Self::Welcome => &[392.0, 523.25, 659.25],
            Self::Goodbye => &[523.25, 392.0],
            Self::VoiceOn => &[330.0, 440.0, 660.0],
            Self::VoiceOff => &[440.0, 330.0],
            Self::Error => &[220.0, 207.65],
            Self::Toggle => &[587.33],
            Self::Type => &[165.0],
        }
    }
}
fn synth(cue: Cue, rate: u32) -> Vec<f32> {
    let length = (rate as f32
        * if matches!(cue, Cue::Type) {
            0.025
        } else {
            0.09
        }) as usize;
    cue.notes()
        .iter()
        .flat_map(|frequency| {
            (0..length).map(move |i| {
                let t = i as f32 / rate as f32;
                let envelope = (t / 0.008).min(1.0) * (1.0 - i as f32 / length as f32).powi(3);
                let phase = std::f32::consts::TAU * frequency * t;
                (if matches!(cue, Cue::Type) {
                    0.018
                } else {
                    0.075
                }) * envelope
                    * (phase.sin() + 0.2 * (phase * 2.0).sin())
            })
        })
        .collect()
}
pub struct Sounds {
    sender: mpsc::SyncSender<Cue>,
    _lifetime: mpsc::Sender<()>,
    muted: Arc<AtomicBool>,
}
impl Sounds {
    pub fn new(muted: bool) -> Self {
        let (sender, receiver) = mpsc::sync_channel::<Cue>(8);
        let muted = Arc::new(AtomicBool::new(muted));
        let state = muted.clone();
        let (lifetime, shutdown) = mpsc::channel::<()>();
        // The stream stays on its owning thread, including on platforms where it is !Send.
        std::thread::spawn(move || {
            let Some(device) = cpal::default_host().default_output_device() else {
                return;
            };
            let Ok(config) = device.default_output_config() else {
                return;
            };
            let rate = config.sample_rate().0;
            let channels = config.channels() as usize;
            let cues = [
                Cue::Navigate,
                Cue::Send,
                Cue::Receive,
                Cue::Welcome,
                Cue::Goodbye,
                Cue::VoiceOn,
                Cue::VoiceOff,
                Cue::Error,
                Cue::Toggle,
                Cue::Type,
            ];
            let clips: Vec<_> = cues.iter().map(|cue| synth(*cue, rate)).collect();
            let mut selected = None;
            let mut position = 0;
            let mut next = move || {
                if state.load(Ordering::Relaxed) {
                    while receiver.try_recv().is_ok() {}
                    selected = None;
                    position = 0;
                    return 0.0;
                }
                if position >= selected.map_or(0, |i: usize| clips[i].len()) {
                    selected = None;
                    position = 0;
                    if let Ok(cue) = receiver.try_recv() {
                        selected = Some(cue as usize);
                    }
                }
                let value = selected
                    .and_then(|i| clips[i].get(position))
                    .copied()
                    .unwrap_or(0.0);
                position += 1;
                value
            };
            macro_rules! stream {
                ($ty:ty) => {
                    device.build_output_stream(
                        &config.config(),
                        move |data: &mut [$ty], _| {
                            for frame in data.chunks_mut(channels) {
                                let sample = <$ty as cpal::FromSample<f32>>::from_sample_(next());
                                frame.fill(sample);
                            }
                        },
                        |_| {},
                        None,
                    )
                };
            }
            let stream = match config.sample_format() {
                cpal::SampleFormat::F32 => stream!(f32),
                cpal::SampleFormat::I16 => stream!(i16),
                cpal::SampleFormat::U16 => stream!(u16),
                _ => return,
            };
            let Ok(stream) = stream else { return };
            if stream.play().is_err() {
                return;
            }
            // A separate lifetime channel avoids polling or tying playback to the UI thread.
            let _ = shutdown.recv();
        });
        Self {
            sender,
            muted,
            _lifetime: lifetime,
        }
    }
    pub fn play(&self, cue: Cue) {
        if !self.muted.load(Ordering::Relaxed) {
            let _ = self.sender.try_send(cue);
        }
    }
    pub fn set_muted(&self, muted: bool) {
        self.muted.store(muted, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn earcons_are_quiet_finite_and_fade_out() {
        for cue in [
            Cue::Navigate,
            Cue::Send,
            Cue::Receive,
            Cue::Welcome,
            Cue::Goodbye,
            Cue::VoiceOn,
            Cue::VoiceOff,
            Cue::Error,
            Cue::Toggle,
            Cue::Type,
        ] {
            let samples = synth(cue, 48000);
            assert!(samples.iter().all(|s| s.is_finite() && s.abs() <= 0.09));
            assert_eq!(samples[0], 0.0);
            assert!(samples.last().unwrap().abs() < 0.00001);
        }
    }
}
