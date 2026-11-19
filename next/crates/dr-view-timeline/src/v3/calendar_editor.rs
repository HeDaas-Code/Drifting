//! Calendar management in the timeline; edits never rewrite event coordinates.
use super::calendar_form::{CycleForm, EraForm, Field, Form, LeapForm, MonthForm, PartForm};
use super::*;
use dr_i18n::keys::{calendar_editor as copy, common};
use dr_model::calendar::{
    CalendarSpec, Calibration, CyclePart, Intercalation, LeapAdjustment, NamedCycle, Periodic,
};
use dr_shell::{dialogs, release};
use dr_ui::components::{Button, Dialog, Switch};
use dr_workspace::{Actor, WeakWorkspace, WsError, commands::v3::calendar};

pub(super) fn open(timeline: &Timeline, window: &mut Window, cx: &mut App) {
    let (Some(book), Some(view), Some(lib)) =
        (timeline.book.v3(), timeline.view(), timeline.lib.as_ref())
    else {
        return;
    };
    let panel = cx.new(|cx| {
        Editor::new(
            lib.downgrade(),
            timeline.book.id.clone(),
            view.clone(),
            book.project.calendars.clone(),
            window,
            cx,
        )
    });
    dialogs::show_in(window.window_handle(), cx, panel.into());
}

use futures::{FutureExt as _, channel::oneshot};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Jobs(Arc<Mutex<Active>>);
#[derive(Default)]
struct Active {
    jobs: usize,
    waiters: Vec<oneshot::Sender<()>>,
}
impl Global for Jobs {}

struct Lease {
    jobs: Arc<Mutex<Active>>,
    library: Option<Library>,
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.library.take();
        let mut active = self.jobs.lock().unwrap_or_else(|error| error.into_inner());
        active.jobs -= 1;
        if active.jobs == 0 {
            for waiter in active.waiters.drain(..) {
                let _ = waiter.send(());
            }
        }
    }
}
fn admit(lib: Library, cx: &mut App) -> Lease {
    if !cx.has_global::<Jobs>() {
        cx.set_global(Jobs::default());
        release::before_release(cx, |cx| {
            let jobs = cx.global::<Jobs>().0.clone();
            let mut active = jobs.lock().unwrap_or_else(|error| error.into_inner());
            if active.jobs == 0 {
                return async { Ok(()) }.boxed_local();
            }
            let (tx, rx) = oneshot::channel();
            active.waiters.push(tx);
            async move { rx.await.map_err(|_| "calendar save was interrupted".into()) }
                .boxed_local()
        });
    }
    let jobs = cx.global::<Jobs>().0.clone();
    jobs.lock().unwrap_or_else(|error| error.into_inner()).jobs += 1;
    Lease {
        jobs,
        library: Some(lib),
    }
}

enum Job {
    Save {
        expected: Option<Box<CalendarSpec>>,
        value: Box<CalendarSpec>,
    },
    Delete(Box<CalendarSpec>),
    Select {
        expected: calendar::Selection,
        value: calendar::Selection,
    },
}

impl Job {
    fn run(self, lib: &Library, book: &str, view: &str) -> Result<Vec<CalendarSpec>, WsError> {
        match self {
            Job::Save { expected, value } => {
                lib.run::<calendar::CalendarRestore>(
                    &Actor::User,
                    calendar::RestoreIn {
                        book: book.into(),
                        id: value.id.clone(),
                        expected: expected.map(|value| *value),
                        value: Some(*value),
                    },
                )?;
            }
            Job::Delete(value) => {
                lib.run::<calendar::CalendarRestore>(
                    &Actor::User,
                    calendar::RestoreIn {
                        book: book.into(),
                        id: value.id.clone(),
                        expected: Some(*value),
                        value: None,
                    },
                )?;
            }
            Job::Select { expected, value } => {
                lib.run::<calendar::ViewCalendarRestore>(
                    &Actor::User,
                    calendar::RestoreSelectionIn {
                        book: book.into(),
                        view: view.into(),
                        expected,
                        value,
                    },
                )?;
            }
        }
        Ok(lib
            .run::<calendar::CalendarList>(
                &Actor::User,
                dr_workspace::commands::BookIn { book: book.into() },
            )?
            .output)
    }
}

pub(super) struct Editor {
    lib: WeakWorkspace,
    book: String,
    view: String,
    calendars: Vec<CalendarSpec>,
    base: Option<CalendarSpec>,
    form: Option<Form>,
    selection: calendar::Selection,
    anchor_coordinate: Field,
    anchor_date: Field,
    day_ratio: Field,
    fields: Vec<Subscription>,
    next_field: u64,
    busy: bool,
    failed: bool,
    confirm_delete: bool,
}

impl Editor {
    fn new(
        lib: WeakWorkspace,
        book: String,
        view: TimelineView,
        calendars: Vec<CalendarSpec>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut empty = |id| cx.new(|cx| TextField::new(id, window, cx));
        let mut editor = Self {
            lib,
            book,
            view: view.id,
            calendars,
            base: None,
            form: None,
            selection: calendar::Selection {
                calendar: view.calendar_id,
                calibration: view.calendar_calibration,
            },
            anchor_coordinate: empty("calendar-anchor-coordinate"),
            anchor_date: empty("calendar-anchor-date"),
            day_ratio: empty("calendar-days-per-unit"),
            fields: Vec::new(),
            next_field: 0,
            busy: false,
            failed: false,
            confirm_delete: false,
        };
        let chosen = editor
            .calendars
            .iter()
            .find(|c| Some(&c.id) == editor.selection.calendar.as_ref())
            .or_else(|| editor.calendars.first())
            .cloned();
        if let Some(spec) = chosen {
            editor.edit(spec, window, cx);
        }
        editor
    }

    fn field(&mut self, value: String, window: &mut Window, cx: &mut Context<Self>) -> Field {
        self.next_field += 1;
        let id: SharedString = format!("calendar-field-{}", self.next_field).into();
        let field = cx.new(|cx| {
            let mut field = TextField::new(id, window, cx);
            field.set_text(value, cx);
            field
        });
        self.fields
            .push(cx.subscribe(&field, |_, _, _, cx| cx.notify()));
        field
    }

    fn edit(&mut self, spec: CalendarSpec, window: &mut Window, cx: &mut Context<Self>) {
        self.fields.clear();
        self.form = Some(Form::new(&spec, &mut |value| self.field(value, window, cx)));
        self.base = Some(spec.clone());
        self.confirm_delete = false;
        if let Some(mapping) = self.selection.calibration {
            self.anchor_coordinate
                .update(cx, |f, cx| f.set_text(mapping.coordinate.to_string(), cx));
            self.day_ratio.update(cx, |f, cx| {
                f.set_text(mapping.days_per_unit.to_string(), cx)
            });
            self.anchor_date.update(cx, |f, cx| {
                f.set_text(spec.format_day(mapping.day).unwrap_or_default(), cx)
            });
        }
        cx.notify();
    }

    fn new_calendar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let spec = CalendarSpec::simple(
            dr_model::id::new_uuid_v7(),
            t(cx, copy::NEW_CALENDAR).to_string(),
        );
        self.edit(spec, window, cx);
        self.base = None;
    }

    fn dirty(&self, cx: &App) -> bool {
        self.form
            .as_ref()
            .is_some_and(|form| form.value(cx).ok().as_ref() != self.base.as_ref())
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(value) = self.form.as_ref().and_then(|form| form.value(cx).ok()) else {
            self.failed = true;
            cx.notify();
            return;
        };
        self.run(
            Job::Save {
                expected: self.base.clone().map(Box::new),
                value: Box::new(value),
            },
            window,
            cx,
        );
    }

    fn use_calendar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(spec) = self.base.as_ref() else {
            return;
        };
        let mapping = if let Some(mapping) = self.selection.calibration {
            mapping
        } else {
            let fields = (&self.anchor_coordinate, &self.anchor_date, &self.day_ratio);
            let mapping = (|| {
                Ok::<_, String>(Calibration {
                    coordinate: fields
                        .0
                        .read(cx)
                        .text()
                        .trim()
                        .parse()
                        .map_err(|_| "invalid anchor")?,
                    day: spec
                        .to_day(
                            spec.parse_date(&fields.1.read(cx).text())
                                .map_err(str::to_owned)?,
                        )
                        .map_err(str::to_owned)?,
                    days_per_unit: fields
                        .2
                        .read(cx)
                        .text()
                        .trim()
                        .parse()
                        .map_err(|_| "invalid day length")?,
                })
            })();
            let Ok(mapping) = mapping else {
                self.failed = true;
                cx.notify();
                return;
            };
            if mapping.validate().is_err() {
                self.failed = true;
                cx.notify();
                return;
            }
            mapping
        };
        self.run(
            Job::Select {
                expected: self.selection.clone(),
                value: calendar::Selection {
                    calendar: Some(spec.id.clone()),
                    calibration: Some(mapping),
                },
            },
            window,
            cx,
        );
    }

    fn run(&mut self, job: Job, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || release::is_releasing(cx) {
            return;
        }
        let Some(lib) = self.lib.upgrade() else {
            return;
        };
        let saved = match &job {
            Job::Save { value, .. } => Some((**value).clone()),
            _ => None,
        };
        let deleted = matches!(&job, Job::Delete(_));
        let selection = match &job {
            Job::Select { value, .. } => Some(value.clone()),
            _ => None,
        };
        self.busy = true;
        self.failed = false;
        let (book, view) = (self.book.clone(), self.view.clone());
        let lease = admit(lib, cx);
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    job.run(
                        lease.library.as_ref().expect("active calendar write"),
                        &book,
                        &view,
                    )
                })
                .await;
            this.update_in(cx, |editor, window, cx| {
                editor.busy = false;
                match result {
                    Ok(calendars) => {
                        editor.calendars = calendars;
                        if let Some(saved) = saved {
                            editor.base = Some(saved);
                        }
                        if let Some(selection) = selection {
                            editor.selection = selection;
                        }
                        if deleted {
                            editor.base = None;
                            editor.form = None;
                            editor.confirm_delete = false;
                            if let Some(spec) = editor.calendars.first().cloned() {
                                editor.edit(spec, window, cx);
                            }
                        }
                    }
                    Err(error) => {
                        log::warn!("calendar edit: {error}");
                        editor.failed = true;
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        if !self.busy {
            let id = cx.entity_id();
            dialogs::hide(cx, id);
        }
    }

    fn add_month(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let row = MonthForm::new(String::new(), 30, &mut |value| {
            self.field(value, window, cx)
        });
        if let Some(form) = &mut self.form {
            form.months.push(row);
        }
        cx.notify();
    }
    fn add_era(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let row = EraForm::new(String::new(), 0, &mut |value| self.field(value, window, cx));
        if let Some(form) = &mut self.form {
            form.eras.push(row);
        }
        cx.notify();
    }
    fn add_leap(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let row = LeapForm::new(
            &Intercalation {
                when: Periodic {
                    every_years: 4,
                    offset_year: 0,
                    except_every: None,
                    restore_every: None,
                },
                adjustment: LeapAdjustment::Days { month: 0, days: 1 },
            },
            &mut |value| self.field(value, window, cx),
        );
        if let Some(form) = &mut self.form {
            form.leaps.push(row);
        }
        cx.notify();
    }
    fn add_weekday(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let row = self.field(String::new(), window, cx);
        if let Some(form) = &mut self.form {
            form.week.push(row);
        }
        cx.notify();
    }
    fn toggle_cycle(&mut self, moon: bool, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        let value = on.then(|| {
            CycleForm::new(
                &NamedCycle {
                    length: 100,
                    denominator: 1,
                    offset: 0,
                    parts: (0..4)
                        .map(|index| CyclePart {
                            name: (index + 1).to_string(),
                            starts_at: index * 25,
                        })
                        .collect(),
                },
                &mut |value| self.field(value, window, cx),
            )
        });
        if let Some(form) = &mut self.form {
            if moon {
                form.moon = value;
            } else {
                form.seasons = value;
            }
        }
        cx.notify();
    }
    fn add_phase(&mut self, moon: bool, window: &mut Window, cx: &mut Context<Self>) {
        let part = PartForm {
            name: self.field(String::new(), window, cx),
            start: self.field(String::new(), window, cx),
        };
        if let Some(form) = &mut self.form
            && let Some(cycle) = if moon {
                &mut form.moon
            } else {
                &mut form.seasons
            }
        {
            cycle.parts.push(part);
        }
        cx.notify();
    }

    fn cycle(&self, moon: bool, cx: &mut Context<Self>) -> AnyElement {
        let entity = cx.entity();
        let form = self.form.as_ref().expect("calendar form");
        let cycle = if moon {
            form.moon.as_ref()
        } else {
            form.seasons.as_ref()
        };
        let prefix = if moon { "moon" } else { "seasons" };
        let mut body = div().flex().flex_col().gap_2().child(
            div()
                .flex()
                .justify_between()
                .child(t(cx, if moon { copy::MOON } else { copy::SEASONS }))
                .child(
                    Switch::new(("calendar-cycle", moon as usize), cycle.is_some())
                        .disabled(self.busy)
                        .on_change(move |on, window, cx| {
                            entity
                                .update(cx, |editor, cx| editor.toggle_cycle(moon, on, window, cx))
                        }),
                ),
        );
        if let Some(cycle) = cycle {
            body = body.child(
                div()
                    .flex()
                    .gap_2()
                    .child(labeled(t(cx, copy::CYCLE_LENGTH), &cycle.length, 120.))
                    .child(labeled(
                        t(cx, copy::UNITS_PER_DAY),
                        &cycle.denominator,
                        120.,
                    ))
                    .child(labeled(t(cx, copy::OFFSET), &cycle.offset, 120.)),
            );
            for (index, phase) in cycle.parts.iter().enumerate() {
                body = body.child(
                    div()
                        .flex()
                        .items_end()
                        .gap_2()
                        .child(labeled(t(cx, copy::NAME), &phase.name, 180.))
                        .child(labeled(t(cx, copy::STARTS_AT), &phase.start, 120.))
                        .child(
                            Button::new(
                                SharedString::from(format!("{prefix}-remove-phase-{index}")),
                                t(cx, common::DELETE),
                            )
                            .small()
                            .disabled(self.busy)
                            .on_click(cx.listener(
                                move |editor, _, _, cx| {
                                    if let Some(form) = &mut editor.form
                                        && let Some(cycle) = if moon {
                                            &mut form.moon
                                        } else {
                                            &mut form.seasons
                                        }
                                    {
                                        cycle.parts.remove(index);
                                    }
                                    cx.notify();
                                },
                            )),
                        ),
                );
            }
            body =
                body.child(
                    Button::new(("calendar-add-phase", moon as usize), t(cx, copy::ADD))
                        .disabled(self.busy)
                        .on_click(cx.listener(move |editor, _, window, cx| {
                            editor.add_phase(moon, window, cx)
                        })),
                );
        }
        body.into_any_element()
    }

    fn form(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(form) = &self.form else {
            return div().into_any_element();
        };
        let mut body = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(labeled(t(cx, copy::NAME), &form.name, 360.))
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(labeled(t(cx, copy::EPOCH_DATE), &form.epoch, 260.))
                    .child(labeled(t(cx, copy::EPOCH_DAY), &form.epoch_day, 160.)),
            )
            .child(div().text_sm().child(t(cx, copy::DATE_HINT)))
            .child(t(cx, copy::MONTHS));
        for (index, month) in form.months.iter().enumerate() {
            body = body.child(
                div()
                    .flex()
                    .items_end()
                    .gap_2()
                    .child(labeled(t(cx, copy::NAME), &month.name, 220.))
                    .child(labeled(t(cx, copy::DAYS), &month.days, 100.))
                    .child(
                        Button::new(("calendar-remove-month", index), t(cx, common::DELETE))
                            .small()
                            .disabled(self.busy)
                            .on_click(cx.listener(move |editor, _, _, cx| {
                                if let Some(form) = &mut editor.form {
                                    form.months.remove(index);
                                }
                                cx.notify();
                            })),
                    ),
            );
        }
        body = body
            .child(
                Button::new("calendar-add-month", t(cx, copy::ADD))
                    .disabled(self.busy)
                    .on_click(cx.listener(|editor, _, window, cx| editor.add_month(window, cx))),
            )
            .child(t(cx, copy::LEAP_RULES));
        for (index, leap) in form.leaps.iter().enumerate() {
            let row = div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_end()
                        .gap_2()
                        .child(labeled(t(cx, copy::EVERY_YEARS), &leap.every, 100.))
                        .child(labeled(t(cx, copy::OFFSET), &leap.offset, 100.))
                        .child(labeled(t(cx, copy::EXCEPT_EVERY), &leap.except, 110.))
                        .child(labeled(t(cx, copy::RESTORE_EVERY), &leap.restore, 110.)),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_end()
                        .gap_2()
                        .child(
                            Button::new(
                                ("calendar-leap-type", index),
                                t(
                                    cx,
                                    if leap.month {
                                        copy::LEAP_MONTH
                                    } else {
                                        copy::LEAP_DAY
                                    },
                                ),
                            )
                            .small()
                            .disabled(self.busy)
                            .on_click(cx.listener(
                                move |editor, _, _, cx| {
                                    if let Some(form) = &mut editor.form {
                                        form.leaps[index].month = !form.leaps[index].month;
                                    }
                                    cx.notify();
                                },
                            )),
                        )
                        .child(labeled(t(cx, copy::BASE_MONTH), &leap.target, 110.))
                        .child(labeled(t(cx, copy::DAYS), &leap.days, 100.))
                        .when(leap.month, |row| {
                            row.child(labeled(t(cx, copy::NAME), &leap.name, 160.))
                        })
                        .child(
                            Button::new(("calendar-remove-leap", index), t(cx, common::DELETE))
                                .small()
                                .disabled(self.busy)
                                .on_click(cx.listener(move |editor, _, _, cx| {
                                    if let Some(form) = &mut editor.form {
                                        form.leaps.remove(index);
                                    }
                                    cx.notify();
                                })),
                        ),
                );
            body = body.child(row);
        }
        body = body
            .child(
                Button::new("calendar-add-leap", t(cx, copy::ADD))
                    .disabled(self.busy)
                    .on_click(cx.listener(|editor, _, window, cx| editor.add_leap(window, cx))),
            )
            .child(t(cx, copy::ERAS));
        for (index, era) in form.eras.iter().enumerate() {
            body = body.child(
                div()
                    .flex()
                    .items_end()
                    .gap_2()
                    .child(labeled(t(cx, copy::NAME), &era.name, 240.))
                    .child(labeled(t(cx, copy::START_YEAR), &era.year, 140.))
                    .child(
                        Button::new(("calendar-remove-era", index), t(cx, common::DELETE))
                            .small()
                            .disabled(self.busy)
                            .on_click(cx.listener(move |editor, _, _, cx| {
                                if let Some(form) = &mut editor.form {
                                    form.eras.remove(index);
                                }
                                cx.notify();
                            })),
                    ),
            );
        }
        body = body
            .child(
                Button::new("calendar-add-era", t(cx, copy::ADD))
                    .disabled(self.busy)
                    .on_click(cx.listener(|editor, _, window, cx| editor.add_era(window, cx))),
            )
            .child(t(cx, copy::WEEK));
        for (index, name) in form.week.iter().enumerate() {
            body = body.child(
                div()
                    .flex()
                    .items_end()
                    .gap_2()
                    .child(labeled(t(cx, copy::NAME), name, 240.))
                    .child(
                        Button::new(("calendar-remove-weekday", index), t(cx, common::DELETE))
                            .small()
                            .disabled(self.busy)
                            .on_click(cx.listener(move |editor, _, _, cx| {
                                if let Some(form) = &mut editor.form {
                                    form.week.remove(index);
                                }
                                cx.notify();
                            })),
                    ),
            );
        }
        body = body
            .child(
                Button::new("calendar-add-weekday", t(cx, copy::ADD))
                    .disabled(self.busy)
                    .on_click(cx.listener(|editor, _, window, cx| editor.add_weekday(window, cx))),
            )
            .child(labeled(t(cx, copy::WEEK_OFFSET), &form.week_offset, 180.))
            .child(self.cycle(false, cx))
            .child(self.cycle(true, cx));
        body.into_any_element()
    }
}

fn labeled(label: SharedString, field: &Field, width: f32) -> AnyElement {
    div()
        .w(px(width))
        .flex()
        .flex_col()
        .gap_1()
        .child(div().text_xs().child(label))
        .child(field.clone())
        .into_any_element()
}

impl Render for Editor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dirty = self.dirty(cx);
        let picker = div()
            .flex()
            .flex_wrap()
            .gap_2()
            .children(self.calendars.iter().map(|calendar| {
                let spec = calendar.clone();
                Button::new(
                    SharedString::from(format!("calendar-pick-{}", calendar.id)),
                    calendar.name.clone(),
                )
                .disabled(self.busy || dirty)
                .on_click(
                    cx.listener(move |editor, _, window, cx| editor.edit(spec.clone(), window, cx)),
                )
            }))
            .child(
                Button::new("calendar-new", t(cx, copy::NEW_CALENDAR))
                    .disabled(self.busy || dirty)
                    .on_click(cx.listener(|editor, _, window, cx| editor.new_calendar(window, cx))),
            );
        let mut body = div()
            .id("calendar-editor-scroll")
            .max_h(px(560.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap_3()
            .child(picker)
            .child(self.form(cx));
        if self.selection.calibration.is_none() {
            body = body
                .child(t(cx, copy::CALIBRATION))
                .child(labeled(
                    t(cx, copy::AXIS_COORDINATE),
                    &self.anchor_coordinate,
                    180.,
                ))
                .child(labeled(t(cx, copy::DATE), &self.anchor_date, 360.))
                .child(labeled(t(cx, copy::DAYS_PER_UNIT), &self.day_ratio, 180.));
        }
        let me = cx.entity();
        let delete = me.clone();
        let use_calendar = me.clone();
        let close = me.clone();
        let numeric = me.clone();
        Dialog::new("calendar-editor", t(cx, copy::TITLE))
            .width(740.)
            .body(body)
            .body(div().when(self.failed, |el| el.child(t(cx, copy::INVALID))))
            .action(
                Button::new("calendar-save", t(cx, common::SAVE))
                    .primary()
                    .disabled(self.busy || self.form.is_none())
                    .on_click(move |_, window, cx| {
                        me.update(cx, |editor, cx| editor.save(window, cx))
                    }),
            )
            .action(
                Button::new("calendar-use", t(cx, copy::USE_CALENDAR))
                    .disabled(self.busy || dirty || self.base.is_none())
                    .on_click(move |_, window, cx| {
                        use_calendar.update(cx, |editor, cx| editor.use_calendar(window, cx))
                    }),
            )
            .action(
                Button::new("calendar-numeric", t(cx, copy::NUMERIC_SCALE))
                    .disabled(self.busy)
                    .on_click(move |_, window, cx| {
                        numeric.update(cx, |editor, cx| {
                            editor.run(
                                Job::Select {
                                    expected: editor.selection.clone(),
                                    value: calendar::Selection {
                                        calendar: None,
                                        calibration: editor.selection.calibration,
                                    },
                                },
                                window,
                                cx,
                            )
                        })
                    }),
            )
            .action(
                Button::new(
                    "calendar-delete",
                    t(
                        cx,
                        if self.confirm_delete {
                            copy::CONFIRM_DELETE
                        } else {
                            common::DELETE
                        },
                    ),
                )
                .danger()
                .disabled(self.busy || dirty || self.base.is_none())
                .on_click(move |_, window, cx| {
                    delete.update(cx, |editor, cx| {
                        if editor.confirm_delete {
                            if let Some(value) = editor.base.clone() {
                                editor.run(Job::Delete(Box::new(value)), window, cx);
                            }
                        } else {
                            editor.confirm_delete = true;
                            cx.notify();
                        }
                    })
                }),
            )
            .on_close(move |_, cx| close.update(cx, |editor, cx| editor.close(cx)))
            .on_mist(|_, _| {})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;
    use dr_model::calendar::{Date, Era, Month};
    use std::time::{Duration, Instant};

    #[gpui::test]
    fn full_native_form_round_trips_rules_and_saves_selection_without_moving_pins(
        cx: &mut TestAppContext,
    ) {
        cx.executor().allow_parking();
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::open_dir(dir.path(), "test").unwrap();
        let book = lib
            .call(
                &Actor::User,
                "project.create",
                serde_json::json!({"name":"Synthetic calendar UI"}),
            )
            .unwrap()["output"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        lib.call(
            &Actor::User,
            "timeline_view.put",
            serde_json::json!({"book":book,"view":{
                "id":"view","name":"Events","pins":[{"id":"synthetic-pin","label":"Event","at":-20}]
            }}),
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
                    ..Default::default()
                },
                dr_shell::SettingsFile::in_memory(),
                cx,
            )
        });
        let mut spec = CalendarSpec::simple("fiction".into(), "Fiction".into());
        spec.months = vec![
            Month {
                name: "A".into(),
                days: 30,
            },
            Month {
                name: "B".into(),
                days: 10,
            },
        ];
        spec.epoch = Date {
            year: -5,
            month: 0,
            day: 2,
        };
        spec.eras = vec![Era {
            name: "Dawn".into(),
            start_year: -10,
        }];
        spec.intercalations = vec![Intercalation {
            when: Periodic {
                every_years: 3,
                offset_year: -1,
                except_every: Some(6),
                restore_every: Some(12),
            },
            adjustment: LeapAdjustment::Month {
                after_month: 0,
                name: "Leap".into(),
                days: 5,
            },
        }];
        spec.moon = Some(NamedCycle {
            length: 2953,
            denominator: 100,
            offset: 2,
            parts: vec![
                CyclePart {
                    name: "New".into(),
                    starts_at: 0,
                },
                CyclePart {
                    name: "Full".into(),
                    starts_at: 1476,
                },
            ],
        });
        let view: TimelineView = serde_json::from_value(
            lib.call(
                &Actor::User,
                "timeline_view.list",
                serde_json::json!({"book":book}),
            )
            .unwrap()["output"][0]
                .clone(),
        )
        .unwrap();
        let (editor, cx) = cx.add_window_view(|window, cx| {
            Editor::new(
                lib.downgrade(),
                book.clone(),
                view.clone(),
                vec![],
                window,
                cx,
            )
        });
        editor.update_in(cx, |editor, window, cx| {
            editor.edit(spec.clone(), window, cx);
            editor.base = None;
            assert_eq!(editor.form.as_ref().unwrap().value(cx).unwrap(), spec);
            editor.save(window, cx);
        });
        let wait = |cx: &mut VisualTestContext| {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                cx.run_until_parked();
                if !editor.read_with(cx, |e, _| e.busy) {
                    break;
                }
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(3));
            }
            assert!(!editor.read_with(cx, |e, _| e.failed));
        };
        wait(cx);
        editor.update_in(cx, |editor, window, cx| {
            editor
                .anchor_coordinate
                .update(cx, |field, cx| field.set_text("-20", cx));
            editor
                .anchor_date
                .update(cx, |field, cx| field.set_text("Dawn:1/A/1", cx));
            editor
                .day_ratio
                .update(cx, |field, cx| field.set_text("2.5", cx));
            editor.use_calendar(window, cx);
        });
        wait(cx);
        let read = lib
            .call(
                &Actor::User,
                "timeline_view.list",
                serde_json::json!({"book":book}),
            )
            .unwrap()["output"][0]
            .clone();
        assert_eq!(read["pins"], serde_json::to_value(view.pins).unwrap());
        assert_eq!(read["calendar_id"], "fiction");
        assert_eq!(read["calendar_calibration"]["days_per_unit"], 2.5);
        cx.update(|_, cx| {
            cx.global_mut::<dr_shell::Services>().library = None;
        });
        lib.close();
    }
}
