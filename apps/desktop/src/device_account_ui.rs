use super::*;

impl MeApp {
    pub(super) fn log_out(&mut self, cx: &mut Context<Self>) {
        self.sign_out_to(AccountMode::SignIn, cx);
    }

    pub(super) fn sign_out_to(&mut self, mode: AccountMode, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(base) = Self::vault_path() else {
            return;
        };
        let viewport = self.knowledge.loaded.then(|| self.knowledge.view.clone());
        let old = self.detach_vault(cx);
        self.stop_codex_setup();
        self.codex_ready = false;
        self.codex_notice = false;
        self.codex_issue = None;
        self.codex_message.clear();
        self.restore_from = None;
        self.show_onepassword = false;
        self.onepassword_preview = None;
        self.onepassword_result = None;
        let generation = self.generation;
        let task = cx.background_executor().spawn(async move {
            {
                let mut guard = old
                    .lock()
                    .map_err(|_| "Couldn't close the vault. Please try again.")?;
                if let (Some(vault), Some(viewport)) = (guard.as_mut(), viewport) {
                    let _ = vault.save_knowledge_viewport(&viewport);
                }
                guard.take();
            }
            device_accounts::sign_out(&base)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if generation != this.generation {
                    return;
                }
                this.busy = false;
                match result {
                    Ok(root) => {
                        this.root = Some(root);
                        this.initialized = false;
                        this.settings = me_core::AppSettings::default();
                        this.account = AccountState {
                            mode,
                            signed_out: true,
                            ..Default::default()
                        };
                        this.account_email.update(cx, |i, cx| i.set_text("", cx));
                        this.focus_password_on_ready = true;
                    }
                    Err(error) => {
                        if this.account.binding.is_some() {
                            this.account.mode = AccountMode::Unlock;
                        }
                        this.error = Some(format!("Your vault is locked. {error}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}
