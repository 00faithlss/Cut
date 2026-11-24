use std::{
    collections::VecDeque,
    fs::{File, OpenOptions},
    io,
    os::windows::fs::{FileExt, OpenOptionsExt},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use win_native_media::encoder::{EncodedSample, ParameterSets};
use windows::Win32::Storage::FileSystem::FILE_FLAG_DELETE_ON_CLOSE;

const CHUNK_BYTES: u64 = 4 * 1024 * 1024;
static CHUNK_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Clone)]
pub(crate) struct Frame {
    file: Arc<File>,
    offset: u64,
    length: usize,
    pub timestamp: Duration,
    pub is_keyframe: bool,
}

impl Frame {
    pub fn read_into(&self, data: &mut Vec<u8>) -> io::Result<()> {
        data.resize(self.length, 0);
        let mut read = 0;
        while read < data.len() {
            let count = self
                .file
                .seek_read(&mut data[read..], self.offset + read as u64)?;
            if count == 0 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            read += count;
        }
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct Writer {
    chunk: Option<Arc<File>>,
    offset: u64,
}

impl Writer {
    pub fn append(&mut self, packet: EncodedSample) -> io::Result<Frame> {
        if self.chunk.is_none() || self.offset >= CHUNK_BYTES {
            self.chunk = Some(Arc::new(open_chunk()?));
            self.offset = 0;
        }
        let file = self.chunk.as_ref().unwrap();
        let mut written = 0;
        while written < packet.data.len() {
            let count = file.seek_write(&packet.data[written..], self.offset + written as u64)?;
            if count == 0 {
                return Err(io::ErrorKind::WriteZero.into());
            }
            written += count;
        }
        let frame = Frame {
            file: Arc::clone(file),
            offset: self.offset,
            length: written,
            timestamp: packet.timestamp,
            is_keyframe: packet.is_keyframe,
        };
        self.offset += written as u64;
        Ok(frame)
    }
}

fn open_chunk() -> io::Result<File> {
    loop {
        let id = CHUNK_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("cut-replay-{}-{id}.tmp", std::process::id()));
        match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .custom_flags(FILE_FLAG_DELETE_ON_CLOSE.0)
            .open(path)
        {
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            result => return result,
        }
    }
}

#[derive(Default)]
pub(super) struct Buffer {
    // Each group starts with an independently decodable frame.
    groups: VecDeque<Vec<Frame>>,
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
            group.push(frame);
        }
        while self
            .groups
            .get(1)
            .is_some_and(|group| group[0].timestamp <= cutoff)
        {
            self.groups.pop_front();
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_preserves_evicted_chunks_and_an_appending_tail() -> io::Result<()> {
        let mut writer = Writer::default();
        let mut history = Buffer::default();
        let mut saved = Vec::new();
        let retain = Duration::from_secs(2);
        for index in 0..8u8 {
            let frame = writer.append(EncodedSample {
                data: vec![index; CHUNK_BYTES as usize / 2],
                timestamp: Duration::from_secs(index.into()),
                is_keyframe: index % 2 == 0,
            })?;
            history.push(frame, retain);
            if index == 4 {
                saved = history.frames().cloned().collect();
            }
        }
        assert_eq!(
            history.timestamps().unwrap(),
            (Duration::from_secs(4), Duration::from_secs(7))
        );
        drop((history, writer));
        let mut bytes = Vec::new();
        for (index, frame) in saved.iter().enumerate() {
            frame.read_into(&mut bytes)?;
            assert_eq!(bytes.len(), CHUNK_BYTES as usize / 2);
            assert!(bytes.iter().all(|byte| *byte == index as u8 + 2));
        }
        assert_eq!(saved.len(), 3);
        Ok(())
    }
}
