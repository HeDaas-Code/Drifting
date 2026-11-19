//! Book-scoped calendars and field-scoped timeline ruler selection.
use super::*;
use dr_model::calendar::{CalendarSpec, Calibration};

pub(crate) fn register(r: &mut Registry) {
    r.add::<CalendarList>()
        .add::<CalendarPut>()
        .add::<CalendarDelete>()
        .add::<CalendarRestore>()
        .add::<ViewCalendar>()
        .add::<ViewCalendarRestore>();
}

op! {
    CalendarList = "calendar.list", Read, Shared,
    "List this book's author-defined calendars, including epochs, months, leap rules and celestial cycles.",
    fn(cx, input: BookIn) -> Vec<CalendarSpec> {
        Ok(cx.book_v3(&input.book)?.reader_v3.view().project().calendars)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct PutIn {
    pub book: String,
    pub calendar: CalendarSpec,
}

op! {
    CalendarPut = "calendar.put", Write, Book,
    "Create or replace one author-defined calendar without moving timeline events.",
    fn(cx, input: PutIn) -> Done {
        input.calendar.validate().map_err(WsError::invalid)?;
        let slot = cx.book_v3(&input.book)?;
        let before = slot.reader_v3.view().calendar(&input.calendar.id)?;
        change_calendar(cx, &slot, &input.calendar.id, before, Some(input.calendar.clone()))
    }
}

op! {
    CalendarDelete = "calendar.delete", Write, Book,
    "Delete a calendar that no stored timeline uses.",
    fn(cx, input: EntityIn) -> Done {
        let slot = cx.book_v3(&input.book)?;
        let before = slot.reader_v3.view().calendar(&input.id)?;
        if before.is_none() { return Err(WsError::not_found("calendar", input.id)); }
        change_calendar(cx, &slot, &input.id, before, None)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct RestoreIn {
    pub book: String,
    pub id: String,
    pub expected: Option<CalendarSpec>,
    pub value: Option<CalendarSpec>,
}
op! {
    CalendarRestore = "calendar.restore", Write, Book,
    "Restore one calendar after checking its exact expected value.",
    fn(cx, input: RestoreIn) -> Done {
        let slot = cx.book_v3(&input.book)?;
        let before = slot.reader_v3.view().calendar(&input.id)?;
        if before != input.expected { return Err(WsError::Conflict("calendar changed after this edit".into())); }
        change_calendar(cx, &slot, &input.id, before, input.value)
    }
}

fn change_calendar(
    cx: &Cx<'_>,
    slot: &BookSlot,
    id: &str,
    before: Option<CalendarSpec>,
    value: Option<CalendarSpec>,
) -> Result<Done> {
    if id.is_empty()
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(WsError::invalid("invalid calendar id"));
    }
    if let Some(calendar) = &value {
        calendar.validate().map_err(WsError::invalid)?;
        if calendar.id != id {
            return Err(WsError::invalid("calendar id mismatch"));
        }
    } else if slot
        .reader_v3
        .view()
        .timeline_views_raw()
        .iter()
        .any(|v| v.calendar_id.as_deref() == Some(id))
    {
        return Err(WsError::Locked("calendar is used by a timeline".into()));
    }
    if before == value {
        cx.undo_nothing();
        return Ok(Done { id: id.into() });
    }
    let key = rec(CollectionV3::Project, "calendars").at([id]);
    let (writing_id, writing_value) = (id.to_owned(), value.clone());
    cx.write_book_v3(slot, Replaces::records([key]), move |tx| {
        tx.project().calendar(&writing_id, writing_value.as_ref())?;
        Ok(())
    })?;
    cx.undo_with(Invocation::new(
        "calendar.restore",
        RestoreIn {
            book: slot.book.to_string(),
            id: id.into(),
            expected: value,
            value: before,
        },
    ));
    cx.effect(EffectAction::Updated, "calendar", id, None);
    Ok(Done { id: id.into() })
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Selection {
    pub calendar: Option<String>,
    pub calibration: Option<Calibration>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ViewIn {
    pub book: String,
    pub view: String,
    pub calendar: Option<String>,
    /// Required on first use. Later selections reuse the original calibration.
    #[serde(default)]
    pub calibration: Option<Calibration>,
}

op! {
    ViewCalendar = "timeline_view.calendar", Write, Book,
    "Choose a calendar for this timeline; first use requires an explicit day-to-axis calibration.",
    fn(cx, input: ViewIn) -> Done {
        let slot = cx.book_v3(&input.book)?;
        let vid: TimelineViewId = pid("timeline view", &input.view)?;
        let view = slot.reader_v3.view().timeline_views_raw().into_iter()
            .find(|view| view.id == vid && !view.is_trashed())
            .ok_or_else(|| WsError::not_found("timeline view", &input.view))?;
        if let (Some(stored), Some(requested)) = (view.calendar_calibration, input.calibration)
            && stored != requested
        {
            return Err(WsError::invalid("a calibrated timeline keeps its original continuous coordinate mapping"));
        }
        let mapping = input.calibration.or(view.calendar_calibration);
        change_selection(cx, &slot, &vid, Selection {
            calendar: view.calendar_id, calibration: view.calendar_calibration,
        }, Selection { calendar: input.calendar, calibration: mapping })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct RestoreSelectionIn {
    pub book: String,
    pub view: String,
    pub expected: Selection,
    pub value: Selection,
}

op! {
    ViewCalendarRestore = "timeline_view.restore_calendar", Write, Book,
    "Restore only a timeline's calendar selection and calibration after a value check.",
    fn(cx, input: RestoreSelectionIn) -> Done {
        let slot = cx.book_v3(&input.book)?;
        let vid: TimelineViewId = pid("timeline view", &input.view)?;
        let view = slot.reader_v3.view().timeline_views_raw().into_iter()
            .find(|view| view.id == vid).ok_or_else(|| WsError::not_found("timeline view", &input.view))?;
        let before = Selection { calendar: view.calendar_id, calibration: view.calendar_calibration };
        if before != input.expected { return Err(WsError::Conflict("timeline calendar changed after this edit".into())); }
        change_selection(cx, &slot, &vid, before, input.value)
    }
}

fn change_selection(
    cx: &Cx<'_>,
    slot: &BookSlot,
    view: &TimelineViewId,
    before: Selection,
    value: Selection,
) -> Result<Done> {
    if let Some(mapping) = value.calibration {
        mapping.validate().map_err(WsError::invalid)?;
    }
    if let Some(id) = &value.calendar {
        if slot.reader_v3.view().calendar(id)?.is_none() {
            return Err(WsError::not_found("calendar", id));
        }
        if value.calibration.is_none() {
            return Err(WsError::invalid(
                "choose the timeline anchor and day length first",
            ));
        }
    }
    if before == value {
        cx.undo_nothing();
        return Ok(Done {
            id: view.to_string(),
        });
    }
    let (writing_view, writing_value) = (view.clone(), value.clone());
    cx.write_book_v3(
        slot,
        Replaces::records([
            rec(CollectionV3::TimelineViews, view).at(["calendar_id"]),
            rec(CollectionV3::TimelineViews, view).at(["calendar_calibration"]),
        ]),
        move |tx| {
            tx.timeline_view(&writing_view)?
                .calendar_id(writing_value.calendar.as_deref())?
                .calendar_calibration(writing_value.calibration)?;
            Ok(())
        },
    )?;
    cx.undo_with(Invocation::new(
        "timeline_view.restore_calendar",
        RestoreSelectionIn {
            book: slot.book.to_string(),
            view: view.to_string(),
            expected: value,
            value: before,
        },
    ));
    cx.effect(EffectAction::Updated, "timeline_view", view.as_str(), None);
    Ok(Done {
        id: view.to_string(),
    })
}
