//! Author-defined calendars over a continuous signed day number.
//! No astronomical ephemeris or Earth-specific rule is implicit.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CalendarSpec {
    pub id: String,
    pub name: String,
    pub epoch: Date,
    pub epoch_day: i64,
    pub months: Vec<Month>,
    #[serde(default)]
    pub intercalations: Vec<Intercalation>,
    #[serde(default)]
    pub eras: Vec<Era>,
    /// Empty means that this world does not group days into weeks.
    #[serde(default)]
    pub week: Vec<String>,
    #[serde(default)]
    pub week_offset: i64,
    #[serde(default)]
    pub seasons: Option<NamedCycle>,
    #[serde(default)]
    pub moon: Option<NamedCycle>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Month {
    pub name: String,
    pub days: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Periodic {
    pub every_years: u32,
    pub offset_year: i64,
    #[serde(default)]
    pub except_every: Option<u32>,
    #[serde(default)]
    pub restore_every: Option<u32>,
}

impl Periodic {
    fn validate(&self) -> Result<(), &'static str> {
        if self.every_years == 0
            || self
                .except_every
                .is_some_and(|n| n == 0 || n % self.every_years != 0)
            || self.restore_every.is_some_and(|n| {
                self.except_every
                    .is_none_or(|except| n == 0 || n % except != 0)
            })
        {
            return Err("invalid periodic leap rule");
        }
        Ok(())
    }

    pub fn occurs(&self, year: i64) -> bool {
        let shifted = i128::from(year) - i128::from(self.offset_year);
        self.every_years > 0
            && shifted.rem_euclid(i128::from(self.every_years)) == 0
            && (self
                .except_every
                .is_none_or(|n| n > 0 && shifted.rem_euclid(i128::from(n)) != 0)
                || self
                    .restore_every
                    .is_some_and(|n| n > 0 && shifted.rem_euclid(i128::from(n)) == 0))
    }

    fn count_before(&self, year: i128) -> i128 {
        let offset = i128::from(self.offset_year);
        let count = |period: u32| {
            let p = i128::from(period);
            (year - 1 - offset).div_euclid(p) - (-1 - offset).div_euclid(p)
        };
        count(self.every_years) - self.except_every.map_or(0, count)
            + self.restore_every.map_or(0, count)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Intercalation {
    pub when: Periodic,
    pub adjustment: LeapAdjustment,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LeapAdjustment {
    Days {
        month: usize,
        days: u32,
    },
    Month {
        after_month: usize,
        name: String,
        days: u32,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Era {
    pub name: String,
    pub start_year: i64,
}

/// Month is an ordinal in that year's month sequence, including inserted months.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Date {
    pub year: i64,
    pub month: usize,
    pub day: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CyclePart {
    pub name: String,
    pub starts_at: u64,
}

/// Period and phase starts are integer units of 1/denominator days.
/// This can express e.g. a 29.53-day moon with length=2953, denominator=100.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NamedCycle {
    pub length: u64,
    pub denominator: u32,
    pub offset: i64,
    pub parts: Vec<CyclePart>,
}

impl NamedCycle {
    fn validate(&self) -> Result<(), &'static str> {
        if self.length == 0
            || self.denominator == 0
            || self.parts.is_empty()
            || self.parts.len() > 128
            || self.parts[0].starts_at != 0
            || self
                .parts
                .iter()
                .any(|p| p.starts_at >= self.length || !valid_name(&p.name))
            || self
                .parts
                .windows(2)
                .any(|p| p[0].starts_at >= p[1].starts_at)
            || !unique(self.parts.iter().map(|p| p.name.as_str()))
        {
            return Err("invalid named cycle");
        }
        Ok(())
    }

    pub fn position(&self, day: i64) -> Result<(&str, u64), &'static str> {
        self.validate()?;
        let phase = (i128::from(day) * i128::from(self.denominator) - i128::from(self.offset))
            .rem_euclid(i128::from(self.length)) as u64;
        let part = self
            .parts
            .iter()
            .rev()
            .find(|part| part.starts_at <= phase)
            .ok_or("invalid cycle")?;
        Ok((&part.name, phase - part.starts_at))
    }
}

/// An explicit calibration of a pre-calendar floating timeline axis.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Calibration {
    pub coordinate: f64,
    pub day: i64,
    pub days_per_unit: f64,
}

impl Calibration {
    pub fn validate(self) -> Result<(), &'static str> {
        if !self.coordinate.is_finite()
            || self.coordinate.abs() > (1u64 << 52) as f64
            || !self.days_per_unit.is_finite()
            || self.days_per_unit <= 0.
            || self.days_per_unit > 1e9
            || self.day.unsigned_abs() > (1u64 << 52)
        {
            return Err("invalid timeline calibration");
        }
        Ok(())
    }

    pub fn to_day(self, coordinate: f64) -> Result<i64, &'static str> {
        self.validate()?;
        let computed = self.day as f64 + (coordinate - self.coordinate) * self.days_per_unit;
        let nearest = computed.round();
        let day = if (computed - nearest).abs() <= 1e-8 {
            nearest
        } else {
            computed.floor()
        };
        if !day.is_finite() || day.abs() > (1u64 << 52) as f64 {
            return Err("timeline day exceeds precise coordinate range");
        }
        Ok(day as i64)
    }

    pub fn to_coordinate(self, day: i64) -> Result<f64, &'static str> {
        self.validate()?;
        if day.unsigned_abs() > (1u64 << 52) {
            return Err("timeline day exceeds precise coordinate range");
        }
        let at = self.coordinate + (day as f64 - self.day as f64) / self.days_per_unit;
        if !at.is_finite() || at.abs() > (1u64 << 52) as f64 {
            return Err("timeline coordinate overflow");
        }
        if self.to_day(at)? != day {
            return Err("timeline coordinate cannot precisely represent this date");
        }
        Ok(at)
    }
}

fn valid_name(name: &str) -> bool {
    !name.trim().is_empty() && name.len() <= 256 && !name.chars().any(char::is_control)
}

fn unique<'a>(names: impl Iterator<Item = &'a str>) -> bool {
    let mut seen = BTreeSet::new();
    names
        .into_iter()
        .all(|name| seen.insert(name.trim().to_lowercase()))
}

impl CalendarSpec {
    pub fn simple(id: String, name: String) -> Self {
        Self {
            id,
            name,
            epoch: Date {
                year: 0,
                month: 0,
                day: 1,
            },
            epoch_day: 0,
            months: (1..=12)
                .map(|n| Month {
                    name: n.to_string(),
                    days: 30,
                })
                .collect(),
            intercalations: vec![],
            eras: vec![],
            week: (1..=7).map(|n| n.to_string()).collect(),
            week_offset: 0,
            seasons: None,
            moon: None,
        }
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        if !valid_name(&self.id)
            || !self
                .id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            || !valid_name(&self.name)
            || self.months.is_empty()
            || self.months.len() > 512
            || self.intercalations.len() > 256
            || self.eras.len() > 256
            || self.week.len() > 128
            || self.months.iter().any(|m| {
                !valid_name(&m.name)
                    || m.name.contains(['/', ':'])
                    || !(1..=1_000_000).contains(&m.days)
            })
            || !unique(self.months.iter().map(|m| m.name.as_str()))
            || !unique(self.week.iter().map(String::as_str))
            || self.week.iter().any(|name| !valid_name(name))
            || self
                .eras
                .iter()
                .any(|era| !valid_name(&era.name) || era.name.contains(['/', ':']))
            || !unique(self.eras.iter().map(|era| era.name.as_str()))
            || self
                .eras
                .iter()
                .map(|era| era.start_year)
                .collect::<BTreeSet<_>>()
                .len()
                != self.eras.len()
        {
            return Err("calendar names and lengths must be valid and unambiguous");
        }
        let mut names: BTreeSet<_> = self
            .months
            .iter()
            .map(|m| m.name.trim().to_lowercase())
            .collect();
        for leap in &self.intercalations {
            leap.when.validate()?;
            let (index, days) = match &leap.adjustment {
                LeapAdjustment::Days { month, days } => (*month, *days),
                LeapAdjustment::Month {
                    after_month,
                    name,
                    days,
                } => {
                    if !valid_name(name)
                        || name.contains(['/', ':'])
                        || !names.insert(name.trim().to_lowercase())
                    {
                        return Err("intercalary month must have a unique name");
                    }
                    (*after_month, *days)
                }
            };
            if index >= self.months.len() || !(1..=1_000_000).contains(&days) {
                return Err("invalid intercalary day or month");
            }
        }
        if let Some(cycle) = &self.seasons {
            cycle.validate()?;
        }
        if let Some(cycle) = &self.moon {
            cycle.validate()?;
        }
        self.raw_day(self.epoch)?;
        Ok(())
    }

    pub fn months_in_year(&self, year: i64) -> Vec<Month> {
        let mut months = Vec::new();
        for (index, month) in self.months.iter().enumerate() {
            let mut month = month.clone();
            for leap in self
                .intercalations
                .iter()
                .filter(|leap| leap.when.occurs(year))
            {
                if let LeapAdjustment::Days {
                    month: target,
                    days,
                } = leap.adjustment
                    && target == index
                {
                    month.days = month.days.saturating_add(days);
                }
            }
            months.push(month);
            for leap in self
                .intercalations
                .iter()
                .filter(|leap| leap.when.occurs(year))
            {
                if let LeapAdjustment::Month {
                    after_month,
                    name,
                    days,
                } = &leap.adjustment
                    && *after_month == index
                {
                    months.push(Month {
                        name: name.clone(),
                        days: *days,
                    });
                }
            }
        }
        months
    }

    fn year_start(&self, year: i128) -> i128 {
        let ordinary: i128 = self.months.iter().map(|m| i128::from(m.days)).sum();
        self.intercalations
            .iter()
            .fold(year * ordinary, |days, leap| {
                let extra = match leap.adjustment {
                    LeapAdjustment::Days { days, .. } | LeapAdjustment::Month { days, .. } => days,
                };
                days + leap.when.count_before(year) * i128::from(extra)
            })
    }

    fn raw_day(&self, date: Date) -> Result<i128, &'static str> {
        let months = self.months_in_year(date.year);
        let month = months
            .get(date.month)
            .ok_or("month is absent in this year")?;
        if date.day == 0 || date.day > month.days {
            return Err("day is outside the month");
        }
        Ok(self.year_start(i128::from(date.year))
            + months[..date.month]
                .iter()
                .map(|m| i128::from(m.days))
                .sum::<i128>()
            + i128::from(date.day - 1))
    }

    pub fn to_day(&self, date: Date) -> Result<i64, &'static str> {
        self.validate()?;
        (self.raw_day(date)? - self.raw_day(self.epoch)? + i128::from(self.epoch_day))
            .try_into()
            .map_err(|_| "continuous day overflow")
    }

    pub fn from_day(&self, day: i64) -> Result<Date, &'static str> {
        self.validate()?;
        let absolute = i128::from(day) - i128::from(self.epoch_day) + self.raw_day(self.epoch)?;
        let mut low = i128::from(i64::MIN);
        let mut high = i128::from(i64::MAX) + 1;
        if self.year_start(low) > absolute {
            return Err("date year overflow");
        }
        while low + 1 < high {
            let mid = low + (high - low) / 2;
            if self.year_start(mid) <= absolute {
                low = mid;
            } else {
                high = mid;
            }
        }
        let year = low as i64;
        let mut rest = absolute - self.year_start(low);
        for (month, spec) in self.months_in_year(year).iter().enumerate() {
            if rest < i128::from(spec.days) {
                return Ok(Date {
                    year,
                    month,
                    day: rest as u32 + 1,
                });
            }
            rest -= i128::from(spec.days);
        }
        Err("date year overflow")
    }

    pub fn format_day(&self, day: i64) -> Result<String, &'static str> {
        let date = self.from_day(day)?;
        let months = self.months_in_year(date.year);
        let prefix = self
            .eras
            .iter()
            .filter(|era| era.start_year <= date.year)
            .max_by_key(|era| era.start_year)
            .map(|era| {
                format!(
                    "{}:{}",
                    era.name,
                    i128::from(date.year) - i128::from(era.start_year) + 1
                )
            })
            .unwrap_or_else(|| date.year.to_string());
        Ok(format!("{prefix}/{}/{}", months[date.month].name, date.day))
    }

    /// Signed year/month-name/day, or era-name:year/month-name/day.
    pub fn parse_date(&self, input: &str) -> Result<Date, &'static str> {
        self.validate()?;
        let fields: Vec<_> = input.trim().rsplitn(3, '/').collect();
        if fields.len() != 3 {
            return Err("use year/month/day");
        }
        let day: u32 = fields[0].trim().parse().map_err(|_| "invalid day")?;
        let year = if let Some((era, relative)) = fields[2].split_once(':') {
            let era = self
                .eras
                .iter()
                .find(|item| item.name == era.trim())
                .ok_or("unknown era")?;
            let relative: i128 = relative.trim().parse().map_err(|_| "invalid era year")?;
            (i128::from(era.start_year)
                .checked_add(relative)
                .and_then(|v| v.checked_sub(1))
                .ok_or("date year overflow")?)
            .try_into()
            .map_err(|_| "date year overflow")?
        } else {
            fields[2].trim().parse().map_err(|_| "invalid year")?
        };
        let months = self.months_in_year(year);
        let month = months
            .iter()
            .position(|m| m.name == fields[1].trim())
            .ok_or("month is absent in this year")?;
        let date = Date { year, month, day };
        self.to_day(date)?;
        Ok(date)
    }

    pub fn weekday(&self, day: i64) -> Option<&str> {
        (!self.week.is_empty()).then(|| {
            let at = (i128::from(day) - i128::from(self.week_offset))
                .rem_euclid(self.week.len() as i128);
            self.week[at as usize].as_str()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fictional() -> CalendarSpec {
        let mut cal = CalendarSpec::simple("cal".into(), "Dawn".into());
        cal.months = vec![
            Month {
                name: "Aster".into(),
                days: 30,
            },
            Month {
                name: "Brume".into(),
                days: 20,
            },
        ];
        cal.intercalations = vec![
            Intercalation {
                when: Periodic {
                    every_years: 4,
                    offset_year: 0,
                    except_every: None,
                    restore_every: None,
                },
                adjustment: LeapAdjustment::Days { month: 0, days: 1 },
            },
            Intercalation {
                when: Periodic {
                    every_years: 3,
                    offset_year: -1,
                    except_every: None,
                    restore_every: None,
                },
                adjustment: LeapAdjustment::Month {
                    after_month: 0,
                    name: "Interlude".into(),
                    days: 7,
                },
            },
        ];
        cal.eras = vec![
            Era {
                name: "Dawn".into(),
                start_year: -10,
            },
            Era {
                name: "New Dawn".into(),
                start_year: 1,
            },
        ];
        cal
    }

    #[test]
    fn all_days_round_trip_across_negative_years_epochs_and_intercalary_months() {
        let mut cal = fictional();
        cal.epoch = Date {
            year: -4,
            month: 1,
            day: 3,
        };
        cal.epoch_day = -500;
        for day in -10_000..10_000 {
            let date = cal.from_day(day).unwrap();
            assert_eq!(cal.to_day(date).unwrap(), day);
            let text = cal.format_day(day).unwrap();
            assert_eq!(cal.parse_date(&text).unwrap(), date, "{text}");
        }
        assert!(cal.parse_date("0/Interlude/1").is_err());
        assert!(cal.parse_date("2/Interlude/1").is_ok());
    }

    #[test]
    fn century_exclusions_restore_the_cycle_and_extreme_years_do_not_panic() {
        let rule = Periodic {
            every_years: 4,
            offset_year: 0,
            except_every: Some(100),
            restore_every: Some(400),
        };
        assert!(rule.occurs(-400));
        assert!(!rule.occurs(-100));
        assert!(rule.occurs(0));
        let cal = fictional();
        assert!(
            cal.to_day(Date {
                year: i64::MAX,
                month: 0,
                day: 1
            })
            .is_err()
        );
        for day in [i64::MIN, -1, 0, 1, i64::MAX] {
            assert_eq!(cal.to_day(cal.from_day(day).unwrap()).unwrap(), day);
        }
    }

    #[test]
    fn rational_named_cycles_and_week_use_author_periods_in_negative_time() {
        let cycle = NamedCycle {
            length: 2953,
            denominator: 100,
            offset: 0,
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
        };
        assert_eq!(cycle.position(15).unwrap(), ("Full", 24));
        assert_eq!(cycle.position(-1).unwrap(), ("Full", 1377));
        let mut cal = fictional();
        cal.week = vec!["A".into(), "B".into(), "C".into()];
        assert_eq!(cal.weekday(-1), Some("C"));
    }

    #[test]
    fn fractional_day_scales_round_trip_and_refuse_precision_loss() {
        for ratio in [0.1, 0.3, 2.5, 3., 7.7] {
            let mapping = Calibration {
                coordinate: -2.3,
                day: -100,
                days_per_unit: ratio,
            };
            for day in -1000..1000 {
                assert_eq!(
                    mapping.to_day(mapping.to_coordinate(day).unwrap()).unwrap(),
                    day
                );
            }
        }
        assert!(
            Calibration {
                coordinate: 1e30,
                day: 0,
                days_per_unit: 1.
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn calibration_changes_date_labels_without_moving_existing_events() {
        let mapping = Calibration {
            coordinate: -5.,
            day: 100,
            days_per_unit: 2.5,
        };
        assert_eq!(mapping.to_day(-1.).unwrap(), 110);
        assert_eq!(mapping.to_coordinate(110).unwrap(), -1.);
        assert!(
            Calibration {
                days_per_unit: 0.,
                ..mapping
            }
            .validate()
            .is_err()
        );
        assert!(mapping.to_day(f64::INFINITY).is_err());
    }
}
