//! Connect project lifetime and proxy results without file I/O on the UI thread.
use super::*;
use todo_interaction::MediaKind;

impl App {
    pub(super) fn sync_edit_proxies(&mut self, ctx: &egui::Context) {
        let identity = self
            .workspace
            .editor()
            .map(|editor| (self.workspace.generation(), editor.revision()));
        if self.proxy_project != identity {
            let paths = self
                .workspace
                .editor()
                .map(|editor| {
                    editor
                        .project()
                        .media
                        .iter()
                        .filter(|media| matches!(media.kind, None | Some(MediaKind::Video)))
                        .filter_map(|media| media.path.clone())
                        .collect()
                })
                .unwrap_or_default();
            self.proxies.set_sources(self.workspace.generation(), paths);
            self.proxy_project = identity;
        }
        for (source, proxy) in self.proxies.poll() {
            self.preview.install_edit_proxy(source, proxy);
            ctx.request_repaint();
        }
        let priority = self.workspace.editor().and_then(|editor| {
            editor
                .shown_layers()
                .iter()
                .find(|layer| !layer.path.ends_with(".png"))
                .map(|layer| layer.path.as_str())
        });
        self.proxies.pump(priority);
        if let Err(error) = self.preview.refresh_edit_sources() {
            self.notice(format!("预览：{error}"));
        }
    }
}
