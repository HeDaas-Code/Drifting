//! Jump and date editing share the selected calendar's continuous coordinate.
use super::*;
use dr_i18n::keys::{calendar_editor as copy, common};
use dr_model::calendar::{CalendarSpec, Calibration};
use dr_shell::dialogs;
use dr_ui::components::{Button, Dialog};

#[derive(Clone)]
pub(super) enum Target {
    Jump,
    Entry { id: String, at: f64 },
    Pin { id: String, at: f64 },
}

pub(super) fn open(
    timeline: &mut Timeline,
    target: Target,
    window: &mut Window,
    cx: &mut Context<Timeline>,
) {
    let Some((spec, mapping)) = timeline.calendar() else {
        return;
    };
    let spec = spec.clone();
    let view = timeline.view().expect("calendar view").id.clone();
    let book = timeline.book.id.clone();
    let weak = cx.weak_entity();
    timeline.popup = None;
    let panel = cx.new(|cx| DateDialog::new(weak, book, view, spec, mapping, target, window, cx));
    dialogs::show_in(window.window_handle(), cx, panel.into());
}

struct DateDialog {
    timeline: WeakEntity<Timeline>,
    book: String,
    view: String,
    calendar: CalendarSpec,
    mapping: Calibration,
    target: Target,
    input: Entity<TextField>,
    failed: bool,
    _sub: Subscription,
}

impl DateDialog {
    #[allow(clippy::too_many_arguments)]
    fn new(
        timeline: WeakEntity<Timeline>,
        book: String,
        view: String,
        calendar: CalendarSpec,
        mapping: Calibration,
        target: Target,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let at = match &target {
            Target::Jump => mapping.coordinate,
            Target::Entry { at, .. } | Target::Pin { at, .. } => *at,
        };
        let initial = mapping
            .to_day(at)
            .ok()
            .and_then(|day| calendar.format_day(day).ok())
            .unwrap_or_default();
        let input =
            cx.new(|cx| TextField::new("timeline-date-input", window, cx).with_text(initial));
        input.focus_handle(cx).focus(window, cx);
        let sub = cx.subscribe_in(&input, window, |dialog, _, event, _, cx| match event {
            FieldEvent::Submitted(_) => dialog.submit(cx),
            FieldEvent::Cancelled => dialog.close(cx),
            _ => cx.notify(),
        });
        Self {
            timeline,
            book,
            view,
            calendar,
            mapping,
            target,
            input,
            failed: false,
            _sub: sub,
        }
    }

    fn day(&self, cx: &App) -> Option<i64> {
        self.calendar
            .parse_date(&self.input.read(cx).text())
            .ok()
            .and_then(|date| self.calendar.to_day(date).ok())
    }

    fn close(&self, cx: &mut Context<Self>) {
        let id = cx.entity_id();
        dialogs::hide(cx, id);
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        let Some(at) = self
            .day(cx)
            .and_then(|day| self.mapping.to_coordinate(day).ok())
        else {
            self.failed = true;
            cx.notify();
            return;
        };
        let target = self.target.clone();
        let valid = self
            .timeline
            .update(cx, |timeline, cx| {
                if timeline.book.id != self.book
                    || timeline.view().is_none_or(|view| view.id != self.view)
                    || timeline.calendar().is_none_or(|(calendar, mapping)| {
                        calendar != &self.calendar || mapping != self.mapping
                    })
                {
                    return false;
                }
                match target {
                    Target::Jump => timeline.jump_to_coordinate(at, cx),
                    Target::Entry {
                        id,
                        at: expected_at,
                        ..
                    } => {
                        let Some(placement) = timeline.view().and_then(|view| {
                            view.placements
                                .iter()
                                .find(|placement| placement.entry == id)
                        }) else {
                            return false;
                        };
                        if placement.at != expected_at {
                            return false;
                        }
                        let span = placement.span;
                        timeline.place(id, None, at, span, cx);
                    }
                    Target::Pin {
                        id,
                        at: expected_at,
                    } => {
                        if timeline
                            .view()
                            .and_then(|view| view.pins.iter().find(|pin| pin.id == id))
                            .is_none_or(|pin| pin.at != expected_at)
                        {
                            return false;
                        }
                        timeline.edit_pin(
                            id,
                            PinEdit::DateAt {
                                at,
                                expected: expected_at,
                                rules: Box::new(self.calendar.clone()),
                                mapping: self.mapping,
                            },
                            cx,
                        );
                    }
                }
                true
            })
            .unwrap_or(false);
        if valid {
            self.close(cx);
        } else {
            self.failed = true;
            cx.notify();
        }
    }
}

impl Render for DateDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let title = t(
            cx,
            if matches!(self.target, Target::Jump) {
                copy::JUMP_DATE
            } else {
                copy::SET_DATE
            },
        );
        let me = cx.entity();
        let close = me.clone();
        let details = self
            .day(cx)
            .map(|day| {
                let mut parts = vec![self.calendar.weekday(day).unwrap_or_default().to_owned()];
                for cycle in [&self.calendar.seasons, &self.calendar.moon]
                    .into_iter()
                    .flatten()
                {
                    if let Ok((phase, elapsed)) = cycle.position(day) {
                        parts.push(format!("{phase} +{elapsed}/{}", cycle.denominator));
                    }
                }
                parts.join(" · ")
            })
            .unwrap_or_default();
        Dialog::new("timeline-date-dialog", title.clone())
            .body(self.input.clone())
            .body(div().text_sm().child(t(cx, copy::DATE_HINT)))
            .body(
                div()
                    .child(details)
                    .when(self.failed, |el| el.child(t(cx, copy::INVALID))),
            )
            .action(
                Button::new("timeline-date-apply", title)
                    .primary()
                    .on_click(move |_, _, cx| me.update(cx, |dialog, cx| dialog.submit(cx))),
            )
            .action(
                Button::new("timeline-date-cancel", t(cx, common::CANCEL))
                    .on_click(move |_, _, cx| close.update(cx, |dialog, cx| dialog.close(cx))),
            )
    }
}

impl Timeline {
    pub(super) fn calendar(&self) -> Option<(&CalendarSpec, Calibration)> {
        let view = self.view()?;
        let id = view.calendar_id.as_ref()?;
        let calendar = self
            .book
            .v3()?
            .project
            .calendars
            .iter()
            .find(|calendar| &calendar.id == id)?;
        Some((calendar, view.calendar_calibration?))
    }

    pub(super) fn axis_extent(&self, view: &TimelineView) -> (f64, f64) {
        let (min, max) = model::extent(view);
        self.jump_coordinate
            .map_or((min, max), |at| (at - 20., at + 180.))
    }

    pub(super) fn jump_to_coordinate(&mut self, at: f64, cx: &mut Context<Self>) {
        if !at.is_finite() {
            return;
        }
        self.jump_coordinate = Some(at);
        if let Some(view) = self.view() {
            let min = self.axis_extent(view).0;
            let offset = ((at - min) as f32 * self.scale().unit() - 40.).clamp(0., 9_000_000.);
            self.scroll.set_offset(point(px(-offset), px(0.)));
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;
    use dr_workspace::Actor;
    use serde_json::json;
    use std::time::{Duration, Instant};

    #[gpui::test]
    fn date_input_jumps_and_moves_pin_and_entry_then_rejects_another_book(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let temp = tempfile::tempdir().unwrap();
        let store =
            dr_store::Library::open(temp.path(), dr_workspace::library_options("test")).unwrap();
        let seed = dr_testkit::seed::generate_v3(
            dr_testkit::seed::Preset::Tiny,
            dr_testkit::seed::DEFAULT_SEED,
        )
        .unwrap();
        let book = seed.books[0].book_id().to_string();
        seed.write_to(&store).unwrap();
        store.close().unwrap();
        let lib = Library::open_dir(temp.path(), "test").unwrap();
        let read = || {
            lib.call(&Actor::User, "timeline_view.list", json!({"book":book}))
                .unwrap()["output"][0]
                .clone()
        };
        let view = read();
        let view_id = view["id"].as_str().unwrap().to_owned();
        let entry = view["placements"][0]["entry"].as_str().unwrap().to_owned();
        let spec = CalendarSpec::simple("synthetic".into(), "Synthetic".into());
        let mapping = Calibration {
            coordinate: 0.,
            day: 0,
            days_per_unit: 2.,
        };
        lib.call(
            &Actor::User,
            "calendar.put",
            json!({"book":book,"calendar":spec}),
        )
        .unwrap();
        lib.call(
            &Actor::User,
            "timeline_view.calendar",
            json!({"book":book,"view":view_id,"calendar":"synthetic","calibration":mapping}),
        )
        .unwrap();
        lib.call(
            &Actor::User,
            "timeline_view.put_pin",
            json!({"book":book,"view":view_id,"pin":{"id":"pin","label":"Synthetic pin","at":3}}),
        )
        .unwrap();
        cx.update(|cx| {
            dr_shell::init(
                dr_shell::ShellInfo {
                    app_name: "Drifting Next",
                    bundle_id: "cc.drifting.next.dev",
                    version: "test",
                    url_scheme: "drifting-next-dev",
                    forced_appearance: None,
                },
                dr_shell::Services {
                    library: Some(lib.clone()),
                    book: Some(book.clone()),
                    ..Default::default()
                },
                dr_shell::SettingsFile::in_memory(),
                cx,
            )
        });
        let (ws, cx) = cx.add_window_view(Workspace::new);
        let host = BottomHost {
            workspace: ws.downgrade(),
            library: Some(lib.clone()),
            book_id: book.clone(),
            book: Rc::new(Book::load(&lib, &book).unwrap()),
            target: None,
        };
        let timeline = cx.update(|_, cx| cx.new(|_| Timeline::new(&host)));
        let dialog = cx.update(|window, cx| {
            cx.new(|cx| {
                DateDialog::new(
                    timeline.downgrade(),
                    book.clone(),
                    view_id.clone(),
                    spec.clone(),
                    mapping,
                    Target::Jump,
                    window,
                    cx,
                )
            })
        });
        dialog.update(cx, |dialog, cx| {
            dialog
                .input
                .update(cx, |field, cx| field.set_text("10000/1/1", cx));
            dialog.submit(cx);
            assert!(!dialog.failed);
        });
        let far = mapping
            .to_coordinate(spec.to_day(spec.parse_date("10000/1/1").unwrap()).unwrap())
            .unwrap();
        assert_eq!(timeline.read_with(cx, |t, _| t.jump_coordinate), Some(far));
        let extent = timeline.read_with(cx, |t, _| t.axis_extent(t.view().unwrap()));
        assert!(extent.0 <= far && extent.1 >= far && extent.1 - extent.0 < 1000.);

        for target in [
            Target::Pin {
                id: "pin".into(),
                at: 3.,
            },
            Target::Entry {
                id: entry.clone(),
                at: view["placements"][0]["at"].as_f64().unwrap(),
            },
        ] {
            let is_pin = matches!(target, Target::Pin { .. });
            dialog.update(cx, |dialog, cx| {
                dialog.target = target;
                dialog
                    .input
                    .update(cx, |field, cx| field.set_text("-2/1/1", cx));
                dialog.submit(cx);
                assert!(!dialog.failed);
            });
            let expected = mapping
                .to_coordinate(spec.to_day(spec.parse_date("-2/1/1").unwrap()).unwrap())
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                cx.run_until_parked();
                let data = read();
                let row = if is_pin {
                    data["pins"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|pin| pin["id"] == "pin")
                        .unwrap()
                } else {
                    data["placements"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|p| p["entry"] == entry)
                        .unwrap()
                };
                if row["at"].as_f64() == Some(expected) {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "date input write did not reach backend (pin={is_pin})"
                );
                std::thread::sleep(Duration::from_millis(3));
            }
            timeline.update(cx, |timeline, _| {
                timeline.book = Rc::new(Book::load(&lib, &book).unwrap())
            });
        }
        let version = lib.observe(&format!("book:{book}")).unwrap().version;
        timeline.update(cx, |timeline, _| timeline.book = Rc::new(Book::default()));
        dialog.update(cx, |dialog, cx| {
            dialog.target = Target::Jump;
            dialog.submit(cx);
            assert!(dialog.failed);
        });
        assert_eq!(
            lib.observe(&format!("book:{book}")).unwrap().version,
            version
        );
        lib.close();
    }
}
