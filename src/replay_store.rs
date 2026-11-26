use std::{collections::VecDeque, io, sync::Arc, time::Duration};

use win_native_media::encoder::{EncodedSample, ParameterSets};

#[derive(Clone)]
pub(crate) struct Frame {
    // Arc<Vec> adopta la asignación del encoder sin copiar el payload H.264.
    // Los snapshots comparten estos bytes aunque el historial expulse el GOP.
    data: Arc<Vec<u8>>,
    pub timestamp: Duration,
    pub is_keyframe: bool,
}

impl Frame {
    pub fn bytes(&self) -> &[u8] {
        self.data.as_slice()
    }

    #[cfg(test)]
    pub fn read_into(&self, data: &mut Vec<u8>) -> io::Result<()> {
        data.clear();
        data.extend_from_slice(&self.data);
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct Writer {}

impl Writer {
    pub fn append(&mut self, packet: EncodedSample) -> io::Result<Frame> {
        Ok(Frame {
            data: Arc::new(packet.data),
            timestamp: packet.timestamp,
            is_keyframe: packet.is_keyframe,
        })
    }
}

#[derive(Default)]
pub(super) struct Buffer {
    // Cada GOP empieza con un fotograma que puede decodificarse por sí solo.
    groups: VecDeque<Vec<Frame>>,
    bytes: usize,
    pub parameters: ParameterSets,
    pub error: Option<String>,
}

impl Buffer {
    pub fn push(&mut self, frame: Frame, retain: Duration) {
        let cutoff = frame.timestamp.saturating_sub(retain);
        if frame.is_keyframe {
            self.groups.push_back(Vec::new());
        }
        if let Some(group) = self.groups.back_mut() {
            self.bytes += frame.data.len();
            group.push(frame);
        }
        while self
            .groups
            .get(1)
            .is_some_and(|group| group[0].timestamp <= cutoff)
        {
            // Conserva como máximo el GOP que contiene el inicio de la ventana.
            let expired = self.groups.pop_front().unwrap();
            self.bytes -= expired.iter().map(|frame| frame.data.len()).sum::<usize>();
        }
    }

    pub fn timestamps(&self) -> Option<(Duration, Duration)> {
        Some((
            self.groups.front()?.first()?.timestamp,
            self.groups.back()?.last()?.timestamp,
        ))
    }

    pub fn frames(&self) -> impl Iterator<Item = &Frame> {
        self.groups.iter().flatten()
    }

    /// Bytes H.264 del historial activo; los snapshots conservan sus propias referencias.
    pub fn bytes(&self) -> usize {
        self.bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guarda_y_lee_sin_directorio_temporal_disponible() -> io::Result<()> {
        const CHILD: &str = "CUT_REPLAY_STORE_SIN_TEMP";
        if std::env::var_os(CHILD).is_some() {
            let mut writer = Writer::default();
            let frame = writer.append(EncodedSample {
                data: vec![0, 0, 0, 1, 0x65, 0xab],
                timestamp: Duration::from_secs(17),
                is_keyframe: true,
            })?;
            let mut bytes = vec![0xff; 20];
            frame.read_into(&mut bytes)?;
            assert_eq!(bytes, [0, 0, 0, 1, 0x65, 0xab]);
            assert_eq!(frame.timestamp, Duration::from_secs(17));
            assert!(frame.is_keyframe);
            return Ok(());
        }

        // El entorno cambia solo en el subproceso, sin interferir con otras pruebas.
        let missing = std::env::current_exe()?
            .with_file_name(format!("cut-sin-temporales-{}", std::process::id()));
        assert!(!missing.exists());
        let output = std::process::Command::new(std::env::current_exe()?)
            .arg("guarda_y_lee_sin_directorio_temporal_disponible")
            .arg("--nocapture")
            .env(CHILD, "1")
            .env("TMP", &missing)
            .env("TEMP", &missing)
            .output()?;
        assert!(
            output.status.success(),
            "El replay debe funcionar sin temporales:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        assert!(!missing.exists());
        Ok(())
    }

    #[test]
    fn veinte_minutos_retienen_solo_los_ultimos_120_o_600_segundos() -> io::Result<()> {
        for (seconds, expected_first, expected_count) in [(120, 1080, 3631), (600, 600, 18031)] {
            let mut writer = Writer::default();
            let mut history = Buffer::default();
            let retain = Duration::from_secs(seconds);
            // Veinte minutos y un segundo, a 30 FPS y con un GOP cada dos segundos.
            for index in 0..=36_030_u64 {
                let timestamp = Duration::from_nanos(index * 1_000_000_000 / 30);
                history.push(
                    writer.append(EncodedSample {
                        data: vec![index as u8; 32],
                        timestamp,
                        is_keyframe: index % 60 == 0,
                    })?,
                    retain,
                );
                if index > seconds * 30 && index % 300 == 0 {
                    let (first, latest) = history.timestamps().unwrap();
                    assert!(latest - first >= retain);
                    assert!(latest - first < retain + Duration::from_secs(2));
                    assert!(history.frames().next().unwrap().is_keyframe);
                }
            }
            assert_eq!(
                history.timestamps(),
                Some((
                    Duration::from_secs(expected_first),
                    Duration::from_secs(1201)
                ))
            );
            assert_eq!(history.frames().count(), expected_count);
            assert_eq!(history.bytes(), expected_count * 32);
        }
        Ok(())
    }

    #[test]
    fn snapshot_lee_bytes_originales_mientras_el_buffer_expulsa_y_recibe_frames() -> io::Result<()>
    {
        let mut writer = Writer::default();
        let mut history = Buffer::default();
        let mut saved = Vec::new();
        let retain = Duration::from_secs(2);
        for index in 0..=4u8 {
            let frame = writer.append(EncodedSample {
                data: vec![index; 4096],
                timestamp: Duration::from_secs(index.into()),
                is_keyframe: index % 2 == 0,
            })?;
            history.push(frame, retain);
            if index == 4 {
                saved = history.frames().cloned().collect();
            }
        }
        assert_eq!(saved.len(), 3);
        let barrier = Arc::new(std::sync::Barrier::new(2));
        std::thread::scope(|scope| -> io::Result<()> {
            let exporter = scope.spawn({
                let barrier = Arc::clone(&barrier);
                move || -> io::Result<()> {
                    barrier.wait();
                    let mut bytes = Vec::new();
                    for _ in 0..32 {
                        for (index, frame) in saved.iter().enumerate() {
                            frame.read_into(&mut bytes)?;
                            assert_eq!(bytes.len(), 4096);
                            assert!(bytes.iter().all(|byte| *byte == index as u8 + 2));
                        }
                    }
                    Ok(())
                }
            });
            barrier.wait();
            for index in 5..=41u8 {
                history.push(
                    writer.append(EncodedSample {
                        data: vec![index; 4096],
                        timestamp: Duration::from_secs(index.into()),
                        is_keyframe: index % 2 == 0,
                    })?,
                    retain,
                );
            }
            assert_eq!(
                history.timestamps().unwrap(),
                (Duration::from_secs(38), Duration::from_secs(41))
            );
            drop((history, writer));
            exporter.join().unwrap()
        })?;
        Ok(())
    }

    #[test]
    fn adopta_el_vec_del_encoder_y_snapshot_comparte_sus_bytes() -> io::Result<()> {
        let mut data = Vec::with_capacity(4096);
        data.extend_from_slice(&[0, 0, 0, 1, 0x65, 0xab]);
        let original = data.as_ptr();
        let mut writer = Writer::default();
        let frame = writer.append(EncodedSample {
            data,
            timestamp: Duration::from_millis(1234),
            is_keyframe: true,
        })?;
        assert_eq!(frame.data.as_ptr(), original);
        let snapshot = frame.clone();
        assert!(Arc::ptr_eq(&frame.data, &snapshot.data));
        let mut bytes = vec![0xff; 4096];
        snapshot.read_into(&mut bytes)?;
        assert_eq!(bytes, [0, 0, 0, 1, 0x65, 0xab]);
        assert_eq!(snapshot.timestamp, Duration::from_millis(1234));
        assert!(snapshot.is_keyframe);
        Ok(())
    }

    #[test]
    fn expulsion_libera_el_payload_al_salir_el_ultimo_snapshot() -> io::Result<()> {
        let mut writer = Writer::default();
        let mut history = Buffer::default();
        let frame = writer.append(EncodedSample {
            data: vec![0xab; 4096],
            timestamp: Duration::ZERO,
            is_keyframe: true,
        })?;
        let payload = Arc::downgrade(&frame.data);
        history.push(frame, Duration::from_secs(2));
        let snapshot: Vec<_> = history.frames().cloned().collect();
        for second in [2, 4] {
            history.push(
                writer.append(EncodedSample {
                    data: vec![second as u8; 32],
                    timestamp: Duration::from_secs(second),
                    is_keyframe: true,
                })?,
                Duration::from_secs(2),
            );
        }
        assert_eq!(history.frames().count(), 2);
        assert_eq!(history.bytes(), 64);
        assert_eq!(payload.strong_count(), 1);
        drop((history, writer));
        assert_eq!(payload.strong_count(), 1);
        let mut bytes = Vec::new();
        snapshot[0].read_into(&mut bytes)?;
        assert_eq!(bytes, vec![0xab; 4096]);
        drop(snapshot);
        assert!(payload.upgrade().is_none());
        Ok(())
    }

    #[test]
    fn bytes_cuenta_solo_payload_activo_y_descarta_frames_sin_keyframe() -> io::Result<()> {
        let mut writer = Writer::default();
        let mut history = Buffer::default();
        let retain = Duration::from_secs(2);
        assert_eq!(history.bytes(), 0);
        assert_eq!(history.timestamps(), None);
        for (second, keyframe, length, expected_bytes) in [
            (0, false, 2000, 0),
            (1, true, 3, 3),
            (2, false, 7, 10),
            (3, true, 11, 21),
            (5, true, 13, 24),
        ] {
            history.push(
                writer.append(EncodedSample {
                    data: vec![second as u8; length],
                    timestamp: Duration::from_secs(second),
                    is_keyframe: keyframe,
                })?,
                retain,
            );
            assert_eq!(history.bytes(), expected_bytes);
        }
        assert_eq!(
            history.timestamps(),
            Some((Duration::from_secs(3), Duration::from_secs(5)))
        );
        Ok(())
    }
}
