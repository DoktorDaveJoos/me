//! The narrow surface the menu bar quick panel uses. It reuses the main
//! window's session, unlock, search, intake and note flows instead of copying them.
use super::*;

/// How many search results the quick panel lists.
const QUICK_RESULTS: usize = 5;
/// Longest note title taken from the first line of a quick note.
const QUICK_TITLE_CHARS: usize = 60;

pub(crate) enum QuickAccess {
    /// First run, account setup or restore: only the main window handles these.
    Setup,
    Locked {
        busy: bool,
        error: Option<String>,
    },
    Ready,
}

pub(crate) struct QuickPending {
    pub name: String,
    pub detail: String,
    pub error: Option<String>,
}

impl MeApp {
    pub(crate) fn quick_access(&self) -> QuickAccess {
        if self.app_ready() {
            QuickAccess::Ready
        } else if !self.initialized
            || self.restore_from.is_some()
            || self.account_setup_visible()
            || self.unlocked
        {
            QuickAccess::Setup
        } else {
            QuickAccess::Locked {
                busy: self.busy,
                error: self.error.clone(),
            }
        }
    }

    pub(crate) fn quick_unlock(&mut self, password: &str, cx: &mut Context<Self>) {
        if !matches!(self.quick_access(), QuickAccess::Locked { busy: false, .. }) {
            return;
        }
        self.password
            .update(cx, |input, cx| input.set_text(password, cx));
        self.unlock_vault(cx);
    }

    pub(crate) fn quick_results(&self, query: &str) -> Vec<me_core::DataFact> {
        if !self.app_ready() {
            return Vec::new();
        }
        let mut facts = me_core::filter_data(&self.filter.facts, query, &[]);
        facts.truncate(QUICK_RESULTS);
        facts
    }

    /// Copies with the same owned, self-clearing clipboard as the main window.
    pub(crate) fn quick_copy(&mut self, fact: me_core::DataFact, cx: &mut Context<Self>) {
        self.copy_detail(fact, cx);
    }

    pub(crate) fn quick_copied(&self) -> Option<&str> {
        self.filter.copied.as_deref()
    }

    /// Dropped paths are only inspected; nothing is imported before confirmation.
    pub(crate) fn quick_drop(&mut self, paths: &[PathBuf], cx: &mut Context<Self>) {
        if self.app_ready() {
            self.accept_documents(paths, cx);
        }
    }

    pub(crate) fn quick_pending(&self) -> Vec<QuickPending> {
        self.pending_imports
            .iter()
            .map(|file| QuickPending {
                name: file
                    .path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                detail: file.detail.clone(),
                error: file.error.clone(),
            })
            .collect()
    }

    /// True while dropped files are inspected or imported.
    pub(crate) fn quick_intake_busy(&self) -> bool {
        self.import_scans > 0 || self.intake_active
    }

    pub(crate) fn quick_confirm_drop(&mut self, cx: &mut Context<Self>) {
        self.confirm_imports(cx);
    }

    pub(crate) fn quick_cancel_drop(&mut self, cx: &mut Context<Self>) {
        self.cancel_import_confirmation(cx);
    }

    /// Saves typed text as a note. Returns false when there is nothing to save
    /// or the vault is busy.
    pub(crate) fn quick_note(&mut self, text: &str, cx: &mut Context<Self>) -> bool {
        let Some((title, value)) = quick_note_parts(text) else {
            return false;
        };
        if !self.app_ready() || self.busy {
            return false;
        }
        let value = zeroize::Zeroizing::new(value);
        self.run_change(cx, move |vault| {
            vault.save_note(None, &title, &value)?;
            Ok("Saved.".into())
        });
        true
    }

    pub(crate) fn quick_feedback(&self) -> (Option<String>, Option<String>) {
        (self.notice.clone(), self.error.clone())
    }
}

/// The first line titles the note; the whole text is its value.
fn quick_note_parts(text: &str) -> Option<(String, String)> {
    let value = text.trim();
    let first = value.lines().next()?.trim();
    if first.is_empty() {
        return None;
    }
    let mut title: String = first.chars().take(QUICK_TITLE_CHARS).collect();
    if first.chars().count() > QUICK_TITLE_CHARS {
        title.push('…');
    }
    Some((title, value.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::quick_note_parts;

    #[test]
    fn quick_note_uses_its_first_line_as_title() {
        assert_eq!(
            quick_note_parts("  Shoe size\n42 EU  "),
            Some(("Shoe size".into(), "Shoe size\n42 EU".into()))
        );
        assert_eq!(
            quick_note_parts("Call the tax office"),
            Some(("Call the tax office".into(), "Call the tax office".into()))
        );
        assert_eq!(quick_note_parts(" \n "), None);
        let long = "x".repeat(70);
        let (title, value) = quick_note_parts(&long).unwrap();
        assert_eq!(title.chars().count(), 61);
        assert_eq!(value, long);
    }
}
