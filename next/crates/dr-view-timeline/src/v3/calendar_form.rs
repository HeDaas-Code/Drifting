//! Native form drafts for all author-defined calendar rules.
use dr_model::calendar::*;
use dr_ui::input::field::TextField;
use gpui::{App, Entity};

pub(super) type Field = Entity<TextField>;

pub(super) struct MonthForm {
    pub name: Field,
    pub days: Field,
}
pub(super) struct EraForm {
    pub name: Field,
    pub year: Field,
}
pub(super) struct LeapForm {
    pub every: Field,
    pub offset: Field,
    pub except: Field,
    pub restore: Field,
    pub target: Field,
    pub days: Field,
    pub name: Field,
    pub month: bool,
}
pub(super) struct PartForm {
    pub name: Field,
    pub start: Field,
}
pub(super) struct CycleForm {
    pub length: Field,
    pub denominator: Field,
    pub offset: Field,
    pub parts: Vec<PartForm>,
}
pub(super) struct Form {
    id: String,
    pub name: Field,
    pub epoch: Field,
    pub epoch_day: Field,
    pub week_offset: Field,
    pub months: Vec<MonthForm>,
    pub leaps: Vec<LeapForm>,
    pub eras: Vec<EraForm>,
    pub week: Vec<Field>,
    pub seasons: Option<CycleForm>,
    pub moon: Option<CycleForm>,
}

fn text(field: &Field, cx: &App) -> String {
    field.read(cx).text().trim().into()
}
fn number<T: std::str::FromStr>(field: &Field, cx: &App) -> Result<T, String> {
    text(field, cx).parse().map_err(|_| "invalid number".into())
}
fn optional(field: &Field, cx: &App) -> Result<Option<u32>, String> {
    if text(field, cx).is_empty() {
        Ok(None)
    } else {
        number(field, cx).map(Some)
    }
}

impl MonthForm {
    pub fn new(name: String, days: u32, field: &mut impl FnMut(String) -> Field) -> Self {
        Self {
            name: field(name),
            days: field(days.to_string()),
        }
    }
}
impl EraForm {
    pub fn new(name: String, year: i64, field: &mut impl FnMut(String) -> Field) -> Self {
        Self {
            name: field(name),
            year: field(year.to_string()),
        }
    }
}
impl LeapForm {
    pub fn new(rule: &Intercalation, field: &mut impl FnMut(String) -> Field) -> Self {
        let (month, target, days, name) = match &rule.adjustment {
            LeapAdjustment::Days { month, days } => (false, *month, *days, String::new()),
            LeapAdjustment::Month {
                after_month,
                name,
                days,
            } => (true, *after_month, *days, name.clone()),
        };
        Self {
            every: field(rule.when.every_years.to_string()),
            offset: field(rule.when.offset_year.to_string()),
            except: field(
                rule.when
                    .except_every
                    .map(|n| n.to_string())
                    .unwrap_or_default(),
            ),
            restore: field(
                rule.when
                    .restore_every
                    .map(|n| n.to_string())
                    .unwrap_or_default(),
            ),
            target: field((target + 1).to_string()),
            days: field(days.to_string()),
            name: field(name),
            month,
        }
    }
    fn value(&self, cx: &App) -> Result<Intercalation, String> {
        let target: usize = number(&self.target, cx)?;
        let target = target.checked_sub(1).ok_or("month indices start at 1")?;
        let days = number(&self.days, cx)?;
        Ok(Intercalation {
            when: Periodic {
                every_years: number(&self.every, cx)?,
                offset_year: number(&self.offset, cx)?,
                except_every: optional(&self.except, cx)?,
                restore_every: optional(&self.restore, cx)?,
            },
            adjustment: if self.month {
                LeapAdjustment::Month {
                    after_month: target,
                    name: text(&self.name, cx),
                    days,
                }
            } else {
                LeapAdjustment::Days {
                    month: target,
                    days,
                }
            },
        })
    }
}

impl CycleForm {
    pub fn new(cycle: &NamedCycle, field: &mut impl FnMut(String) -> Field) -> Self {
        Self {
            length: field(cycle.length.to_string()),
            denominator: field(cycle.denominator.to_string()),
            offset: field(cycle.offset.to_string()),
            parts: cycle
                .parts
                .iter()
                .map(|part| PartForm {
                    name: field(part.name.clone()),
                    start: field(part.starts_at.to_string()),
                })
                .collect(),
        }
    }
    fn value(&self, cx: &App) -> Result<NamedCycle, String> {
        Ok(NamedCycle {
            length: number(&self.length, cx)?,
            denominator: number(&self.denominator, cx)?,
            offset: number(&self.offset, cx)?,
            parts: self
                .parts
                .iter()
                .map(|part| {
                    Ok(CyclePart {
                        name: text(&part.name, cx),
                        starts_at: number(&part.start, cx)?,
                    })
                })
                .collect::<Result<_, String>>()?,
        })
    }
}

impl Form {
    pub fn new(calendar: &CalendarSpec, field: &mut impl FnMut(String) -> Field) -> Self {
        Self {
            id: calendar.id.clone(),
            name: field(calendar.name.clone()),
            epoch: field(calendar.format_day(calendar.epoch_day).unwrap_or_default()),
            epoch_day: field(calendar.epoch_day.to_string()),
            week_offset: field(calendar.week_offset.to_string()),
            months: calendar
                .months
                .iter()
                .map(|m| MonthForm::new(m.name.clone(), m.days, field))
                .collect(),
            leaps: calendar
                .intercalations
                .iter()
                .map(|rule| LeapForm::new(rule, field))
                .collect(),
            eras: calendar
                .eras
                .iter()
                .map(|era| EraForm::new(era.name.clone(), era.start_year, field))
                .collect(),
            week: calendar
                .week
                .iter()
                .map(|name| field(name.clone()))
                .collect(),
            seasons: calendar
                .seasons
                .as_ref()
                .map(|cycle| CycleForm::new(cycle, field)),
            moon: calendar
                .moon
                .as_ref()
                .map(|cycle| CycleForm::new(cycle, field)),
        }
    }

    pub fn value(&self, cx: &App) -> Result<CalendarSpec, String> {
        let mut value = CalendarSpec {
            id: self.id.clone(),
            name: text(&self.name, cx),
            epoch: Date {
                year: 0,
                month: 0,
                day: 1,
            },
            epoch_day: number(&self.epoch_day, cx)?,
            months: self
                .months
                .iter()
                .map(|m| {
                    Ok(Month {
                        name: text(&m.name, cx),
                        days: number(&m.days, cx)?,
                    })
                })
                .collect::<Result<_, String>>()?,
            intercalations: self
                .leaps
                .iter()
                .map(|rule| rule.value(cx))
                .collect::<Result<_, _>>()?,
            eras: self
                .eras
                .iter()
                .map(|era| {
                    Ok(Era {
                        name: text(&era.name, cx),
                        start_year: number(&era.year, cx)?,
                    })
                })
                .collect::<Result<_, String>>()?,
            week: self.week.iter().map(|field| text(field, cx)).collect(),
            week_offset: number(&self.week_offset, cx)?,
            seasons: self
                .seasons
                .as_ref()
                .map(|cycle| cycle.value(cx))
                .transpose()?,
            moon: self
                .moon
                .as_ref()
                .map(|cycle| cycle.value(cx))
                .transpose()?,
        };
        value.epoch = value
            .parse_date(&text(&self.epoch, cx))
            .map_err(str::to_owned)?;
        value.validate().map_err(str::to_owned)?;
        Ok(value)
    }
}
