//! The inspector window. State changes come from the worker channel;
//! clicks only enqueue a [`crate::worker::Cmd`].

use std::sync::mpsc::Sender;

use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::clipboard::Clipboard;
use gpui_kit::component::collapsible::Collapsible;
use gpui_kit::component::list::ListItem;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ActiveTheme as _, IndexPath, Sizable as _, Size};
use gpui_kit::prelude::*;
use gpui_kit::{
    div, px, App, ClickEvent, ClipboardItem, Context, Entity, FontWeight, IntoElement, Render,
    SharedString, Task, WeakEntity, Window,
};

use crlib::ReaderSnapshot;

use crate::format::plaintext;
use crate::model::{AppState, CardRead, Event, Status};
use crate::worker::{self, Cmd};

pub struct CrApp {
    state: AppState,
    select: Entity<SelectState<Vec<String>>>,
    cmd_tx: Sender<Cmd>,
    dump_open: bool,
    log_open: bool,
    /// Reader list changed; the select is refreshed at the start of the next frame.
    select_dirty: bool,
    /// Keeps the event loop alive for as long as the window is.
    _events: Task<()>,
}

impl CrApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (cmd_tx, cmd_rx) = std::sync::mpsc::channel();
        let (event_tx, event_rx) = smol::channel::unbounded();
        worker::spawn(cmd_rx, event_tx);

        let select = cx.new(|cx| SelectState::new(Vec::<String>::new(), None, window, cx));
        cx.subscribe(&select, |this, _, event: &SelectEvent<Vec<String>>, cx| {
            if let SelectEvent::Confirm(Some(name)) = event {
                this.choose_reader(name.clone(), cx);
            }
        })
        .detach();

        let events = cx.spawn(async move |this, cx| {
            while let Ok(event) = event_rx.recv().await {
                let applied = this
                    .update(cx, |this, cx| {
                        this.on_event(event, cx);
                    })
                    .is_ok();
                if !applied {
                    break;
                }
            }
        });

        Self {
            state: AppState::new(),
            select,
            cmd_tx,
            dump_open: false,
            log_open: false,
            select_dirty: false,
            _events: events,
        }
    }

    fn choose_reader(&mut self, name: String, cx: &mut Context<Self>) {
        if let Some(index) = self.state.readers.iter().position(|reader| reader == &name) {
            self.state.selected = Some(index);
        }
        self.state.status = Status::WaitingForCard;
        self.state.result = None;
        self.state.push_log(format!("selected reader {name}"));
        let _ = self.cmd_tx.send(Cmd::Select(name));
        cx.notify();
    }

    fn on_event(&mut self, event: Event, cx: &mut Context<Self>) {
        match event {
            Event::ReadersChanged(readers) => {
                let count = readers.len();
                let previous = self.state.selected_name().map(str::to_string);
                self.state.set_readers(readers);
                let current = self.state.selected_name().map(str::to_string);
                self.state.push_log(format!(
                    "{count} reader{}",
                    if count == 1 { "" } else { "s" }
                ));
                // The worker follows the first reader until the user picks one.
                // If that name disappeared, point it at the new selection and
                // drop the previous card so the window does not keep stale fields.
                if previous.is_some() && previous != current {
                    self.state.result = None;
                    if !matches!(self.state.status, Status::NoReader) {
                        self.state.status = Status::WaitingForCard;
                    }
                    if let Some(name) = current.clone() {
                        let _ = self.cmd_tx.send(Cmd::Select(name));
                    }
                }
                self.select_dirty = true;
            }
            Event::ReaderState(flags) => {
                self.state.reader_flags = flags;
            }
            Event::CardUnsupported { reader, detail } => {
                self.state
                    .push_log(format!("card in {reader} detected but unsupported"));
                self.state.push_log(detail.clone());
                self.state.result = None;
                self.state.status = Status::Unsupported(detail);
            }
            Event::CardInserted(reader) => {
                self.state.status = Status::Reading;
                self.state.push_log(format!("card inserted in {reader}"));
            }
            Event::CardRemoved => {
                self.state.result = None;
                self.state.status = if self.state.readers.is_empty() {
                    Status::NoReader
                } else {
                    Status::WaitingForCard
                };
                self.state.push_log("card removed");
            }
            Event::ReadComplete(read) => {
                let apps = read.infos.len();
                for note in &read.notes {
                    self.state.push_log(note.clone());
                }
                self.state.push_log(format!(
                    "read {apps} app{}",
                    if apps == 1 { "" } else { "s" }
                ));
                self.state.result = Some(read);
                self.state.status = Status::Done;
            }
            Event::ReadError(message) => {
                self.state.push_log(message.clone());
                self.state.result = None;
                self.state.status = Status::Error(message);
            }
        }
        cx.notify();
    }

    fn sync_select(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let readers = self.state.readers.clone();
        let selected = self.state.selected.map(IndexPath::new);
        self.select.update(cx, |state, cx| {
            state.set_items(readers, window, cx);
            state.set_selected_index(selected, window, cx);
        });
    }

    fn copy_all(&mut self, cx: &mut App) {
        let Some(read) = &self.state.result else {
            return;
        };
        let mut text = String::new();
        if let Some(cred) = &read.credential {
            text.push_str("== credential ==\n");
            text.push_str(&format!("{:<22} {}\n", "scheme", cred.scheme));
            text.push_str(&format!("{:<22} {}\n", "pan", cred.pan));
            text.push_str(&format!(
                "{:<22} {}\n",
                "expiry",
                cred.expiry().unwrap_or_default()
            ));
            if let Some(m) = cred.expiry_month {
                text.push_str(&format!("{:<22} {m:02}\n", "month"));
            }
            if let Some(y) = cred.expiry_year {
                text.push_str(&format!("{:<22} {y:02}\n", "year"));
            }
            if let Some(n) = &cred.cardholder {
                text.push_str(&format!("{:<22} {n}\n", "cardholder"));
            }
            text.push('\n');
        }
        text.push_str(&plaintext(&read.infos));
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.state.push_log("copied all fields");
    }
}

impl Drop for CrApp {
    fn drop(&mut self) {
        let _ = self.cmd_tx.send(Cmd::Shutdown);
    }
}

impl Render for CrApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.select_dirty {
            self.select_dirty = false;
            self.sync_select(window, cx);
        }
        let weak = cx.entity().downgrade();
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .font_family(cx.theme().font_family.clone())
            .child(self.toolbar(weak.clone(), cx))
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_y_scrollbar()
                    .id("card-scroll")
                    .p_4()
                    .child(self.body(cx)),
            )
    }
}

impl CrApp {
    fn toolbar(&self, weak: WeakEntity<Self>, cx: &mut Context<Self>) -> impl IntoElement {
        let rescan_tx = self.cmd_tx.clone();
        let rescan_view = weak.clone();
        let copy_view = weak;
        let can_copy = self
            .state
            .result
            .as_ref()
            .is_some_and(|read| !read.infos.is_empty());

        div()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .px_4()
            .py_3()
            .border_b_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().title_bar)
            .child(div().font_weight(FontWeight::BOLD).child("crgui"))
            .child(
                div().flex_1().min_w(px(180.)).child(
                    Select::new(&self.select)
                        .placeholder("Select a reader")
                        .accessibility_label("PC/SC reader")
                        .disabled(self.state.readers.is_empty()),
                ),
            )
            .child(
                Button::new("rescan")
                    .label("Rescan")
                    .on_click(move |_, _, cx| {
                        let _ = rescan_tx.send(Cmd::Refresh);
                        rescan_view
                            .update(cx, |this, cx| {
                                this.state.push_log("rescanning readers");
                                cx.notify();
                            })
                            .ok();
                    }),
            )
            .child(status_tag(&self.state.status))
            .when(can_copy, |this| {
                this.child(Button::new("copy-all").ghost().label("Copy all").on_click(
                    move |_, _, cx| {
                        copy_view
                            .update(cx, |this, cx| {
                                this.copy_all(cx);
                                cx.notify();
                            })
                            .ok();
                    },
                ))
            })
    }

    fn body(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(self.center(cx))
            .child(self.log_section(cx))
    }

    fn center(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        match &self.state.status {
            Status::Error(message) => div()
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    Alert::error("read-error", SharedString::from(message.clone()))
                        .title("Read failed")
                        .banner(),
                )
                .into_any_element(),
            Status::NoReader => empty_state("No reader found — is pcscd running?", cx),
            Status::WaitingForCard => {
                empty_state_with_flags("Insert a card", self.state.reader_flags.as_ref(), cx)
            }
            Status::Reading => empty_state("Reading…", cx),
            Status::Unsupported(detail) => div()
                .flex()
                .flex_col()
                .gap_3()
                .child(
                    Alert::warning("unsupported-card", SharedString::from(detail.clone()))
                        .title("Card detected, not ISO 7816")
                        .banner(),
                )
                .into_any_element(),
            Status::Done => self.card_panel(cx),
        }
    }

    fn card_panel(&self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let Some(read) = self.state.result.clone() else {
            return empty_state("Insert a card", cx);
        };
        if read.infos.is_empty() && read.credential.is_none() {
            let note = read
                .notes
                .first()
                .cloned()
                .unwrap_or_else(|| "No built-in app recognized this card.".into());
            return empty_state(&note, cx);
        }

        let mut rows = Vec::new();

        if let Some(cred) = &read.credential {
            rows.push(section_heading("credential", cx));
            let fields: Vec<(&str, String)> = vec![
                ("scheme", cred.scheme.clone()),
                ("pan", cred.pan.clone()),
                ("expiry", cred.expiry().unwrap_or_default()),
                (
                    "month",
                    cred.expiry_month.map(|m| format!("{m:02}")).unwrap_or_default(),
                ),
                (
                    "year",
                    cred.expiry_year.map(|y| format!("{y:02}")).unwrap_or_default(),
                ),
                ("cardholder", cred.cardholder.clone().unwrap_or_default()),
            ];
            for (key, value) in fields {
                rows.push(field_row(
                    format!("cred-{key}"),
                    key.to_string(),
                    value,
                    cx,
                ));
            }
        }

        for info in &read.infos {
            rows.push(
                div()
                    .pt_3()
                    .pb_1()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(info.app.clone())
                    .into_any_element(),
            );
            for (index, field) in info.fields.iter().enumerate() {
                rows.push(field_row(
                    format!("field-{}-{}-{index}", info.app, field.key),
                    field.key.clone(),
                    field.value.clone(),
                    cx,
                ));
            }
        }

        let has_dump = read.infos.iter().any(|info| !info.raw.is_empty());
        let dump = if has_dump {
            Some(self.dump_section(&read, cx))
        } else {
            None
        };

        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("Reader  {}", read.reader)),
            )
            .children(rows)
            .children(dump)
            .into_any_element()
    }

    fn dump_section(&self, read: &CardRead, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        let weak = cx.entity().downgrade();
        let open = self.dump_open;
        let label = if open {
            "Hide raw dump"
        } else {
            "Raw APDU dump"
        };

        let mut lines = Vec::new();
        for info in &read.infos {
            for (index, line) in info.raw.iter().enumerate() {
                lines.push(field_row(
                    format!("dump-{}-{}-{index}", info.app, line.label),
                    format!("{}/{}", info.app, line.label),
                    line.value.clone(),
                    cx,
                ));
            }
        }

        div()
            .pt_3()
            .child(
                Collapsible::new()
                    .open(open)
                    .child(Button::new("toggle-dump").ghost().label(label).on_click(
                        move |_: &ClickEvent, _, cx| {
                            weak.update(cx, |this, cx| {
                                this.dump_open = !this.dump_open;
                                cx.notify();
                            })
                            .ok();
                        },
                    ))
                    .content(div().flex().flex_col().gap_1().children(lines)),
            )
            .into_any_element()
    }

    fn log_section(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let weak = cx.entity().downgrade();
        let open = self.log_open;
        let label = format!("Log ({})", self.state.log.len());
        let lines: Vec<_> = self
            .state
            .log
            .iter()
            .rev()
            .take(40)
            .map(|line| {
                div()
                    .text_xs()
                    .font_family(cx.theme().mono_font_family.clone())
                    .text_color(cx.theme().muted_foreground)
                    .child(line.clone())
                    .into_any_element()
            })
            .collect();

        Collapsible::new()
            .open(open)
            .child(Button::new("toggle-log").ghost().label(label).on_click(
                move |_: &ClickEvent, _, cx| {
                    weak.update(cx, |this, cx| {
                        this.log_open = !this.log_open;
                        cx.notify();
                    })
                    .ok();
                },
            ))
            .content(div().flex().flex_col().gap_1().pt_1().children(lines))
    }
}

fn section_heading(label: &str, _cx: &App) -> gpui_kit::AnyElement {
    div()
        .pt_3()
        .pb_1()
        .text_sm()
        .font_weight(FontWeight::SEMIBOLD)
        .child(label.to_string())
        .into_any_element()
}

fn field_row(id: String, label: String, value: String, cx: &App) -> gpui_kit::AnyElement {
    let copy_id = format!("copy-{id}");
    let copy_value = value.clone();
    ListItem::new(SharedString::from(id))
        .accessibility_label(format!("{label} {value}"))
        .child(
            div()
                .w(px(220.))
                .flex_shrink_0()
                .text_color(cx.theme().muted_foreground)
                .child(label),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .font_family(cx.theme().mono_font_family.clone())
                .text_size(cx.theme().mono_font_size)
                .child(value),
        )
        .suffix(move |_, _| {
            Clipboard::new(SharedString::from(copy_id.clone()))
                .value(copy_value.clone())
                .tooltip("Copy")
        })
        .into_any_element()
}

fn status_tag(status: &Status) -> impl IntoElement {
    // `Tag` is the kit's status pill. `Badge` only draws a count overlay.
    let tag = match status {
        Status::NoReader | Status::Error(_) => Tag::danger(),
        Status::WaitingForCard | Status::Unsupported(_) => Tag::warning(),
        Status::Reading => Tag::info(),
        Status::Done => Tag::success(),
    };
    tag.with_size(Size::Small)
        .child(SharedString::from(status.label()))
}

fn flags_summary(status: Option<&ReaderSnapshot>) -> String {
    let Some(status) = status else {
        return String::new();
    };
    let flags = status.flags;
    let mut parts = Vec::new();
    if flags.present {
        parts.push("PRESENT");
    }
    if flags.mute {
        parts.push("MUTE");
    }
    if flags.empty {
        parts.push("EMPTY");
    }
    if flags.unavailable {
        parts.push("UNAVAILABLE");
    }
    if flags.in_use {
        parts.push("INUSE");
    }
    if flags.unpowered {
        parts.push("UNPOWERED");
    }
    parts.join(" | ")
}

fn empty_state_with_flags(
    message: &str,
    status: Option<&ReaderSnapshot>,
    cx: &App,
) -> gpui_kit::AnyElement {
    let summary = flags_summary(status);
    let mut container = div()
        .py(px(64.))
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_2()
        .text_color(cx.theme().muted_foreground)
        .child(message.to_string());
    if !summary.is_empty() {
        container = container.child(div().text_xs().child(format!("reader state: {summary}")));
    }
    container.into_any_element()
}

fn empty_state(message: &str, cx: &App) -> gpui_kit::AnyElement {
    div()
        .py(px(64.))
        .flex()
        .justify_center()
        .text_color(cx.theme().muted_foreground)
        .child(message.to_string())
        .into_any_element()
}
