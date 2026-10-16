use super::pcm::decode;
use std::io::{self, Cursor, Read};

struct Fragments(Cursor<Vec<u8>>);
impl Read for Fragments {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let count = buffer.len().min(3);
        self.0.read(&mut buffer[..count])
    }
}
#[test]
fn stereo_fragments_keep_channel_frames_and_clamp_nonfinite_or_large_samples() {
    let samples = [-0.1_f32, 0.5, 0.9, -0.2, f32::NAN, 0.3, 2.0, -0.8];
    let bytes = samples.into_iter().flat_map(f32::to_le_bytes).collect();
    let peaks = decode(Fragments(Cursor::new(bytes)), 4.0 / 48000.0, 2, 2).unwrap();
    assert!((peaks[0] - 0.9).abs() < 1e-6);
    assert_eq!(peaks[1], 1.0);
}
#[test]
fn incomplete_samples_and_incomplete_channel_frames_fail() {
    assert!(decode(Cursor::new(vec![0; 3]), 1.0, 1, 10).is_err());
    assert!(decode(Cursor::new(vec![0; 4]), 1.0, 2, 10).is_err());
    assert!(decode(Cursor::new(Vec::<u8>::new()), 1.0, 1, 10).is_err());
}
