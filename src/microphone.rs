use nnnoiseless::DenoiseState;
use std::time::Duration;

const SAMPLES: usize = DenoiseState::FRAME_SIZE;
const PERIOD: Duration = Duration::from_millis(10);

struct Channel {
    filter: Option<Box<DenoiseState<'static>>>,
    input: [f32; SAMPLES],
    previous_input: [f32; SAMPLES],
    output: [f32; SAMPLES],
    gain: f32,
    presence: f32,
    noise_gain: f32,
    noise_floor: Option<f32>,
    previous_gain: f32,
}

impl Channel {
    fn new() -> Self {
        Self {
            filter: None,
            input: [0.; SAMPLES],
            previous_input: [0.; SAMPLES],
            output: [0.; SAMPLES],
            gain: 1.35,
            presence: 0.,
            noise_gain: 0.12,
            noise_floor: None,
            previous_gain: 1.35 * 0.12,
        }
    }

    fn filter(&mut self) {
        self.previous_gain = self.gain * self.noise_gain;
        let speech = self
            .filter
            .get_or_insert_with(DenoiseState::new)
            .process_frame(&mut self.output, &self.input);
        self.presence = speech.max(self.presence * 0.92);
        let energy = self
            .output
            .iter()
            .map(|sample| sample * sample)
            .sum::<f32>()
            / SAMPLES as f32;
        let level = energy.sqrt();
        let floor = self.noise_floor.get_or_insert(level.clamp(30., 460.));
        if level < *floor || speech < 0.6 {
            let rate = if level < *floor { 0.12 } else { 0.025 };
            *floor += (level.clamp(30., 460.) - *floor) * rate;
        }
        let above_floor = ((level / *floor - 1.55) / 1.4).clamp(0., 1.);
        let voice = ((self.presence - 0.20) / 0.45).clamp(0., 1.);
        let confirmed = ((self.presence - 0.84) / 0.13).clamp(0., 1.);
        let target = 0.10 + 0.90 * (above_floor * voice).max(confirmed);
        let speed = if target > self.noise_gain { 0.9 } else { 0.24 };
        self.noise_gain += (target - self.noise_gain) * speed;
        if speech >= 0.65 && level > 100. && self.noise_gain >= 0.65 {
            let target = (2200. / level).clamp(0.7, 2.6);
            self.gain += (target - self.gain) * 0.025;
        }
    }
}

pub struct Microphone {
    channels: Vec<Channel>,
    position: usize,
    timestamp: Option<Duration>,
    primed: bool,
    was_suppressed: bool,
    energy: [f32; 2],
    correlation: f32,
}

impl Microphone {
    pub fn new(channels: u16) -> Self {
        Self {
            channels: (0..channels).map(|_| Channel::new()).collect(),
            position: 0,
            timestamp: None,
            primed: false,
            was_suppressed: true,
            energy: [0.; 2],
            correlation: 0.,
        }
    }

    pub fn process(
        &mut self,
        data: &[u8],
        inputs: u16,
        timestamp: Duration,
        volume: u16,
        suppress: bool,
        pcm: &mut Vec<u8>,
    ) -> Option<Duration> {
        let gain = f32::from(volume.min(200)) / 100.;
        if let Some(last) = self.timestamp {
            let pending = Duration::from_nanos(self.position as u64 * 1_000_000_000 / 48_000);
            let expected = last + pending + if self.primed { PERIOD } else { Duration::ZERO };
            if expected.abs_diff(timestamp) > Duration::from_millis(80) {
                *self = Self::new(self.channels.len() as u16);
            }
        }
        self.timestamp.get_or_insert(timestamp);
        let mut first = None;
        for samples in data.chunks_exact(usize::from(inputs) * 4) {
            let read = |bytes: &[u8]| {
                let value = f32::from_le_bytes(bytes.try_into().unwrap());
                if value.is_finite() {
                    value.clamp(-1., 1.) * 32768.
                } else {
                    0.
                }
            };
            let left = read(&samples[..4]);
            let right = if inputs == 1 {
                left
            } else {
                read(&samples[4..8])
            };
            if self.channels.len() == 1 && inputs > 1 {
                self.energy[0] += (left * left - self.energy[0]) * 0.002;
                self.energy[1] += (right * right - self.energy[1]) * 0.002;
                self.correlation += (left * right - self.correlation) * 0.002;
                let opposite_phase = self.correlation < 0.
                    && self.correlation.powi(2) > 0.25 * self.energy[0] * self.energy[1];
                // Un único canal real no pierde nivel al centrarlo.
                let center = if self.energy[1] > self.energy[0] * 6.25 {
                    right
                } else if self.energy[0] > self.energy[1] * 6.25 || opposite_phase {
                    left
                } else {
                    (left + right) * 0.5
                };
                self.channels[0].input[self.position] = center;
            } else {
                for (channel, value) in self.channels.iter_mut().zip([left, right]) {
                    channel.input[self.position] = value;
                }
            }
            self.position += 1;
            if self.position != SAMPLES {
                continue;
            }
            self.position = 0;
            for channel in &mut self.channels {
                if suppress {
                    if !self.was_suppressed && self.primed {
                        channel
                            .filter
                            .get_or_insert_with(DenoiseState::new)
                            .process_frame(&mut channel.output, &channel.previous_input);
                    }
                    channel.filter();
                } else {
                    channel.filter = None;
                    channel.output = channel.previous_input;
                }
                channel.previous_input = channel.input;
            }
            self.was_suppressed = suppress;

            // RNNoise necesita un bloque inicial para el solapamiento.
            if !self.primed {
                self.primed = true;
                continue;
            }
            let output_time = self.timestamp.unwrap();
            first.get_or_insert(output_time);
            self.timestamp = Some(output_time + PERIOD);
            for index in 0..SAMPLES {
                let progress = (index + 1) as f32 / SAMPLES as f32;
                for channel in &self.channels {
                    let target = channel.gain * channel.noise_gain;
                    let smooth_gain =
                        channel.previous_gain + (target - channel.previous_gain) * progress;
                    let value =
                        channel.output[index] * if suppress { smooth_gain * gain } else { gain };
                    let quantized = value.round().clamp(-32768., 32767.) as i16;
                    pcm.extend_from_slice(&quantized.to_le_bytes());
                }
            }
        }
        first
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reduce_ruido_continuo_y_compensa_latencia_del_primer_bloque() {
        let mut mic = Microphone::new(1);
        let mut input = Vec::new();
        let mut random = 0x40529u32;
        for _ in 0..SAMPLES * 80 {
            random ^= random << 13;
            random ^= random >> 17;
            random ^= random << 5;
            let noise = (random as f32 / u32::MAX as f32 - 0.5) * 0.04;
            input.extend_from_slice(&noise.to_le_bytes());
        }
        let mut pcm = Vec::new();
        let timestamp = Duration::from_secs(2);
        assert_eq!(
            mic.process(&input, 1, timestamp, 80, true, &mut pcm),
            Some(timestamp)
        );
        assert_eq!(pcm.len(), SAMPLES * 79 * 2);
        let tail = &pcm[pcm.len() - SAMPLES * 20 * 2..];
        let rms = (tail
            .as_chunks::<2>()
            .0
            .iter()
            .map(|sample| f64::from(i16::from_le_bytes(*sample)).powi(2))
            .sum::<f64>()
            / (SAMPLES * 20) as f64)
            .sqrt();
        assert!(rms < 130.0, "Ruido residual RMS {rms}");
    }

    #[test]
    fn alterna_supresion_sin_retener_el_filtro_ni_perder_audio() {
        let mut mic = Microphone::new(1);
        let input = 0.125f32.to_le_bytes().repeat(SAMPLES * 2);
        for (index, enabled) in [false, true, false, true].into_iter().enumerate() {
            let mut pcm = Vec::new();
            let at = Duration::from_millis(index as u64 * 20);
            assert_eq!(
                mic.process(&input, 1, at, 100, enabled, &mut pcm),
                Some(at.saturating_sub(PERIOD))
            );
            assert_eq!(
                pcm.len(),
                if index == 0 { SAMPLES * 2 } else { SAMPLES * 4 }
            );
            assert_eq!(mic.channels[0].filter.is_some(), enabled);
        }
    }

    #[test]
    fn desactivar_supresion_conserva_el_audio_directo_y_el_reloj() {
        let mut mic = Microphone::new(1);
        let input = 0.125f32.to_le_bytes().repeat(SAMPLES * 2);
        let timestamp = Duration::from_secs(2);
        let mut pcm = Vec::new();
        assert_eq!(
            mic.process(&input, 1, timestamp, 100, false, &mut pcm),
            Some(timestamp)
        );
        assert_eq!(pcm.len(), SAMPLES * 2);
        assert!(
            pcm.as_chunks::<2>()
                .0
                .iter()
                .all(|sample| i16::from_le_bytes(*sample) == 4096)
        );
        pcm.clear();
        assert_eq!(
            mic.process(&input, 1, timestamp + PERIOD * 2, 100, true, &mut pcm),
            Some(timestamp + PERIOD)
        );
        assert_eq!(pcm.len(), SAMPLES * 4);
    }
}
