//! Production panel glue: build its view, start new productions and resume
//! the stored ones when a project opens.
use super::*;
use std::collections::BTreeMap;
use std::path::Path;
use todo_interaction::{production_view_with_document, ParamValue, ProductionPanelView};
use todo_production::extension::protocol::Canvas;

impl App {
    fn canvas(&self) -> Option<(String, Canvas)> {
        let editor = self.workspace.editor()?;
        let project = editor.project();
        let (width, height) = project.output_size();
        Some((
            editor.project_name().to_owned(),
            Canvas {
                width,
                height,
                fps: project.fps,
            },
        ))
    }

    pub(super) fn production_panel(&self) -> ProductionPanelView {
        let Some(host) = &self.production else {
            return ProductionPanelView {
                project_open: self.workspace.editor().is_some(),
                service_problem: Some(match &self.production_error {
                    Some(error) => format!("AI 服务启动失败：{error}"),
                    None => "AI 服务没有启动".into(),
                }),
                ..ProductionPanelView::default()
            };
        };
        let project = self
            .workspace
            .editor()
            .map(|editor| editor.project_name().to_owned());
        ProductionPanelView {
            extensions: host.cards(),
            workflows: host.workflow_entries(),
            productions: self
                .workspace
                .production_status()
                .iter()
                .map(|status| {
                    let mut view = production_view_with_document(
                        status,
                        host.extension_name(&status.extension).as_deref(),
                        self.workspace.production(&status.id),
                        host.workflow_panel(&status.extension),
                    );
                    // The document stores files relative to its production
                    // workdir. The UI needs an absolute key for the media
                    // worker and thumbnail cache, while edits still carry
                    // only element/variant ids.
                    if let Some(project) = project.as_deref() {
                        if let Ok(workdir) = host.workdir_path(project, &status.id) {
                            for segment in &mut view.segments {
                                for element in &mut segment.elements {
                                    for variant in &mut element.variants {
                                        if let Some(file) = variant.file.as_mut() {
                                            if !Path::new(file).is_absolute() {
                                                *file =
                                                    workdir.join(&*file).to_string_lossy().into();
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    view
                })
                .collect(),
            project_open: self.workspace.editor().is_some(),
            service_problem: None,
        }
    }

    pub(super) fn start_production(
        &mut self,
        extension: &str,
        params: &BTreeMap<String, ParamValue>,
        script: &str,
        ctx: &egui::Context,
    ) {
        let Some((project, canvas)) = self.canvas() else {
            self.notice("请先打开一个工程".into());
            return;
        };
        let Some(host) = &self.production else {
            self.notice("AI 成片服务没有启动".into());
            return;
        };
        match host.create(&project, extension, params, script, canvas) {
            Ok((doc, setup)) => {
                let update = self.workspace.add_production(doc, setup);
                self.handle(update, ctx);
            }
            Err(message) => self.notice(format!("无法开始：{message}")),
        }
    }

    /// Resume every stored production of the project that just opened.
    pub(super) fn resume_productions(&mut self) -> Vec<todo_application::Effect> {
        let Some((project, canvas)) = self.canvas() else {
            return Vec::new();
        };
        let Some(host) = &self.production else {
            return Vec::new();
        };
        let (found, problems) = host.open_all(&project, &canvas);
        for problem in problems {
            self.notice(problem);
        }
        let mut effects = Vec::new();
        for (doc, setup) in found {
            effects.extend(self.workspace.add_production(doc, setup).effects);
        }
        effects
    }
}
