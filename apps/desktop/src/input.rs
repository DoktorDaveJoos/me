use crate::theme::*;
// Adapted from GPUI 0.2.2 examples/input.rs, Copyright Zed Industries.
// Licensed under Apache-2.0; see assets/licenses/GPUI-APACHE-2.0.txt.
// Changes: ME styling, scoped cross-platform bindings, focus, horizontal scrolling,
// IME range conversion, and reusable public API.

use std::ops::Range;

use gpui::{
    App, Bounds, ClipboardItem, Context, CursorStyle, ElementId, ElementInputHandler, Entity,
    EntityInputHandler, FocusHandle, Focusable, GlobalElementId, KeyBinding, LayoutId, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point, ShapedLine,
    SharedString, Style, TextRun, UTF16Selection, UnderlineStyle, Window, actions, div, fill,
    point, prelude::*, px, relative, rgb, rgba, size,
};
use unicode_segmentation::*;

actions!(
    text_input,
    [
        Backspace,
        Delete,
        Left,
        Right,
        SelectLeft,
        SelectRight,
        SelectAll,
        Home,
        End,
        ShowCharacterPalette,
        Paste,
        Cut,
        Copy,
        Newline,
        UpLine,
        DownLine,
    ]
);

pub struct TextInput {
    focus_handle: FocusHandle,
    secret: bool,
    multiline: bool,
    last_lines: Vec<(usize, ShapedLine)>,
    last_line_height: Pixels,
    file_paste: bool,
    pub content: SharedString,
    placeholder: SharedString,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    last_layout: Option<ShapedLine>,
    last_bounds: Option<Bounds<Pixels>>,
    is_selecting: bool,
    scroll_offset: Pixels,
}

impl TextInput {
    pub fn bounds(&self) -> Option<Bounds<Pixels>> {
        self.last_bounds
    }
    pub fn multiline(placeholder: &str, cx: &mut Context<Self>) -> Self {
        let mut input = Self::new(placeholder, cx);
        input.multiline = true;
        input
    }
    pub fn set_concealed(&mut self, concealed: bool, cx: &mut Context<Self>) {
        self.secret = concealed;
        self.last_lines.clear();
        self.last_layout = None;
        self.scroll_offset = px(0.);
        cx.notify();
    }
    fn newline(&mut self, _: &Newline, window: &mut Window, cx: &mut Context<Self>) {
        if self.multiline {
            self.replace_text_in_range(None, "\n", window, cx);
        } else {
            cx.propagate();
        }
    }
    fn up_line(&mut self, _: &UpLine, _: &mut Window, cx: &mut Context<Self>) {
        self.move_line(-1, cx);
    }
    fn down_line(&mut self, _: &DownLine, _: &mut Window, cx: &mut Context<Self>) {
        self.move_line(1, cx);
    }
    fn move_line(&mut self, direction: isize, cx: &mut Context<Self>) {
        if !self.multiline || self.secret || self.last_lines.is_empty() {
            cx.propagate();
            return;
        }
        let cursor = self.cursor_offset();
        let index = self
            .last_lines
            .iter()
            .rposition(|(start, _)| *start <= cursor)
            .unwrap_or(0);
        let x = self.last_lines[index]
            .1
            .x_for_index(cursor - self.last_lines[index].0);
        let next = index
            .saturating_add_signed(direction)
            .min(self.last_lines.len() - 1);
        self.move_to(
            self.last_lines[next].0 + self.last_lines[next].1.closest_index_for_x(x),
            cx,
        );
    }
    fn multiline_index(&self, position: Point<Pixels>) -> usize {
        let Some(bounds) = self.last_bounds else {
            return 0;
        };
        if self.last_lines.is_empty() {
            return 0;
        }
        let height = self.last_line_height;
        let index = (((position.y - bounds.top()) / height).max(0.) as usize)
            .min(self.last_lines.len() - 1);
        let (start, line) = &self.last_lines[index];
        start + line.closest_index_for_x(position.x - bounds.left() + self.scroll_offset)
    }

    pub fn filter(cx: &mut Context<Self>) -> Self {
        let mut input = Self::new("Find anything, or drop a form…", cx);
        input.file_paste = true;
        input
    }
    pub fn password(cx: &mut Context<Self>) -> Self {
        Self::secret("Master password", cx)
    }
    pub fn secret(placeholder: &str, cx: &mut Context<Self>) -> Self {
        let mut input = Self::new(placeholder, cx);
        input.secret = true;
        input
    }
    /// An inert field snapshot while its submitted value is being processed.
    /// Secret values use exactly the same mask as the interactive editor.
    pub fn frozen(&self) -> impl IntoElement {
        let content: SharedString = if self.secret {
            masked_text(&self.content).into()
        } else {
            self.content.clone()
        };
        div()
            .w_full()
            .min_w_0()
            .overflow_hidden()
            .type_style(Type::Body)
            .text_color(rgb(if content.is_empty() { FAINT } else { INK }))
            .child(if content.is_empty() {
                self.placeholder.clone()
            } else {
                content
            })
    }
    fn display_offset(&self, offset: usize) -> usize {
        if self.secret {
            self.content[..offset].chars().count()
        } else {
            offset
        }
    }
    fn content_offset(&self, offset: usize) -> usize {
        if self.secret {
            self.content
                .char_indices()
                .nth(offset)
                .map_or(self.content.len(), |(i, _)| i)
        } else {
            offset
        }
    }
    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.previous_boundary(self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.start, cx)
        }
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.next_boundary(self.selected_range.end), cx);
        } else {
            self.move_to(self.selected_range.end, cx)
        }
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_boundary(self.cursor_offset()), cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_boundary(self.cursor_offset()), cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
        self.select_to(self.content.len(), cx)
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.content.len(), cx);
    }

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.previous_boundary(self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.next_boundary(self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle);
        self.is_selecting = true;

        if event.modifiers.shift {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        } else {
            self.move_to(self.index_for_mouse_position(event.position), cx)
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _window: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_selecting {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        }
    }

    fn show_character_palette(
        &mut self,
        _: &ShowCharacterPalette,
        window: &mut Window,
        _: &mut Context<Self>,
    ) {
        window.show_character_palette();
    }

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if self.file_paste {
            cx.propagate();
            return;
        }
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.paste_text(&text, window, cx);
        }
    }

    pub fn paste_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.replace_text_in_range(
            None,
            &pasted_text(text, self.multiline, self.secret),
            window,
            cx,
        );
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if self.secret {
            return;
        }
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
        }
    }
    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if self.secret {
            return;
        }
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
            self.replace_text_in_range(None, "", window, cx)
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selected_range = offset..offset;
        self.selection_reversed = false;
        cx.notify()
    }

    fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        if self.multiline && !self.secret {
            return self.multiline_index(position);
        }
        if self.content.is_empty() {
            return 0;
        }

        let (Some(bounds), Some(line)) = (self.last_bounds.as_ref(), self.last_layout.as_ref())
        else {
            return 0;
        };
        if position.y < bounds.top() {
            return 0;
        }
        if position.y > bounds.bottom() {
            return self.content.len();
        }
        self.content_offset(
            line.closest_index_for_x(position.x - bounds.left() + self.scroll_offset),
        )
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        if self.selection_reversed {
            self.selected_range.start = offset
        } else {
            self.selected_range.end = offset
        };
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        cx.notify()
    }

    fn offset_from_utf16(&self, offset: usize) -> usize {
        let mut utf8_offset = 0;
        let mut utf16_count = 0;

        for ch in self.content.chars() {
            if utf16_count >= offset {
                break;
            }
            utf16_count += ch.len_utf16();
            utf8_offset += ch.len_utf8();
        }

        utf8_offset
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        let mut utf16_offset = 0;
        let mut utf8_count = 0;

        for ch in self.content.chars() {
            if utf8_count >= offset {
                break;
            }
            utf8_count += ch.len_utf8();
            utf16_offset += ch.len_utf16();
        }

        utf16_offset
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    fn range_from_utf16(&self, range_utf16: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range_utf16.start)..self.offset_from_utf16(range_utf16.end)
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .rev()
            .find_map(|(idx, _)| (idx < offset).then_some(idx))
            .unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .find_map(|(idx, _)| (idx > offset).then_some(idx))
            .unwrap_or(self.content.len())
    }

    pub fn new(placeholder: &str, cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            secret: false,
            multiline: false,
            last_lines: Vec::new(),
            last_line_height: px(1.),
            file_paste: false,
            content: "".into(),
            placeholder: placeholder.to_owned().into(),
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            last_layout: None,
            last_bounds: None,
            is_selecting: false,
            scroll_offset: px(0.),
        }
    }

    pub fn set_text(&mut self, value: &str, cx: &mut Context<Self>) {
        self.content = value.to_owned().into();
        self.selected_range = value.len()..value.len();
        self.selection_reversed = false;
        self.marked_range = None;
        self.last_layout = None;
        self.last_lines.clear();
        self.last_bounds = None;
        self.is_selecting = false;
        self.scroll_offset = px(0.);
        cx.notify();
    }
}

impl EntityInputHandler for TextInput {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        if self.secret {
            return None;
        }
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.content[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected_range),
            reversed: self.selection_reversed,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.marked_range = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());

        self.content =
            (self.content[0..range.start].to_owned() + new_text + &self.content[range.end..])
                .into();
        self.selected_range = range.start + new_text.len()..range.start + new_text.len();
        self.selection_reversed = false;
        self.marked_range.take();
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());

        self.content =
            (self.content[0..range.start].to_owned() + new_text + &self.content[range.end..])
                .into();
        if !new_text.is_empty() {
            self.marked_range = Some(range.start..range.start + new_text.len());
        } else {
            self.marked_range = None;
        }
        self.selection_reversed = false;
        self.selected_range = new_selected_range_utf16
            .as_ref()
            .map(|range_utf16| {
                let utf8 = |offset: usize| {
                    let mut units = 0;
                    new_text
                        .char_indices()
                        .find_map(|(i, ch)| {
                            if units >= offset {
                                Some(i)
                            } else {
                                units += ch.len_utf16();
                                None
                            }
                        })
                        .unwrap_or(new_text.len())
                };
                range.start + utf8(range_utf16.start)..range.start + utf8(range_utf16.end)
            })
            .unwrap_or_else(|| range.start + new_text.len()..range.start + new_text.len());

        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        if self.multiline && !self.secret {
            let range = self.range_from_utf16(&range_utf16);
            let index = self
                .last_lines
                .iter()
                .rposition(|(start, _)| *start <= range.start)?;
            let (start, line) = &self.last_lines[index];
            let height = self.last_line_height;
            let x = line.x_for_index(range.start - start) - self.scroll_offset;
            return Some(Bounds::new(
                point(bounds.left() + x, bounds.top() + height * index as f32),
                size(px(1.), height),
            ));
        }
        let last_layout = self.last_layout.as_ref()?;
        let range = self.range_from_utf16(&range_utf16);
        Some(Bounds::from_corners(
            point(
                bounds.left() + last_layout.x_for_index(self.display_offset(range.start))
                    - self.scroll_offset,
                bounds.top(),
            ),
            point(
                bounds.left() + last_layout.x_for_index(self.display_offset(range.end))
                    - self.scroll_offset,
                bounds.bottom(),
            ),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: gpui::Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        if self.multiline && !self.secret {
            return Some(self.offset_to_utf16(self.multiline_index(point)));
        }
        if self.content.is_empty() {
            return Some(0);
        }
        let bounds = self.last_bounds?;
        let last_layout = self.last_layout.as_ref()?;
        let utf8_index =
            last_layout.closest_index_for_x(point.x - bounds.left() + self.scroll_offset);
        Some(self.offset_to_utf16(self.content_offset(utf8_index)))
    }
}

struct TextElement {
    input: Entity<TextInput>,
}

struct PrepaintState {
    line: Option<ShapedLine>,
    cursor: Option<PaintQuad>,
    selection: Option<PaintQuad>,
    scroll: Pixels,
}

impl IntoElement for TextElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextElement {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = window.line_height().into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let input = self.input.read(cx);
        let content: SharedString = if input.secret {
            masked_text(&input.content).into()
        } else {
            input.content.clone()
        };
        let selected_range = input.display_offset(input.selected_range.start)
            ..input.display_offset(input.selected_range.end);
        let cursor = input.display_offset(input.cursor_offset());
        let style = window.text_style();

        let (display_text, text_color) = if content.is_empty() {
            (input.placeholder.clone(), rgb(crate::theme::FAINT).into())
        } else {
            (content, style.color)
        };

        let run = TextRun {
            len: display_text.len(),
            font: style.font(),
            color: text_color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let marked_display = input
            .marked_range
            .as_ref()
            .map(|r| input.display_offset(r.start)..input.display_offset(r.end));
        let runs = if let Some(marked_range) = marked_display.as_ref() {
            vec![
                TextRun {
                    len: marked_range.start,
                    ..run.clone()
                },
                TextRun {
                    len: marked_range.end - marked_range.start,
                    underline: Some(UnderlineStyle {
                        color: Some(run.color),
                        thickness: px(1.0),
                        wavy: false,
                    }),
                    ..run.clone()
                },
                TextRun {
                    len: display_text.len() - marked_range.end,
                    ..run
                },
            ]
            .into_iter()
            .filter(|run| run.len > 0)
            .collect()
        } else {
            vec![run]
        };

        let font_size = style.font_size.to_pixels(window.rem_size());
        let line = window
            .text_system()
            .shape_line(display_text, font_size, &runs, None);

        let cursor_pos = line.x_for_index(cursor);
        let available = (bounds.size.width - px(3.)).max(px(0.));
        let mut scroll = input.scroll_offset;
        if cursor_pos - scroll > available {
            scroll = cursor_pos - available;
        }
        if cursor_pos < scroll {
            scroll = cursor_pos;
        }
        scroll = scroll.min((line.width - available).max(px(0.)));
        let cursor_pos = cursor_pos - scroll;
        let (selection, cursor) = if selected_range.is_empty() {
            (
                None,
                Some(fill(
                    Bounds::new(
                        point(bounds.left() + cursor_pos, bounds.top()),
                        size(px(1.), bounds.bottom() - bounds.top()),
                    ),
                    rgb(crate::theme::INK),
                )),
            )
        } else {
            (
                Some(fill(
                    Bounds::from_corners(
                        point(
                            bounds.left() + line.x_for_index(selected_range.start) - scroll,
                            bounds.top(),
                        ),
                        point(
                            bounds.left() + line.x_for_index(selected_range.end) - scroll,
                            bounds.bottom(),
                        ),
                    ),
                    rgba(SELECTION),
                )),
                None,
            )
        };
        PrepaintState {
            line: Some(line),
            cursor,
            selection,
            scroll,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.input.read(cx).focus_handle.clone();
        window.handle_input(
            &focus_handle,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );
        if let Some(selection) = prepaint.selection.take() {
            window.paint_quad(selection)
        }
        let line = prepaint.line.take().unwrap();
        line.paint(
            point(bounds.left() - prepaint.scroll, bounds.top()),
            window.line_height(),
            window,
            cx,
        )
        .unwrap();

        if focus_handle.is_focused(window)
            && let Some(cursor) = prepaint.cursor.take()
        {
            window.paint_quad(cursor);
        }

        self.input.update(cx, |input, _cx| {
            input.scroll_offset = prepaint.scroll;
            input.last_layout = Some(line);
            input.last_bounds = Some(bounds);
        });
    }
}

impl Render for TextInput {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .key_context(if self.multiline {
                "TextInput MultilineInput"
            } else {
                "TextInput"
            })
            .track_focus(&self.focus_handle(cx))
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::show_character_palette))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::newline))
            .on_action(cx.listener(Self::up_line))
            .on_action(cx.listener(Self::down_line))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .w_full()
            .min_w_0()
            .overflow_hidden()
            .font_family(font::SANS)
            .text_color(rgb(crate::theme::INK))
            .type_style(Type::Body)
            .when(self.multiline && !self.secret, |s| {
                s.child(MultilineElement { input: cx.entity() })
            })
            .when(!self.multiline || self.secret, |s| {
                s.child(TextElement { input: cx.entity() })
            })
    }
}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

pub fn register_bindings(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("enter", Newline, Some("MultilineInput")),
        KeyBinding::new("up", UpLine, Some("MultilineInput")),
        KeyBinding::new("down", DownLine, Some("MultilineInput")),
        KeyBinding::new("backspace", Backspace, Some("TextInput")),
        KeyBinding::new("delete", Delete, Some("TextInput")),
        KeyBinding::new("left", Left, Some("TextInput")),
        KeyBinding::new("right", Right, Some("TextInput")),
        KeyBinding::new("shift-left", SelectLeft, Some("TextInput")),
        KeyBinding::new("shift-right", SelectRight, Some("TextInput")),
        KeyBinding::new("home", Home, Some("TextInput")),
        KeyBinding::new("end", End, Some("TextInput")),
    ]);
    let modifier = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    // Native macOS dialogs also need these menu key equivalents.
    // GPUI handles them only when the focused view has an action listener.
    cx.bind_keys([
        KeyBinding::new(&format!("{modifier}-a"), SelectAll, None),
        KeyBinding::new(&format!("{modifier}-v"), Paste, None),
        KeyBinding::new(&format!("{modifier}-c"), Copy, None),
        KeyBinding::new(&format!("{modifier}-x"), Cut, None),
    ]);
}

// Multiline editing uses the same selection, clipboard, grapheme and IME model
// as single-line inputs. Hard line breaks are retained verbatim.
struct MultilineElement {
    input: Entity<TextInput>,
}
struct MultilinePaint {
    lines: Vec<(usize, ShapedLine)>,
    selection: Vec<PaintQuad>,
    cursor: Option<PaintQuad>,
    scroll: Pixels,
}
impl IntoElement for MultilineElement {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
impl Element for MultilineElement {
    type RequestLayoutState = ();
    type PrepaintState = MultilinePaint;
    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let rows = self.input.read(cx).content.split('\n').count().max(3);
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = (window.line_height() * rows as f32).into();
        (window.request_layout(style, [], cx), ())
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> MultilinePaint {
        let input = self.input.read(cx);
        let style = window.text_style();
        let height = window.line_height();
        let mut lines = Vec::new();
        let mut offset = 0;
        for text in input.content.split('\n') {
            let display: SharedString = if input.content.is_empty() {
                input.placeholder.clone()
            } else {
                if input.secret {
                    masked_text(text).into()
                } else {
                    text.to_owned().into()
                }
            };
            let run = TextRun {
                len: display.len(),
                font: style.font(),
                color: if input.content.is_empty() {
                    rgb(FAINT).into()
                } else {
                    style.color
                },
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            lines.push((
                offset,
                window.text_system().shape_line(
                    display,
                    style.font_size.to_pixels(window.rem_size()),
                    &[run],
                    None,
                ),
            ));
            offset += text.len() + 1;
        }
        let cursor = input.cursor_offset();
        let row = lines
            .iter()
            .rposition(|(start, _)| *start <= cursor)
            .unwrap_or(0);
        let cursor_x = lines[row].1.x_for_index(cursor - lines[row].0);
        let available = (bounds.size.width - px(3.)).max(px(0.));
        let mut scroll = input.scroll_offset;
        if cursor_x - scroll > available {
            scroll = cursor_x - available;
        }
        if cursor_x < scroll {
            scroll = cursor_x;
        }
        let mut selection = Vec::new();
        for (i, (start, line)) in lines.iter().enumerate() {
            let end = lines.get(i + 1).map_or(input.content.len(), |(s, _)| s - 1);
            let left = input.selected_range.start.max(*start);
            let right = input.selected_range.end.min(end);
            if left <= right
                && input.selected_range.start <= end
                && input.selected_range.end > *start
                && !input.selected_range.is_empty()
            {
                let x1 = line.x_for_index(left - start) - scroll;
                let x2 = if input.selected_range.end > end {
                    line.width + px(space::SM)
                } else {
                    line.x_for_index(right - start)
                } - scroll;
                selection.push(fill(
                    Bounds::new(
                        point(bounds.left() + x1, bounds.top() + height * i as f32),
                        size((x2 - x1).max(px(1.)), height),
                    ),
                    rgba(SELECTION),
                ));
            }
        }
        let cursor = input.selected_range.is_empty().then(|| {
            fill(
                Bounds::new(
                    point(
                        bounds.left() + cursor_x - scroll,
                        bounds.top() + height * row as f32,
                    ),
                    size(px(1.), height),
                ),
                rgb(INK),
            )
        });
        MultilinePaint {
            lines,
            selection,
            cursor,
            scroll,
        }
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        paint: &mut MultilinePaint,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus = self.input.read(cx).focus_handle.clone();
        window.handle_input(
            &focus,
            ElementInputHandler::new(bounds, self.input.clone()),
            cx,
        );
        for quad in paint.selection.drain(..) {
            window.paint_quad(quad);
        }
        let height = window.line_height();
        for (i, (_, line)) in paint.lines.iter().enumerate() {
            let _ = line.paint(
                point(
                    bounds.left() - paint.scroll,
                    bounds.top() + height * i as f32,
                ),
                height,
                window,
                cx,
            );
        }
        if focus.is_focused(window)
            && let Some(cursor) = paint.cursor.take()
        {
            window.paint_quad(cursor);
        }
        self.input.update(cx, |input, _| {
            input.last_lines = std::mem::take(&mut paint.lines);
            input.last_bounds = Some(bounds);
            input.last_line_height = height;
            input.scroll_offset = paint.scroll;
        });
    }
}

fn masked_text(text: &str) -> String {
    "*".repeat(text.chars().count())
}

fn pasted_text(text: &str, multiline: bool, secret: bool) -> std::borrow::Cow<'_, str> {
    if multiline || secret {
        std::borrow::Cow::Borrowed(text)
    } else {
        std::borrow::Cow::Owned(text.replace('\n', " "))
    }
}
#[cfg(test)]
mod paste_tests {
    use super::pasted_text;
    #[test]
    fn masks_multiline_secrets_and_unicode_without_exposing_contents() {
        let secret = "SYNTHETIC-API-KEY\nprivate 🗝";
        let masked = super::masked_text(secret);
        assert_eq!(masked.len(), secret.chars().count());
        assert!(masked.bytes().all(|b| b == b'*'));
        assert!(!masked.contains("SYNTHETIC"));
    }
    #[test]
    fn preserves_multiline_notes_and_exact_secret_bytes() {
        let text = "  café 日本語\nsecond line\t  ";
        assert_eq!(pasted_text(text, true, false), text);
        assert_eq!(pasted_text(text, false, true), text);
        assert_eq!(pasted_text("first\nsecond", false, false), "first second");
    }
}
