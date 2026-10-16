use crate::MediaError;
use std::io::Read;

/// Fixed-size decoder buffer; never allocates a complete file's PCM samples.
pub(super) fn decode(
    mut reader: impl Read,
    duration: f64,
    channels: u32,
    bins: usize,
) -> Result<Vec<f64>, MediaError> {
    if channels == 0 || !duration.is_finite() || duration <= 0.0 || bins == 0 {
        return Err(MediaError::Invalid("invalid waveform dimensions".into()));
    }
    let mut peaks = vec![0.0_f64; bins];
    let mut buffer = [0_u8; 16_387];
    let mut carry = 0;
    let mut samples = 0_u64;
    loop {
        let count = reader
            .read(&mut buffer[carry..])
            .map_err(|error| MediaError::Invalid(format!("audio decode: {error}")))?;
        if count == 0 {
            break;
        }
        let length = carry + count;
        let complete = length / 4 * 4;
        for sample in buffer[..complete].as_chunks::<4>().0 {
            let value = f32::from_le_bytes(*sample).abs();
            let frame = samples / u64::from(channels);
            let bucket =
                ((frame as f64 * bins as f64 / (duration * 48000.0)) as usize).min(bins - 1);
            if value.is_finite() {
                peaks[bucket] = peaks[bucket].max(f64::from(value.min(1.0)));
            }
            samples += 1;
        }
        carry = length - complete;
        buffer.copy_within(complete..length, 0);
    }
    if samples == 0 || carry != 0 || !samples.is_multiple_of(u64::from(channels)) {
        return Err(MediaError::Invalid(
            "audio decode produced incomplete PCM".into(),
        ));
    }
    Ok(peaks)
}
