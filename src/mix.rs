use std::{
    collections::{BTreeMap, VecDeque},
    sync::Arc,
    time::Duration,
};
use win_native_media::audio::aac::AacEncoder;

use crate::{audio::History, replay::Result};

const RATE: u64 = 48_000;
const FRAMES: usize = 1024;
const DELAY_FRAMES: u64 = RATE / 16;
const FRAME_NS: u128 = 1_000_000_000;

pub struct Mix {
    encoder: AacEncoder,
    pending: BTreeMap<u64, Box<[i32; FRAMES * 2]>>,
    rendered_until: u64,
}

fn samples_at(time: Duration) -> u64 {
    (time.as_nanos() * u128::from(RATE) / FRAME_NS) as u64
}

impl Mix {
    pub fn new() -> Result<Self> {
        Ok(Self {
            encoder: AacEncoder::new(RATE as u32, 2, 128_000)?,
            pending: BTreeMap::new(),
            rendered_until: 0,
        })
    }

    pub fn new_history(&self) -> History {
        History {
            name: "Mezcla".into(),
            channels: 2,
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
                let pcm = self
                    .pending
                    .entry(index)
                    .or_insert_with(|| Box::new([0; FRAMES * 2]));
                for (n, frame) in samples.chunks_exact(width).enumerate() {
                    let left = i16::from_le_bytes([frame[0], frame[1]]);
                    let right = if channels == 1 {
                        left
                    } else {
                        i16::from_le_bytes([frame[2], frame[3]])
                    };
                    let at = (offset + n) * 2;
                    pcm[at] = pcm[at].saturating_add(i32::from(left));
                    pcm[at + 1] = pcm[at + 1].saturating_add(i32::from(right));
                }
            }
            sample += count as u64;
            data = rest;
        }
    }

    pub fn render(&mut self, time: Duration, history: &mut History) -> Result<()> {
        let ready = samples_at(time).saturating_sub(DELAY_FRAMES) / FRAMES as u64;
        while self
            .pending
            .first_key_value()
            .is_some_and(|(&index, _)| index < ready)
        {
            let (index, block) = self.pending.pop_first().unwrap();
            self.rendered_until = index + 1;
            let mut pcm = [0; FRAMES * 4];
            for (i, sample) in block.iter().enumerate() {
                let value = (*sample).clamp(i32::from(i16::MIN), i32::from(i16::MAX));
                pcm[i * 2..i * 2 + 2].copy_from_slice(&(value as i16).to_le_bytes());
            }
            let timestamp = Duration::from_nanos(index * FRAMES as u64 * 1_000_000_000 / RATE);
            let mut output = Vec::new();
            self.encoder.encode(&pcm, timestamp, &mut output)?;
            history.packets.extend(output.into_iter().map(Arc::new));
        }
        Ok(())
    }
}
