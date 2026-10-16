use super::*;

#[test]
fn attached_cover_does_not_turn_an_audio_file_into_video() {
    let json = serde_json::from_str(r#"{"format":{"duration":"2","format_name":"mp3"},"streams":[{"codec_type":"video","width":96,"height":64,"disposition":{"attached_pic":1}},{"codec_type":"audio","channels":2}]}"#).unwrap();
    let metadata = probe::parse(Path::new("album.mp3"), json).unwrap();
    assert_eq!(metadata.kind, MediaKind::Audio);
    assert_eq!(metadata.width, None);
    assert_eq!(metadata.audio_channels, 2);
}
#[test]
fn stream_duration_fallback_is_finite_and_missing_streams_fail() {
    let json = serde_json::from_str(
        r#"{"streams":[{"codec_type":"audio","duration":"3.5","channels":1}]}"#,
    )
    .unwrap();
    assert_eq!(
        probe::parse(Path::new("audio.wav"), json).unwrap().duration,
        3.5
    );
    for text in [
        r#"{}"#,
        r#"{"streams":[{"codec_type":"audio","duration":"NaN"}]}"#,
    ] {
        assert!(probe::parse(Path::new("audio.wav"), serde_json::from_str(text).unwrap()).is_err());
    }
}
