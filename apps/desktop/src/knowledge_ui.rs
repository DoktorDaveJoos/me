use super::knowledge_canvas as geometry;
use super::*;
use crate::design_system::knowledge as map_style;
use gpui::{Bounds, MouseButton, Pixels, Point, SharedString, canvas, point, relative};
use me_core::{KnowledgeKind, KnowledgeMap, KnowledgeStatus, KnowledgeViewport};
use std::{cell::Cell, rc::Rc, time::Duration};

#[derive(Default)]
pub(super) struct KnowledgeState {
    pub graph: Arc<KnowledgeMap>,
    pub view: KnowledgeViewport,
    pub loaded: bool,
    pub loading: bool,
    pub pending: bool,
    pub error: Option<String>,
    pub save_error: Option<String>,
    pub save_revision: u64,
    pub query: String,
    pub matching: Arc<BTreeSet<String>>,
    pub matches: Vec<String>,
    pub match_index: usize,
    pub index: BTreeMap<(i32, i32), usize>,
    pub routes: Arc<Vec<(usize, Vec<Point<f32>>)>>,
    pub bounds: Rc<Cell<Bounds<Pixels>>>,
    pub drag: Option<Point<Pixels>>,
    pub expanded: bool,
    pub copied: Option<String>,
}
impl KnowledgeState {
    pub fn install(&mut self, graph: KnowledgeMap) {
        if !self.loaded {
            self.view = graph.viewport.clone();
        }
        if self
            .view
            .selected_node
            .as_deref()
            .is_none_or(|id| graph.node(id).is_none())
        {
            self.view.selected_node = graph
                .viewport
                .selected_node
                .clone()
                .filter(|id| graph.node(id).is_some())
                .or_else(|| graph.nodes.first().map(|n| n.id.clone()));
        }
        if !self.loaded
            && graph.viewport == KnowledgeViewport::default()
            && let Some(n) = self
                .view
                .selected_node
                .as_deref()
                .and_then(|id| graph.node(id))
        {
            self.view.center_q = f64::from(n.position.q);
            self.view.center_r = f64::from(n.position.r);
        }
        self.index = graph
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| ((n.position.r, n.position.q), i))
            .collect();
        self.graph = Arc::new(graph);
        self.loaded = true;
        self.loading = false;
        self.error = None;
        self.rebuild_matches();
        self.routes = Arc::default();
    }
    fn rebuild_matches(&mut self) {
        let words = self
            .query
            .split_whitespace()
            .map(str::to_lowercase)
            .collect::<Vec<_>>();
        self.matches = self
            .graph
            .nodes
            .iter()
            .filter(|n| {
                let haystack = format!("{} {} {} {}", n.label, n.value, n.context, n.group_label)
                    .to_lowercase();
                words.iter().all(|w| haystack.contains(w))
            })
            .map(|n| n.id.clone())
            .collect();
        self.matching = Arc::new(self.matches.iter().cloned().collect());
        self.match_index = 0;
    }
    fn visible(&self, window: &Window) -> Vec<usize> {
        let size = self.bounds.get().size;
        let w = if size.width > px(0.) {
            f32::from(size.width)
        } else {
            f32::from(window.viewport_size().width)
        };
        let h = if size.height > px(0.) {
            f32::from(size.height)
        } else {
            f32::from(window.viewport_size().height)
        };
        let center = geometry::world_fraction(self.view.center_q, self.view.center_r);
        let z = self.view.zoom as f32;
        let radius = map_style::CELL_RADIUS;
        let width = 3f32.sqrt() * radius;
        let rmin = ((center.y - h / (2. * z) - radius) / (1.5 * radius)).floor() as i32;
        let rmax = ((center.y + h / (2. * z) + radius) / (1.5 * radius)).ceil() as i32;
        let mut visible = Vec::new();
        for r in rmin..=rmax {
            let qmin = ((center.x - w / (2. * z) - width) / width - r as f32 / 2.).floor() as i32;
            let qmax = ((center.x + w / (2. * z) + width) / width - r as f32 / 2.).ceil() as i32;
            visible.extend(self.index.range((r, qmin)..=(r, qmax)).map(|(_, i)| *i));
        }
        visible
    }
}
impl MeApp {
    pub(super) fn refresh_knowledge_routes(&mut self, cx: &mut Context<Self>) {
        self.knowledge.routes = Arc::default();
        let graph = self.knowledge.graph.clone();
        let selected = self.knowledge.view.selected_node.clone();
        let source = graph.clone();
        let selection = selected.clone();
        let task = cx
            .background_executor()
            .spawn(async move { geometry::routes(&source, selection.as_deref()) });
        cx.spawn(async move |this, cx| {
            let paths = task.await;
            let _ = this.update(cx, |this, cx| {
                if Arc::ptr_eq(&graph, &this.knowledge.graph)
                    && this.knowledge.view.selected_node == selected
                {
                    this.knowledge.routes = Arc::new(paths);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(super) fn refresh_knowledge(&mut self, cx: &mut Context<Self>) {
        if !self.app_ready() || self.busy {
            return;
        }
        if self.knowledge.loading {
            self.knowledge.pending = true;
            return;
        }
        self.knowledge.loading = true;
        self.knowledge.error = None;
        let session = self.session.clone();
        let generation = self.generation;
        let task = cx.background_executor().spawn(async move {
            session
                .lock()
                .map_err(|_| me_core::Error::Format)?
                .as_mut()
                .ok_or(me_core::Error::Format)?
                .knowledge_map()
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.knowledge.loading = false;
                match result {
                    Ok(graph) => {
                        this.knowledge.install(graph);
                        this.refresh_knowledge_routes(cx);
                    }
                    Err(e) => this.knowledge.error = Some(e.to_string()),
                }
                if std::mem::take(&mut this.knowledge.pending) {
                    this.refresh_knowledge(cx);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    pub(super) fn knowledge_query_changed(&mut self, cx: &mut Context<Self>) {
        self.knowledge.query = self.knowledge_input.read(cx).content.trim().to_owned();
        self.knowledge.rebuild_matches();
        if !self.knowledge.query.is_empty()
            && let Some(id) = self.knowledge.matches.first().cloned()
        {
            self.select_knowledge(id, true, cx);
        }
        cx.notify();
    }
    fn select_knowledge(&mut self, id: String, center: bool, cx: &mut Context<Self>) {
        let Some(node) = self.knowledge.graph.node(&id) else {
            return;
        };
        if center {
            self.knowledge.view.center_q = f64::from(node.position.q);
            self.knowledge.view.center_r = f64::from(node.position.r);
        }
        self.knowledge.view.selected_node = Some(id);
        self.knowledge.copied = None;
        self.knowledge.expanded = false;
        self.refresh_knowledge_routes(cx);
        self.save_knowledge_view(cx);
        cx.notify();
    }
    fn save_knowledge_view(&mut self, cx: &mut Context<Self>) {
        if !self.knowledge.loaded || !self.app_ready() {
            return;
        }
        self.knowledge.save_revision += 1;
        let revision = self.knowledge.save_revision;
        let generation = self.generation;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(250))
                .await;
            let request = this.update(cx, |this, _| {
                (this.generation == generation && this.knowledge.save_revision == revision)
                    .then(|| (this.session.clone(), this.knowledge.view.clone()))
            });
            let Ok(Some((session, view))) = request else {
                return;
            };
            let result = cx
                .background_executor()
                .spawn(async move {
                    session
                        .lock()
                        .map_err(|_| me_core::Error::Format)?
                        .as_mut()
                        .ok_or(me_core::Error::Format)?
                        .save_knowledge_viewport(&view)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.generation == generation && this.knowledge.save_revision == revision {
                    let error = result.err().map(|e| e.to_string());
                    if this.knowledge.save_error != error {
                        this.knowledge.save_error = error;
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }
    fn pan_knowledge(&mut self, delta: Point<Pixels>, cx: &mut Context<Self>) {
        let (q, r) = geometry::axial(
            f32::from(delta.x) / self.knowledge.view.zoom as f32,
            f32::from(delta.y) / self.knowledge.view.zoom as f32,
        );
        self.knowledge.view.center_q =
            (self.knowledge.view.center_q - q).clamp(-999_000., 999_000.);
        self.knowledge.view.center_r =
            (self.knowledge.view.center_r - r).clamp(-999_000., 999_000.);
        self.save_knowledge_view(cx);
        cx.notify();
    }
    fn zoom_knowledge(&mut self, delta: f64, cx: &mut Context<Self>) {
        self.knowledge.view.zoom = (self.knowledge.view.zoom + delta).clamp(0.4, 2.);
        self.save_knowledge_view(cx);
        cx.notify();
    }
    fn open_knowledge_item(&mut self, item: u64, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(
            self.collection.get(item).map(|i| &i.content),
            Some(Content::Note(_))
        ) {
            self.open_editor(Some(item), window, cx);
        } else {
            self.open_document(item, cx);
        }
    }
    fn open_knowledge_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(item) = self
            .knowledge
            .view
            .selected_node
            .as_deref()
            .and_then(|id| self.knowledge.graph.node(id))
            .and_then(|n| n.item)
        {
            self.open_knowledge_item(item, window, cx);
        }
    }
    fn copy_knowledge(&mut self, cx: &mut Context<Self>) {
        let Some(n) = self
            .knowledge
            .view
            .selected_node
            .as_deref()
            .and_then(|id| self.knowledge.graph.node(id))
        else {
            return;
        };
        if !matches!(n.kind, KnowledgeKind::Fact | KnowledgeKind::Note)
            || !matches!(
                n.status,
                KnowledgeStatus::Confirmed | KnowledgeStatus::Conflicting
            )
        {
            return;
        }
        let value = n.value.clone();
        self.knowledge.copied = Some(n.id.clone());
        cx.write_to_clipboard(ClipboardItem::new_string(value.clone()));
        self.copied_value = Some(zeroize::Zeroizing::new(value));
        self.clear_clipboard_later(cx);
        cx.notify();
    }
    fn move_knowledge_selection(&mut self, dx: f32, dy: f32, cx: &mut Context<Self>) {
        let start = self
            .knowledge
            .view
            .selected_node
            .as_deref()
            .and_then(|id| self.knowledge.graph.node(id))
            .map(|n| geometry::world(n.position))
            .unwrap_or_else(|| {
                geometry::world_fraction(self.knowledge.view.center_q, self.knowledge.view.center_r)
            });
        let next = self
            .knowledge
            .graph
            .nodes
            .iter()
            .filter_map(|n| {
                let p = geometry::world(n.position);
                let (x, y) = (p.x - start.x, p.y - start.y);
                let dot = x * dx + y * dy;
                if dot <= 0.1 {
                    return None;
                }
                let score = (x * x + y * y) / dot + ((x * dy - y * dx).abs() * 2.);
                Some((score, n.id.clone()))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        if let Some((_, id)) = next {
            self.select_knowledge(id, true, cx);
        }
    }
    pub(super) fn knowledge_view(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let selected = self.knowledge.view.selected_node.as_deref();
        let visible = self.knowledge.visible(window);
        let graph = self.knowledge.graph.clone();
        let view = self.knowledge.view.clone();
        let paths = self.knowledge.routes.clone();
        let matching = self.knowledge.matching.clone();
        let query = !self.knowledge.query.is_empty();
        let painted = visible.clone();
        let bounds = self.knowledge.bounds.clone();
        let entity = cx.entity().downgrade();
        let center = geometry::world_fraction(view.center_q, view.center_r);
        let zoom = view.zoom as f32;
        let cells = visible
            .into_iter()
            .map(|index| {
                let n = &self.knowledge.graph.nodes[index];
                let id = n.id.clone();
                let p = geometry::world(n.position);
                let source = matches!(
                    n.kind,
                    KnowledgeKind::Document | KnowledgeKind::Credential | KnowledgeKind::Entity
                );
                let selected = selected == Some(n.id.as_str());
                let review = n.status.needs_review() || n.status == KnowledgeStatus::Conflicting;
                let label = if n.kind == KnowledgeKind::Credential {
                    "Login"
                } else if n.kind == KnowledgeKind::Document {
                    "Source"
                } else {
                    &n.label
                };
                let value = if n.kind == KnowledgeKind::Document {
                    std::path::Path::new(&n.label)
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or(&n.label)
                        .to_owned()
                } else if n.kind == KnowledgeKind::Credential {
                    n.label.clone()
                } else {
                    n.value.clone()
                };
                let text_scale = zoom.min(1.);
                div()
                    .absolute()
                    .left(px(
                        (p.x - center.x) * zoom - map_style::TEXT_WIDTH * text_scale / 2.
                    ))
                    .top(px(
                        (p.y - center.y) * zoom - map_style::TEXT_HEIGHT * text_scale / 2.
                    ))
                    .w(px(map_style::TEXT_WIDTH * text_scale))
                    .h(px(map_style::TEXT_HEIGHT * text_scale))
                    .id(SharedString::from(format!("knowledge-{}", n.id)))
                    .rounded(px(radius::STANDARD))
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .text_color(rgb(if source { SURFACE } else { INK }))
                    .when(
                        query && !self.knowledge.matching.contains(&n.id) && !selected,
                        |s| s.opacity(map_style::DIM_OPACITY),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, cx| {
                            window.focus(&this.knowledge_focus);
                            cx.stop_propagation();
                        }),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.select_knowledge(id.clone(), false, cx)
                    }))
                    .when(self.knowledge.view.zoom >= map_style::DETAIL_ZOOM, |s| {
                        s.child(
                            div()
                                .w_full()
                                .text_center()
                                .type_style(Type::Caption)
                                .text_color(rgb(if source {
                                    SURFACE
                                } else if review {
                                    WARNING
                                } else {
                                    MUTED
                                }))
                                .text_ellipsis()
                                .child(label.to_owned()),
                        )
                    })
                    .when(self.knowledge.view.zoom >= map_style::DETAIL_ZOOM, |s| {
                        s.child(
                            div()
                                .w_full()
                                .text_center()
                                .type_style(Type::Small)
                                .when(!source, |s| s.font_family(font::MONO))
                                .text_ellipsis()
                                .child(if self.knowledge.view.zoom >= map_style::DETAIL_ZOOM {
                                    value
                                } else {
                                    n.label.clone()
                                }),
                        )
                    })
                    .when(
                        review && self.knowledge.view.zoom >= map_style::DETAIL_ZOOM,
                        |s| {
                            s.child(
                                div()
                                    .type_style(Type::Caption)
                                    .text_color(rgb(WARNING))
                                    .child(if n.status == KnowledgeStatus::Conflicting {
                                        "Conflict"
                                    } else if n.status == KnowledgeStatus::Unverified {
                                        "Unverified"
                                    } else {
                                        "Review"
                                    }),
                            )
                        },
                    )
                    .when(self.knowledge.view.zoom < map_style::DETAIL_ZOOM, |s| {
                        s.child(icon(
                            match n.kind {
                                KnowledgeKind::Document => Icon::Document,
                                KnowledgeKind::Credential => Icon::Card,
                                _ => Icon::Honeycomb,
                            },
                            IconSize::Medium,
                            if source {
                                SURFACE
                            } else if review {
                                WARNING
                            } else {
                                MUTED
                            },
                        ))
                    })
            })
            .collect::<Vec<_>>();
        let canvas = div()
            .id("knowledge-canvas")
            .relative()
            .flex_1()
            .min_h_0()
            .overflow_hidden()
            .track_focus(&self.knowledge_focus)
            .key_context("KnowledgeMap")
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "left" => this.move_knowledge_selection(-1., 0., cx),
                    "right" => this.move_knowledge_selection(1., 0., cx),
                    "up" => this.move_knowledge_selection(0., -1., cx),
                    "down" => this.move_knowledge_selection(0., 1., cx),
                    "enter" => this.open_knowledge_selected(window, cx),
                    "+" | "=" => this.zoom_knowledge(map_style::ZOOM_STEP, cx),
                    "-" => this.zoom_knowledge(-map_style::ZOOM_STEP, cx),
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &gpui::MouseDownEvent, window, _| {
                    this.knowledge.drag = Some(e.position);
                    window.focus(&this.knowledge_focus);
                }),
            )
            .on_mouse_move(cx.listener(|this, e: &gpui::MouseMoveEvent, _, cx| {
                if !e.dragging() {
                    this.knowledge.drag = None;
                    return;
                }
                if let Some(previous) = this.knowledge.drag.replace(e.position) {
                    this.pan_knowledge(e.position - previous, cx);
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.knowledge.drag = None),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.knowledge.drag = None),
            )
            .on_scroll_wheel(cx.listener(|this, e: &gpui::ScrollWheelEvent, _, cx| {
                let delta = e.delta.pixel_delta(px(space::XL));
                if e.modifiers.control || e.modifiers.platform {
                    this.zoom_knowledge(
                        if delta.y > px(0.) {
                            map_style::ZOOM_STEP
                        } else {
                            -map_style::ZOOM_STEP
                        },
                        cx,
                    );
                } else {
                    this.pan_knowledge(
                        if e.modifiers.shift && delta.x == px(0.) {
                            point(delta.y, px(0.))
                        } else {
                            delta
                        },
                        cx,
                    );
                }
                cx.stop_propagation();
            }))
            .child(
                canvas(
                    move |new_bounds, _, cx| {
                        let old = bounds.replace(new_bounds);
                        if old.size != new_bounds.size {
                            cx.defer(move |cx| {
                                let _ = entity.update(cx, |_, cx| cx.notify());
                            });
                        }
                    },
                    move |bounds, _, window, _| {
                        geometry::paint(
                            bounds,
                            &graph,
                            &view,
                            &painted,
                            &paths,
                            query.then_some(&matching),
                            window,
                        )
                    },
                )
                .absolute()
                .size_full(),
            )
            .child(
                div()
                    .absolute()
                    .left(relative(0.5))
                    .top(relative(0.5))
                    .children(cells),
            );
        div().size_full().flex().flex_col().min_h_0()
            .child(div().pt(px(space::PAGE_TOP)).px(px(space::XXL)).pb(px(space::LG)).flex_shrink_0().flex().flex_col().gap(px(space::MD))
                .child(div().flex().items_center().justify_between().gap(px(space::LG)).child(heading("Knowledge"))
                    .child(div().flex().items_center().gap(px(space::SM))
                        .child(secondary_action().id("knowledge-refresh").hover(|s|s.bg(rgb(HOVER))).on_click(cx.listener(|this,_,_,cx|this.refresh_knowledge(cx))).child(if self.knowledge.loading{"Updating…"}else{"Refresh"}))
                        .child(primary_action().id("knowledge-add").hover(|s|s.bg(rgb(PRIMARY_HOVER))).on_click(cx.listener(|this,_,_,cx|{this.show_add=true;cx.notify();})).child(icon(Icon::Plus,IconSize::Medium,SURFACE)).child("Add"))))
                .child(div().flex().items_center().gap(px(space::LG))
                    .child(div().flex_1().min_w_0().h(px(layout::CONTROL)).px(px(space::MD)).rounded(px(radius::STANDARD)).border_1().border_color(rgb(LINE)).bg(rgb(SURFACE)).flex().items_center().gap(px(space::SM)).child(icon(Icon::Search,IconSize::Medium,MUTED)).child(self.knowledge_input.clone()))
                    .child(div().type_style(Type::Caption).text_color(rgb(MUTED)).child(if query{format!("{} matches",self.knowledge.matches.len())}else{format!("{} items · {} groups",self.knowledge.graph.nodes.len(),self.knowledge.graph.groups.len())})))
                .when(query&&!self.knowledge.matches.is_empty(),|s|s.child(div().id("knowledge-next-match").type_style(Type::Small).text_color(rgb(ACCENT)).cursor_pointer().on_click(cx.listener(|this,_,_,cx|{if this.knowledge.matches.is_empty(){return;}this.knowledge.match_index=(this.knowledge.match_index+1)%this.knowledge.matches.len();let id=this.knowledge.matches[this.knowledge.match_index].clone();this.select_knowledge(id,true,cx);})).child("Next match →")))
                .when_some(self.knowledge.error.clone().or_else(||self.knowledge.save_error.clone()),|s,error|s.child(div().type_style(Type::Small).text_color(rgb(DANGER)).child(error))))
            .child(if self.knowledge.graph.nodes.is_empty(){
                div().flex_1().flex().flex_col().items_center().justify_center().px(px(space::XXL)).gap(px(space::LG))
                    .child(icon(Icon::Honeycomb,IconSize::Large,ACCENT))
                    .child(div().type_style(Type::Section).child(if self.knowledge.loading{"Loading your knowledge…"}else if self.knowledge.error.is_some(){"Your map could not be loaded"}else{"Your knowledge starts here"}))
                    .child(div().type_style(Type::Body).text_color(rgb(MUTED)).text_center().child("Add a detail or a document. Related information will settle into its own place.")).into_any_element()
            }else{canvas.into_any_element()})
            .child(self.knowledge_footer(cx))
    }
    fn knowledge_footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let node = self
            .knowledge
            .view
            .selected_node
            .as_deref()
            .and_then(|id| self.knowledge.graph.node(id));
        let selected = node.map(|n| n.id.as_str());
        let connected = self
            .knowledge
            .graph
            .edges
            .iter()
            .filter(|e| Some(e.from.as_str()) == selected || Some(e.to.as_str()) == selected)
            .collect::<Vec<_>>();
        let groups = self
            .knowledge
            .graph
            .groups
            .iter()
            .map(|g| {
                let id = g.id.clone();
                let label = g.label.clone();
                let anchor = g.anchor;
                secondary_action()
                    .id(SharedString::from(format!("knowledge-group-{id}")))
                    .max_w(px(180.))
                    .flex_shrink_0()
                    .hover(|s| s.bg(rgb(HOVER)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.knowledge.view.center_q = f64::from(anchor.q);
                        this.knowledge.view.center_r = f64::from(anchor.r);
                        this.save_knowledge_view(cx);
                        cx.notify();
                    }))
                    .child(div().text_ellipsis().child(label))
            })
            .collect::<Vec<_>>();
        div()
            .flex_shrink_0()
            .border_t_1()
            .border_color(rgb(LINE))
            .bg(rgb(SURFACE))
            .px(px(space::XXL))
            .py(px(space::LG))
            .flex()
            .flex_col()
            .gap(px(space::MD))
            .when_some(node, |s, n| {
                let item = n.item;
                let review = n.status.needs_review();
                let copy = matches!(n.kind, KnowledgeKind::Fact | KnowledgeKind::Note)
                    && matches!(
                        n.status,
                        KnowledgeStatus::Confirmed | KnowledgeStatus::Conflicting
                    );
                s.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(space::LG))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap(px(space::XS))
                                .child(
                                    div()
                                        .flex()
                                        .gap(px(space::MD))
                                        .child(
                                            div()
                                                .type_style(Type::Caption)
                                                .text_color(rgb(MUTED))
                                                .text_ellipsis()
                                                .child(n.label.clone()),
                                        )
                                        .child(
                                            div()
                                                .type_style(Type::Caption)
                                                .text_color(rgb(
                                                    if review
                                                        || n.status == KnowledgeStatus::Conflicting
                                                    {
                                                        WARNING
                                                    } else {
                                                        MUTED
                                                    },
                                                ))
                                                .child(n.status.label()),
                                        ),
                                )
                                .child(
                                    div()
                                        .id("knowledge-value-scroll")
                                        .max_h(px(72.))
                                        .overflow_y_scroll()
                                        .type_style(Type::Value)
                                        .font_family(font::MONO)
                                        .child(if n.value.is_empty() {
                                            n.label.clone()
                                        } else {
                                            n.value.clone()
                                        }),
                                ),
                        )
                        .child(
                            div()
                                .flex()
                                .gap(px(space::SM))
                                .when(copy, |s| {
                                    s.child(
                                        secondary_action()
                                            .id("knowledge-copy")
                                            .hover(|s| s.bg(rgb(HOVER)))
                                            .on_click(
                                                cx.listener(|this, _, _, cx| {
                                                    this.copy_knowledge(cx)
                                                }),
                                            )
                                            .child(
                                                if self.knowledge.copied.as_deref() == Some(&n.id) {
                                                    "Copied"
                                                } else {
                                                    "Copy"
                                                },
                                            ),
                                    )
                                })
                                .when_some(item, |s, item| {
                                    s.child(
                                        secondary_action()
                                            .id("knowledge-open")
                                            .hover(|s| s.bg(rgb(HOVER)))
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.open_knowledge_item(item, window, cx)
                                            }))
                                            .child(if n.kind == KnowledgeKind::Note {
                                                "Edit"
                                            } else {
                                                "Open"
                                            }),
                                    )
                                })
                                .when(review, |s| {
                                    s.child(
                                        primary_action()
                                            .id("knowledge-review")
                                            .hover(|s| s.bg(rgb(PRIMARY_HOVER)))
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.navigate(Page::Review, window, cx)
                                            }))
                                            .child("Review"),
                                    )
                                }),
                        ),
                )
                .when(!n.context.is_empty(), |s| {
                    s.child(
                        div()
                            .type_style(Type::Caption)
                            .text_color(rgb(MUTED))
                            .line_clamp(2)
                            .child(n.context.clone()),
                    )
                })
                .when(!connected.is_empty(), |s| {
                    s.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(space::MD))
                            .child(
                                div()
                                    .id("knowledge-toggle-evidence")
                                    .type_style(Type::Small)
                                    .text_color(rgb(ACCENT))
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.knowledge.expanded = !this.knowledge.expanded;
                                        cx.notify();
                                    }))
                                    .child(format!(
                                        "{} {} {}",
                                        if self.knowledge.expanded {
                                            "Hide"
                                        } else {
                                            "Show"
                                        },
                                        connected.len(),
                                        if connected.len() == 1 {
                                            "connection"
                                        } else {
                                            "connections"
                                        }
                                    )),
                            )
                            .child(
                                div()
                                    .type_style(Type::Caption)
                                    .text_color(rgb(MUTED))
                                    .text_ellipsis()
                                    .child(
                                        connected
                                            .first()
                                            .map(|e| e.label_from(selected.unwrap_or_default()))
                                            .unwrap_or_default(),
                                    ),
                            ),
                    )
                })
                .when(connected.is_empty() && n.kind == KnowledgeKind::Note, |s| {
                    s.child(eyebrow("Entered by you"))
                })
            })
            .when(self.knowledge.expanded, |s| {
                s.child(
                    div()
                        .id("knowledge-evidence-scroll")
                        .max_h(px(152.))
                        .overflow_y_scroll()
                        .flex()
                        .flex_col()
                        .gap(px(space::MD))
                        .children(connected.into_iter().enumerate().filter_map(|(i, e)| {
                            let id = if Some(e.from.as_str()) == selected {
                                &e.to
                            } else {
                                &e.from
                            };
                            let n = self.knowledge.graph.node(id)?;
                            let target = id.clone();
                            let item = n.item;
                            Some(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(space::XS))
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(px(space::MD))
                                            .child(
                                                div()
                                                    .id(("knowledge-evidence", i))
                                                    .flex_1()
                                                    .min_w_0()
                                                    .type_style(Type::Small)
                                                    .text_color(rgb(ACCENT))
                                                    .cursor_pointer()
                                                    .on_click(cx.listener(move |this, _, _, cx| {
                                                        this.select_knowledge(
                                                            target.clone(),
                                                            true,
                                                            cx,
                                                        )
                                                    }))
                                                    .child(format!(
                                                        "{} · {}{}",
                                                        e.label_from(selected.unwrap_or_default()),
                                                        n.label,
                                                        if e.location.is_empty() {
                                                            String::new()
                                                        } else {
                                                            format!(" · {}", e.location)
                                                        }
                                                    )),
                                            )
                                            .when_some(item, |s, item| {
                                                s.child(
                                                    secondary_action()
                                                        .id(("knowledge-source-open", i))
                                                        .hover(|s| s.bg(rgb(HOVER)))
                                                        .on_click(cx.listener(
                                                            move |this, _, window, cx| {
                                                                this.open_knowledge_item(
                                                                    item, window, cx,
                                                                )
                                                            },
                                                        ))
                                                        .child("Open"),
                                                )
                                            }),
                                    )
                                    .when(!e.quote.is_empty(), |s| {
                                        s.child(
                                            div()
                                                .type_style(Type::Small)
                                                .text_color(rgb(MUTED))
                                                .child(e.quote.clone()),
                                        )
                                    }),
                            )
                        })),
                )
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::MD))
                    .child(
                        div()
                            .id("knowledge-groups")
                            .flex_1()
                            .min_w_0()
                            .overflow_x_scroll()
                            .flex()
                            .gap(px(space::SM))
                            .children(groups),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .gap(px(space::SM))
                            .child(
                                secondary_action()
                                    .id("knowledge-zoom-out")
                                    .hover(|s| s.bg(rgb(HOVER)))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.zoom_knowledge(-map_style::ZOOM_STEP, cx)
                                    }))
                                    .child("−"),
                            )
                            .child(eyebrow(format!(
                                "{}%",
                                (self.knowledge.view.zoom * 100.).round() as u32
                            )))
                            .child(
                                secondary_action()
                                    .id("knowledge-zoom-in")
                                    .hover(|s| s.bg(rgb(HOVER)))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.zoom_knowledge(map_style::ZOOM_STEP, cx)
                                    }))
                                    .child("+"),
                            ),
                    ),
            )
    }
}
