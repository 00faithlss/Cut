use std::{
    fs::{File, OpenOptions},
    io::{self, BufWriter, Read, Seek, SeekFrom, Write},
    path::Path,
    sync::Arc,
    time::Duration,
};

use ::mp4::{
    AacConfig, AvcConfig, Bytes, ChannelConfig, Mp4Config, Mp4Sample, Mp4Writer, TrackConfig,
};
use win_native_media::encoder::{EncodedSample, ParameterSets, mf_h264::iter_annex_b_nals};

use crate::{audio::History, replay::Result};

const VIDEO_CLOCK: u32 = 90_000;
const AUDIO_CLOCK: u32 = 48_000;
const AAC_BLOCK: u32 = 1024;
// AAC-LC silencioso a 48 kHz, sin cabecera ADTS.
const SILENCE_MONO: &[u8] = &[0x00, 0x00, 0x20, 0x07];
const SILENCE_STEREO: &[u8] = &[0x21, 0x10, 0x00, 0x00, 0x00, 0x1c];

pub struct Video {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub parameters: ParameterSets,
}

fn ticks(time: Duration, timescale: u32) -> u64 {
    ((time.as_nanos() * u128::from(timescale) + 500_000_000) / 1_000_000_000) as u64
}

pub fn write(
    path: &Path,
    file: File,
    video: &Video,
    frames: &[Arc<EncodedSample>],
    tracks: &[History],
) -> Result<()> {
    let origin = frames
        .first()
        .filter(|frame| frame.is_keyframe)
        .ok_or("Falta un fotograma clave.")?
        .timestamp;
    let end = frames.last().ok_or("Falta video.")?.timestamp
        + Duration::from_nanos(1_000_000_000 / u64::from(video.fps));
    let mut tracks: Vec<_> = tracks
        .iter()
        .filter(|track| {
            track
                .packets
                .iter()
                .any(|packet| (origin..end).contains(&packet.timestamp))
        })
        .collect();
    tracks.sort_by(|a, b| {
        (a.name != "Mezcla")
            .cmp(&(b.name != "Mezcla"))
            .then_with(|| a.name.cmp(&b.name))
    });

    let brands = [*b"isom", *b"iso2", *b"avc1", *b"mp41"].map(::mp4::FourCC::from);
    let mut mux = Mp4Writer::write_start(
        BufWriter::with_capacity(256 * 1024, file),
        &Mp4Config {
            major_brand: brands[0],
            minor_version: 512,
            compatible_brands: brands.to_vec(),
            timescale: 1000,
        },
    )?;
    let mut config = TrackConfig::from(AvcConfig {
        width: video.width.try_into()?,
        height: video.height.try_into()?,
        seq_param_set: video.parameters.sps.clone(),
        pic_param_set: video.parameters.pps.clone(),
    });
    config.timescale = VIDEO_CLOCK;
    mux.add_track(&config)?;
    for track in &tracks {
        let mut config = TrackConfig::from(AacConfig {
            bitrate: if track.channels == 1 { 96_000 } else { 128_000 },
            chan_conf: if track.channels == 1 {
                ChannelConfig::Mono
            } else {
                ChannelConfig::Stereo
            },
            ..Default::default()
        });
        config.timescale = AUDIO_CLOCK;
        mux.add_track(&config)?;
    }

    let mut audio = tracks
        .iter()
        .map(|track| AudioSamples::new(track, origin))
        .collect::<Vec<_>>();
    let blocks = ticks(end - origin, AUDIO_CLOCK).div_ceil(u64::from(AAC_BLOCK));
    let mut block = 0;
    for (index, frame) in frames.iter().enumerate() {
        let relative = frame.timestamp.saturating_sub(origin);
        let start = ticks(relative, VIDEO_CLOCK);
        while block < blocks && block * u64::from(AAC_BLOCK) <= ticks(relative, AUDIO_CLOCK) {
            write_audio_block(&mut mux, &mut audio, block)?;
            block += 1;
        }
        let next = frames.get(index + 1).map_or(end, |frame| frame.timestamp);
        let duration = ticks(next.saturating_sub(origin), VIDEO_CLOCK)
            .saturating_sub(start)
            .max(1);
        let mut avcc = Vec::with_capacity(frame.data.len());
        for nalu in iter_annex_b_nals(&frame.data).filter(|nalu| !nalu.is_empty()) {
            avcc.extend_from_slice(&u32::try_from(nalu.len())?.to_be_bytes());
            avcc.extend_from_slice(nalu);
        }
        if avcc.is_empty() {
            return Err("Paquete H.264 sin datos.".into());
        }
        mux.write_sample(
            1,
            &Mp4Sample {
                start_time: start,
                duration: duration.try_into()?,
                rendering_offset: 0,
                is_sync: frame.is_keyframe,
                bytes: Bytes::from(avcc),
            },
        )?;
    }
    while block < blocks {
        write_audio_block(&mut mux, &mut audio, block)?;
        block += 1;
    }
    mux.write_end()?;
    mux.into_writer().flush()?;
    label_audio_tracks(path, tracks.iter().map(|track| track.name.as_str()))?;
    Ok(())
}

struct AudioSamples<'a> {
    packets: Vec<(u64, &'a [u8])>,
    position: usize,
    silence: &'static [u8],
}

impl<'a> AudioSamples<'a> {
    fn new(track: &'a History, origin: Duration) -> Self {
        let mut packets: Vec<_> = track
            .packets
            .iter()
            .filter(|packet| packet.timestamp >= origin)
            .map(|packet| {
                let position = ticks(packet.timestamp - origin, AUDIO_CLOCK);
                let block = (position + u64::from(AAC_BLOCK) / 2) / u64::from(AAC_BLOCK);
                (block, packet.data.as_slice())
            })
            .collect();
        packets.sort_by_key(|packet| packet.0);
        Self {
            packets,
            position: 0,
            silence: if track.channels == 1 {
                SILENCE_MONO
            } else {
                SILENCE_STEREO
            },
        }
    }

    fn sample(&mut self, index: u64) -> Mp4Sample {
        while self
            .packets
            .get(self.position)
            .is_some_and(|packet| packet.0 < index)
        {
            self.position += 1;
        }
        let bytes = match self.packets.get(self.position) {
            Some(&(block, data)) if block == index => {
                self.position += 1;
                Bytes::copy_from_slice(data)
            }
            _ => Bytes::from_static(self.silence),
        };
        Mp4Sample {
            start_time: index * u64::from(AAC_BLOCK),
            duration: AAC_BLOCK,
            rendering_offset: 0,
            is_sync: true,
            bytes,
        }
    }
}

fn write_audio_block<W: Write + Seek>(
    mux: &mut Mp4Writer<W>,
    tracks: &mut [AudioSamples<'_>],
    block: u64,
) -> Result<()> {
    for (index, track) in tracks.iter_mut().enumerate() {
        mux.write_sample((index + 2).try_into()?, &track.sample(block))?;
    }
    Ok(())
}

fn label_audio_tracks<'a>(path: &Path, mut names: impl Iterator<Item = &'a str>) -> io::Result<()> {
    if names.size_hint().0 == 0 {
        return Ok(());
    }
    let mut file = OpenOptions::new().read(true).write(true).open(path)?;
    let length = file.metadata()?.len();
    let mut offset = 0;
    while offset < length {
        let mut header = [0; 8];
        file.read_exact(&mut header)?;
        let kind: [u8; 4] = header[4..].try_into().unwrap();
        let mut size = u64::from(u32::from_be_bytes(header[..4].try_into().unwrap()));
        let mut header_size = 8;
        if size == 1 {
            file.read_exact(&mut header)?;
            size = u64::from_be_bytes(header);
            header_size = 16;
        }
        if size < header_size || size > length - offset {
            return Err(invalid_atoms());
        }
        if &kind == b"moov" {
            // Media Foundation finaliza con moov después de mdat. Al reescribir
            // ese índice, los offsets de los datos ya guardados siguen vigentes.
            if offset + size != length || size > 32 * 1024 * 1024 {
                return Err(invalid_atoms());
            }
            let mut movie = vec![0; (size - header_size) as usize];
            file.read_exact(&mut movie)?;
            let movie = relabel(&movie, &mut names)?;
            if names.next().is_some() {
                return Err(invalid_atoms());
            }
            file.seek(SeekFrom::Start(offset))?;
            file.write_all(&atom(b"moov", &movie)?)?;
            let end = file.stream_position()?;
            file.set_len(end)?;
            return Ok(());
        }
        offset += size;
        file.seek(SeekFrom::Start(offset))?;
    }
    Err(invalid_atoms())
}

fn relabel<'a>(mut data: &[u8], names: &mut impl Iterator<Item = &'a str>) -> io::Result<Vec<u8>> {
    let mut result = Vec::with_capacity(data.len());
    while !data.is_empty() {
        if data.len() < 8 {
            return Err(invalid_atoms());
        }
        let size = u32::from_be_bytes(data[..4].try_into().unwrap()) as usize;
        if !(8..=data.len()).contains(&size) {
            return Err(invalid_atoms());
        }
        let kind: &[u8; 4] = data[4..8].try_into().unwrap();
        let contents = &data[8..size];
        match kind {
            b"trak" | b"mdia" => result.extend(atom(kind, &relabel(contents, names)?)?),
            b"hdlr" if contents.get(8..12) == Some(b"soun") && contents.len() >= 24 => {
                let mut label = contents[..24].to_vec();
                label.extend_from_slice(names.next().ok_or_else(invalid_atoms)?.as_bytes());
                label.push(0);
                result.extend(atom(kind, &label)?);
            }
            _ => result.extend_from_slice(&data[..size]),
        }
        data = &data[size..];
    }
    Ok(result)
}

fn atom(kind: &[u8; 4], data: &[u8]) -> io::Result<Vec<u8>> {
    let size = u32::try_from(data.len() + 8).map_err(|_| invalid_atoms())?;
    let mut bytes = Vec::with_capacity(size as usize);
    bytes.extend_from_slice(&size.to_be_bytes());
    bytes.extend_from_slice(kind);
    bytes.extend_from_slice(data);
    Ok(bytes)
}

fn invalid_atoms() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "El índice MP4 de Media Foundation tiene una estructura inesperada.",
    )
}
