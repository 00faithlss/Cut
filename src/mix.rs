use std::{
    collections::{BTreeMap, VecDeque},
    sync::Arc,
    time::Duration,
};
use win_native_media::audio::{EncodedAudio, aac::AacEncoder};

use crate::{audio::History, replay::Result};

const RATE: u64 = 48_000;
const FRAMES: usize = 1024;
const DELAY_FRAMES: u64 = RATE / 16;
const MIX_DELAY_FRAMES: u64 = RATE / 5;
const FRAME_NS: u128 = 1_000_000_000;

pub struct Mix {
    channels: u16,
    encoder: AacEncoder,
    pending: BTreeMap<u64, Box<[i32; FRAMES * 2]>>,
    spare: Option<Box<[i32; FRAMES * 2]>>,
    rendered_until: u64,
    encoded: Vec<EncodedAudio>,
    delay_frames: u64,
}

fn samples_at(time: Duration) -> u64 {
    (time.as_nanos() * u128::from(RATE) / FRAME_NS) as u64
}

impl Mix {
    pub fn new(channels: u16) -> Result<Self> {
        Ok(Self {
            channels,
            encoder: AacEncoder::new(
                RATE as u32,
                channels,
                if channels == 1 { 96_000 } else { 128_000 },
            )?,
            pending: BTreeMap::new(),
            spare: None,
            rendered_until: 0,
            encoded: Vec::new(),
            delay_frames: DELAY_FRAMES,
        })
    }

    pub fn combined() -> Result<Self> {
        let mut mix = Self::new(2)?;
        // Espera al micrófono aunque otra aplicación entregue primero su bloque.
        mix.delay_frames = MIX_DELAY_FRAMES;
        Ok(mix)
    }

    pub fn new_history(&self, name: &str) -> History {
        History {
            name: name.into(),
            channels: self.channels,
            packets: VecDeque::new(),
            level: 0.0,
        }
    }

    pub fn add(&mut self, timestamp: Duration, mut data: &[u8], channels: u16) {
        let mut sample = samples_at(timestamp);
        let width = usize::from(channels) * 2;
        while data.len() >= width {
            let index = sample / FRAMES as u64;
            let offset = (sample % FRAMES as u64) as usize;
            let count = (FRAMES - offset).min(data.len() / width);
            let (samples, rest) = data.split_at(count * width);
            if index >= self.rendered_until {
                let pcm = self.pending.entry(index).or_insert_with(|| {
                    self.spare
                        .take()
                        .unwrap_or_else(|| Box::new([0; FRAMES * 2]))
                });
                for (n, frame) in samples.chunks_exact(width).enumerate() {
                    let left = i16::from_le_bytes([frame[0], frame[1]]);
                    let right = if channels == 1 {
                        left
                    } else {
                        i16::from_le_bytes([frame[2], frame[3]])
                    };
                    let at = (offset + n) * usize::from(self.channels);
                    pcm[at] = pcm[at].saturating_add(i32::from(left));
                    if self.channels == 2 {
                        pcm[at + 1] = pcm[at + 1].saturating_add(i32::from(right));
                    }
                }
            }
            sample += count as u64;
            data = rest;
        }
    }

    pub fn render(&mut self, time: Duration, history: &mut History) -> Result<()> {
        let ready = samples_at(time).saturating_sub(self.delay_frames) / FRAMES as u64;
        let mut peak = 0;
        let samples = FRAMES * usize::from(self.channels);
        while self
            .pending
            .first_key_value()
            .is_some_and(|(&index, _)| index < ready)
        {
            let (index, mut block) = self.pending.pop_first().unwrap();
            self.rendered_until = index + 1;
            let mut pcm = [0; FRAMES * 4];
            for (i, sample) in block[..samples].iter().enumerate() {
                let value = (*sample).clamp(i32::from(i16::MIN), i32::from(i16::MAX));
                peak = peak.max(value.unsigned_abs());
                pcm[i * 2..i * 2 + 2].copy_from_slice(&(value as i16).to_le_bytes());
            }
            if self.spare.is_none() {
                block[..samples].fill(0);
                self.spare = Some(block);
            }
            let timestamp = Duration::from_nanos(
                (u128::from(index) * FRAMES as u128 * FRAME_NS / u128::from(RATE)) as u64,
            );
            self.encoder
                .encode(&pcm[..samples * 2], timestamp, &mut self.encoded)?;
            history.packets.extend(self.encoded.drain(..).map(Arc::new));
        }
        history.level = (history.level * 0.9).max(f64::from(peak) / 32768.0);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conserva_el_reloj_aac_despues_de_cinco_dias() -> Result<()> {
        let mut mix = Mix::new(2)?;
        let mut history = mix.new_history("Prueba");
        let at = Duration::from_secs(5 * 24 * 3600);
        mix.add(at, &vec![0; FRAMES * 4 * 12], 2);
        mix.render(at + Duration::from_secs(1), &mut history)?;
        let last = history.packets.back().expect("Falta audio codificado");
        assert!(last.timestamp >= at && last.timestamp < at + Duration::from_secs(1));
        Ok(())
    }

    #[test]
    fn mezcla_conserva_microfono_que_llega_despues_del_audio_de_aplicacion() -> Result<()> {
        let mut mix = Mix::combined()?;
        let mut history = mix.new_history("Mezcla");
        let pcm = |value: i16| value.to_le_bytes().repeat(FRAMES * 2);
        let at = Duration::from_secs(1);
        mix.add(at, &pcm(100), 2);
        mix.render(at + Duration::from_millis(110), &mut history)?;
        mix.add(at, &pcm(200), 2);
        let first = mix.pending.first_key_value().unwrap().1;
        assert_eq!(first[(samples_at(at) as usize % FRAMES) * 2], 300);
        mix.render(at + Duration::from_millis(300), &mut history)?;
        assert!(mix.pending.is_empty());
        Ok(())
    }
}
