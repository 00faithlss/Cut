use memchr::memmem;

/// Itera las unidades H.264 directamente sobre el paquete entregado por la GPU.
pub fn units(data: &[u8]) -> impl Iterator<Item = &[u8]> {
    let prefix = b"\0\0\x01";
    let mut remaining = memmem::find(data, prefix).map(|at| &data[at + 3..]);
    std::iter::from_fn(move || {
        let data = remaining.take()?;
        let Some(at) = memmem::find(data, prefix) else {
            return Some(data);
        };
        remaining = Some(&data[at + 3..]);
        let end = if at > 0 && data[at - 1] == 0 {
            at - 1
        } else {
            at
        };
        Some(&data[..end])
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use win_native_media::encoder::mf_h264::iter_annex_b_nals;

    #[test]
    fn conserva_los_limites_de_cada_unidad_del_codificador() {
        let packet = [
            9, 0, 0, 0, 1, 0x67, 0, 0, 3, 1, 0, 0, 1, 0x68, 0, 0, 1, 0, 0, 0, 1, 0x65, 0, 0, 0, 0,
            1,
        ];
        for start in 0..packet.len() {
            for end in start..=packet.len() {
                let data = &packet[start..end];
                assert!(units(data).eq(iter_annex_b_nals(data)));
            }
        }
    }
}
