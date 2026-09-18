use super::*;

impl MeApp {
    pub(super) fn paste_files(
        &mut self,
        _: &crate::input::Paste,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.app_ready() {
            return;
        }
        let generation = self.generation;
        let text = cx.read_from_clipboard().and_then(|item| item.text());
        let paste_into_chat = self.filter_input.focus_handle(cx).is_focused(window);
        let task = cx
            .background_executor()
            .spawn(async move { clipboard_files() });
        cx.spawn_in(window, async move |this, cx| {
            let paths = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.generation != generation || !this.app_ready() {
                    return;
                }
                if !paths.is_empty() {
                    this.pending_attach_to_search |= paste_into_chat && this.page == Page::Search;
                    this.accept_documents(&paths, cx);
                } else if paste_into_chat && let Some(text) = text {
                    this.filter_input
                        .update(cx, |input, cx| input.paste_text(&text, window, cx));
                }
            });
        })
        .detach();
    }
}

fn clipboard_files() -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let script = r#"ObjC.import('AppKit'); const pasteboard=$.NSPasteboard.generalPasteboard; const legacy=ObjC.deepUnwrap(pasteboard.propertyListForType('NSFilenamesPboardType')); const paths=Array.isArray(legacy) ? legacy : []; if(paths.length===0) { const items=pasteboard.pasteboardItems; for(let i=0; i<items.count; i++) { const raw=ObjC.unwrap(items.objectAtIndex(i).stringForType('public.file-url')); if(typeof raw==='string') { const url=$.NSURL.URLWithString(raw); if(url.isFileURL) paths.push(ObjC.unwrap(url.path)); } } } JSON.stringify(paths);"#;
        std::process::Command::new("/usr/bin/osascript")
            .args(["-l", "JavaScript", "-e", script])
            .output()
            .ok()
            .and_then(|o| serde_json::from_slice::<Vec<String>>(&o.stdout).ok())
            .unwrap_or_default()
            .into_iter()
            .map(PathBuf::from)
            .collect()
    }
    #[cfg(target_os = "linux")]
    {
        for (command, args) in [
            ("wl-paste", vec!["--type", "text/uri-list"]),
            (
                "xclip",
                vec!["-selection", "clipboard", "-o", "-t", "text/uri-list"],
            ),
        ] {
            if let Ok(output) = std::process::Command::new(command).args(args).output()
                && output.status.success()
            {
                return String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .filter_map(|line| {
                        url::Url::parse(line.trim())
                            .ok()
                            .and_then(|u| u.to_file_path().ok())
                    })
                    .collect();
            }
        }
        Vec::new()
    }
}
