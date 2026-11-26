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
        let pcm = 1000i16.to_le_bytes().repeat(FRAMES * 16 * 2);
        let at = Duration::from_secs(1);
        mix.add(at, &pcm, 2);
        mix.render(at + Duration::from_millis(110), &mut history)?;

        let mut microphone = crate::microphone::Microphone::new(1);
        let input = (2000.0f32 / 32768.0).to_le_bytes().repeat(480 * 8);
        for block in 0..4 {
            let mut voice = Vec::new();
            let timestamp = microphone
                .process(
                    &input,
                    1,
                    at + Duration::from_millis(block * 80),
                    100,
                    false,
                    &mut voice,
                )
                .expect("Falta el bloque de micrófono");
            mix.add(timestamp, &voice, 1);
        }
        mix.render(at + Duration::from_secs(1), &mut history)?;
        // Each source contributes once: a duplicated application would be
        // 4000, and duplicated microphone blocks would be 5000.
        assert_eq!(history.level, 3000.0 / 32768.0);
        assert!(!history.packets.is_empty(), "Falta audio AAC codificado");
        assert!(
            history
                .packets
                .iter()
                .zip(history.packets.iter().skip(1))
                .all(|(first, next)| first.timestamp < next.timestamp)
        );
        assert!(mix.pending.is_empty());
        Ok(())
    }

    #[test]
    fn una_fuente_estereo_no_se_duplica_al_cruzar_bloques_aac() -> Result<()> {
        let mut mix = Mix::combined()?;
        let mut output = mix.new_history("Una fuente");
        let frame = [1200i16.to_le_bytes(), (-2300i16).to_le_bytes()].concat();
        let pcm = frame.repeat(FRAMES * 16);
        // 7 ms deliberately splits both incoming packets across AAC blocks.
        mix.add(Duration::from_millis(7), &pcm[..777 * 4], 2);
        mix.add(
            Duration::from_nanos(7_000_000 + 777 * 1_000_000_000 / 48_000 + 1),
            &pcm[777 * 4..],
            2,
        );
        mix.render(Duration::from_secs(1), &mut output)?;
        assert_eq!(output.level, 2300.0 / 32768.0);
        assert!(!output.packets.is_empty(), "Falta audio AAC codificado");
        assert!(
            output
                .packets
                .iter()
                .zip(output.packets.iter().skip(1))
                .all(|(first, next)| first.timestamp < next.timestamp)
        );
        let count = output.packets.len();
        mix.render(Duration::from_secs(2), &mut output)?;
        assert_eq!(
            output.packets.len(),
            count,
            "Se repitió audio ya renderizado"
        );
        Ok(())
    }
}
