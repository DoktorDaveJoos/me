use super::*;

#[derive(Default)]
pub(super) struct PermissionsState {
    pub open: bool,
    busy: bool,
    request: u64,
    status: Option<context_source::Permissions>,
    error: Option<String>,
}
impl MeApp {
    pub(super) fn open_permissions(
        &mut self,
        _: &OpenPermissions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.permissions.open = true;
        window.focus(&self.focus);
        self.refresh_permissions(None, cx);
    }
    pub(super) fn close_permissions(&mut self, cx: &mut Context<Self>) {
        self.permissions.open = false;
        self.permissions.busy = false;
        self.permissions.request += 1;
        cx.notify();
    }
    fn refresh_permissions(
        &mut self,
        request: Option<context_source::PermissionKind>,
        cx: &mut Context<Self>,
    ) {
        if self.permissions.busy {
            return;
        }
        self.permissions.busy = true;
        self.permissions.error = None;
        self.permissions.request += 1;
        let generation = self.permissions.request;
        let task = cx
            .background_executor()
            .spawn(async move { context_source::permissions(request) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if !this.permissions.open || this.permissions.request != generation {
                    return;
                }
                this.permissions.busy = false;
                match result {
                    Ok(status) => this.permissions.status = Some(status),
                    Err(error) => this.permissions.error = Some(error.into()),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(super) fn permissions_modal(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let state = &self.permissions;
        let rows = [
            (
                context_source::PermissionKind::ScreenRecording,
                "Window previews",
                "Screen Recording lets ME show thumbnails of your open windows. Screenshots stay on this Mac.",
                state.status.as_ref().map(|s| s.screen_recording),
                "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture",
            ),
            (
                context_source::PermissionKind::Accessibility,
                "Read and fill selected windows",
                "Accessibility lets ME read the window you choose and fill fields after you review and save a login.",
                state.status.as_ref().map(|s| s.accessibility),
                "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility",
            ),
        ];
        self.overlay(cx).p(px(space::LG)).child(
            modal_panel(640., self.motion_enabled())
                .max_h((window.viewport_size().height - px(space::SECTION)).min(px(700.)))
                .child(div().flex().items_center().justify_between().gap(px(space::LG))
                    .child(heading("Privacy permissions"))
                    .child(secondary_action().id("close-permissions").on_click(cx.listener(|this,_,_,cx|this.close_permissions(cx))).child("Done")))
                .child(div().id("permissions-scroll").min_h_0().overflow_y_scroll().flex().flex_col().gap(px(space::XL))
                    .child(div().type_style(Type::Small).text_color(rgb(MUTED)).child("Grant access once for this installed app. You decide which window ME reads; opening this panel captures nothing."))
                    .children(rows.into_iter().enumerate().map(|(i,(kind,title,description,allowed,url))| {
                        div().p(px(space::LG)).rounded(px(radius::STANDARD)).bg(rgb(BG)).border_1().border_color(rgb(LINE)).flex().flex_col().gap(px(space::MD))
                            .child(div().flex().items_center().justify_between().gap(px(space::MD)).child(div().type_style(Type::Label).child(title))
                                .child(div().type_style(Type::Small).text_color(rgb(if allowed==Some(true){SUCCESS}else{MUTED})).child(if state.busy {"Checking…"} else {match allowed {Some(true)=>"Allowed",Some(false)=>"Not allowed",None=>"Unknown"}})))
                            .child(div().type_style(Type::Small).text_color(rgb(MUTED)).child(description))
                            .child(div().flex().flex_wrap().gap(px(space::SM))
                                .when(allowed!=Some(true),|s|s.child(primary_action().id(("request-permission",i)).on_click(cx.listener(move|this,_,_,cx|this.refresh_permissions(Some(kind),cx))).child("Allow access")))
                                .child(secondary_action().id(("permission-settings",i)).on_click(cx.listener(move|_,_,_,cx|cx.open_url(url))).child("Open Settings")))
                    }))
                    .when_some(state.error.clone(),|s,e|s.child(div().p(px(space::MD)).rounded(px(radius::STANDARD)).bg(rgb(WARNING_SURFACE)).type_style(Type::Small).text_color(rgb(WARNING)).child(e)))
                    .child(div().type_style(Type::Small).text_color(rgb(MUTED)).child("After granting access, choose Check again. If macOS asks you to quit and reopen ME, do so first. Permission entries for older test apps do not apply to ME Dev."))
                    .child(secondary_action().id("refresh-permissions").on_click(cx.listener(|this,_,_,cx|this.refresh_permissions(None,cx))).child("Check again"))
                )
        ).into_any_element()
    }
}
