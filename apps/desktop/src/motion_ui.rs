use super::*;
use std::time::Instant;

#[derive(Default)]
pub(super) struct MotionPreferences {
    pub loaded: bool,
    pub window_started: Option<Instant>,
    pub reduced: bool,
    pub website_icons: bool,
    pub saving: bool,
    pub error: Option<String>,
    pub path: Option<PathBuf>,
}
impl MeApp {
    pub(super) fn motion_enabled(&self) -> bool {
        self.motion.loaded && !self.motion.reduced
    }
    pub(super) fn window_entrance_phase(&self, window: &mut Window, milliseconds: u64) -> f32 {
        let phase = motion::phase(
            self.motion.window_started,
            milliseconds,
            self.motion_enabled(),
        );
        if phase < 1. {
            window.request_animation_frame();
        }
        phase
    }
    pub(super) fn workspace_activity(&self) -> bool {
        self.busy
            || self.logins.loading
            || self.logins.detail_loading
            || self.settings_saving
            || self.filter.loading
            || self.filter.local_loading
            || self.filter.form_loading
            || self.filter.handoff_busy
            || self.knowledge.loading
            || self.import_refresh
            || self.import_scans > 0
            || self.intake_active
            || !self.active_imports.is_empty()
            || self
                .organize_cancel
                .as_ref()
                .is_some_and(|cancel| !cancel.load(std::sync::atomic::Ordering::SeqCst))
    }
    pub(super) fn load_motion_preferences(cx: &mut Context<Self>) {
        let path =
            Self::vault_path().and_then(|path| path.parent().map(|p| p.join("interface.json")));
        let task = cx.background_executor().spawn(async move {
            let result = path
                .as_deref()
                .map(interface_preferences::load)
                .unwrap_or(Ok(interface_preferences::Preferences::default()));
            (path, result)
        });
        cx.spawn(async move|this,cx|{let(path,result)=task.await;let _=this.update(cx,|this,cx|{
            this.motion.path=path;this.motion.loaded=true;this.motion.window_started.get_or_insert_with(Instant::now);
            match result {Ok(p)=>{this.motion.reduced=p.reduced;this.motion.website_icons=p.website_icons;},Err(_)=>{this.motion.reduced=true;this.motion.error=Some("Couldn't read the motion preference. Choose a setting to save it again.".into());}}
            cx.notify();
        });}).detach();
    }
    pub(super) fn toggle_motion(&mut self, cx: &mut Context<Self>) {
        if !self.motion.loaded || self.motion.saving {
            return;
        }
        let Some(path) = self.motion.path.clone() else {
            self.motion.error = Some("Couldn't locate the interface settings.".into());
            cx.notify();
            return;
        };
        let previous = interface_preferences::Preferences {
            reduced: self.motion.reduced,
            website_icons: self.motion.website_icons,
        };
        self.motion.reduced = !self.motion.reduced;
        self.save_interface_preferences(path, previous, cx);
    }
    pub(super) fn toggle_website_icons(&mut self, cx: &mut Context<Self>) {
        if !self.motion.loaded || self.motion.saving {
            return;
        }
        let Some(path) = self.motion.path.clone() else {
            return;
        };
        let previous = interface_preferences::Preferences {
            reduced: self.motion.reduced,
            website_icons: self.motion.website_icons,
        };
        self.motion.website_icons = !self.motion.website_icons;
        self.logins.icons.clear();
        self.save_interface_preferences(path, previous, cx);
    }
    fn save_interface_preferences(
        &mut self,
        path: PathBuf,
        previous: interface_preferences::Preferences,
        cx: &mut Context<Self>,
    ) {
        let preferences = interface_preferences::Preferences {
            reduced: self.motion.reduced,
            website_icons: self.motion.website_icons,
        };
        self.motion.saving = true;
        self.motion.error = None;
        let task = cx
            .background_executor()
            .spawn(async move { interface_preferences::save(&path, preferences) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.motion.saving = false;
                if result.is_err() {
                    this.motion.reduced = previous.reduced;
                    this.motion.website_icons = previous.website_icons;
                    this.logins.icons.clear();
                    this.motion.error =
                        Some("Couldn't save the interface preference. Please try again.".into());
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}
