//! Keep source identity separate from disposable editing representations.
use std::{collections::HashMap, path::PathBuf};
use todo_application::PreviewLayer;

#[derive(Default)]
pub(super) struct PreviewSources {
    proxies: HashMap<String, PathBuf>,
    original: Vec<PreviewLayer>,
    published: Option<Vec<PreviewLayer>>,
    moving_until: f64,
    playing: bool,
    transport_time: Option<f64>,
}

impl PreviewSources {
    pub fn install(&mut self, path: String, proxy: PathBuf) {
        self.proxies.insert(path, proxy);
    }
    pub fn layers(&mut self, layers: Vec<PreviewLayer>, now: f64) {
        if self
            .original
            .iter()
            .map(|l| (&l.path, l.media_time))
            .ne(layers.iter().map(|l| (&l.path, l.media_time)))
        {
            self.moving_until = now + 0.18;
        }
        self.original = layers;
    }
    pub fn transport(&mut self, time: f64, playing: bool, now: f64) {
        if self.transport_time != Some(time) {
            self.moving_until = now + 0.18;
        }
        self.transport_time = Some(time);
        self.playing = playing;
    }
    pub fn finish_scrub(&mut self) {
        self.moving_until = f64::NEG_INFINITY;
    }
    pub fn take_changed(&mut self, now: f64) -> Option<Vec<PreviewLayer>> {
        let mut mapped = self.original.clone();
        if self.playing || now < self.moving_until {
            for layer in &mut mapped {
                if let Some(proxy) = self.proxies.get(&layer.path) {
                    // The compositor aspect-fits media; preserved display aspect
                    // means proxy resolution must not alter the clip transform.
                    layer.path = proxy.to_string_lossy().into_owned();
                }
            }
        }
        if self.published.as_ref() == Some(&mapped) {
            return None;
        }
        self.published = Some(mapped.clone());
        Some(mapped)
    }
    pub fn top_source(&self) -> Option<&str> {
        self.original.last().map(|l| l.path.as_str())
    }
    pub fn reset_presentation(&mut self) {
        self.original.clear();
        self.published = None;
        self.playing = false;
        self.moving_until = f64::NEG_INFINITY;
        self.transport_time = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layer(path: &str, time: f64) -> PreviewLayer {
        PreviewLayer {
            path: path.into(),
            media_time: time,
            aspect: Some((1920, 1080)),
            transform: Default::default(),
        }
    }

    #[test]
    fn moving_preview_uses_proxy_but_keeps_geometry_time_and_original_identity() {
        let mut sources = PreviewSources::default();
        sources.install("source.mp4".into(), PathBuf::from("cache/proxy.mp4"));
        let mut video = layer("source.mp4", 12.345);
        video.transform.x = 0.2;
        video.transform.scale_x = 1.6;
        let original_transform = video.transform;
        sources.layers(vec![video, layer("text.png", 0.0)], 1.0);
        let mapped = sources.take_changed(1.0).unwrap();
        assert_eq!(mapped[0].path, "cache/proxy.mp4");
        assert_eq!(mapped[0].media_time, 12.345);
        assert_eq!(mapped[0].aspect, Some((1920, 1080)));
        assert_eq!(mapped[0].transform, original_transform);
        assert_eq!(mapped[1].path, "text.png");
        assert_eq!(sources.top_source(), Some("text.png"));
    }

    #[test]
    fn release_and_idle_restore_original_even_without_a_new_picture_command() {
        for release in [false, true] {
            let mut sources = PreviewSources::default();
            sources.install("source.mp4".into(), "proxy.mp4".into());
            sources.layers(vec![layer("source.mp4", 8.1)], 1.0);
            assert_eq!(sources.take_changed(1.0).unwrap()[0].path, "proxy.mp4");
            if release {
                sources.finish_scrub();
            }
            let settled = sources
                .take_changed(if release { 1.01 } else { 1.3 })
                .unwrap();
            assert_eq!(settled[0].path, "source.mp4");
            assert_eq!(settled[0].media_time, 8.1);
            assert!(sources.take_changed(1.4).is_none());
        }
    }

    #[test]
    fn playback_keeps_proxy_while_transforms_and_late_completion_do_not_restart_idle() {
        let mut sources = PreviewSources::default();
        sources.layers(vec![layer("source.mp4", 8.1)], 1.0);
        assert_eq!(sources.take_changed(1.0).unwrap()[0].path, "source.mp4");
        sources.install("source.mp4".into(), "proxy.mp4".into());
        assert!(
            sources.take_changed(2.0).is_none(),
            "late proxy must not replace the settled original"
        );
        let mut changed = layer("source.mp4", 8.1);
        changed.transform.x = 0.1;
        sources.layers(vec![changed], 2.0);
        assert_eq!(sources.take_changed(2.0).unwrap()[0].path, "source.mp4");
        sources.transport(8.1, true, 2.1);
        assert_eq!(sources.take_changed(5.0).unwrap()[0].path, "proxy.mp4");
        sources.transport(8.1, false, 5.1);
        assert_eq!(sources.take_changed(5.1).unwrap()[0].path, "source.mp4");
    }

    #[test]
    fn clearing_or_resetting_cannot_restore_the_previous_stack() {
        let mut sources = PreviewSources::default();
        sources.install("old.mp4".into(), "proxy.mp4".into());
        sources.layers(vec![layer("old.mp4", 1.0)], 0.0);
        sources.take_changed(0.0);
        sources.layers(Vec::new(), 0.1);
        assert!(sources.take_changed(0.1).unwrap().is_empty());
        sources.reset_presentation();
        assert_eq!(sources.top_source(), None);
        assert!(sources.take_changed(1.0).unwrap().is_empty());
    }
}
