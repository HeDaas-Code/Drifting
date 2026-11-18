//! Pure calendar math for fictional timelines. The storage layer can keep a
//! continuous day number while each book selects a presentation calendar.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarSpec {
    pub id: String,
    pub name: String,
    pub months: Vec<Month>,
    pub leap: Option<LeapRule>,
    pub eras: Vec<Era>,
    pub week_length: u32,
    pub season_cycle_days: Option<u64>,
    pub moon_cycle_days: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Month {
    pub name: String,
    pub days: u32,
    pub leap_days: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeapRule {
    pub every_years: u32,
    pub offset: i64,
    pub month: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Era {
    pub name: String,
    pub start_year: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Date {
    pub year: i64,
    pub month: usize,
    pub day: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CelestialPosition {
    pub weekday: u32,
    pub season_day: Option<u64>,
    pub moon_day: Option<u64>,
}

impl CalendarSpec {
    pub fn days_in_year(&self, year: i64) -> u64 {
        let leap = self.is_leap(year);
        self.months
            .iter()
            .map(|month| u64::from(month.days + if leap { month.leap_days } else { 0 }))
            .sum()
    }

    pub fn is_leap(&self, year: i64) -> bool {
        self.leap.as_ref().is_some_and(|rule| {
            rule.every_years > 0
                && (year - rule.offset).rem_euclid(i64::from(rule.every_years)) == 0
        })
    }

    fn leaps_before(&self, year: i64) -> i64 {
        let Some(rule) = self.leap.as_ref().filter(|rule| rule.every_years > 0) else {
            return 0;
        };
        let period = i64::from(rule.every_years);
        (year - 1 - rule.offset).div_euclid(period) + 1
    }

    fn days_before_year(&self, year: i64) -> i64 {
        let normal: i64 = self.months.iter().map(|month| i64::from(month.days)).sum();
        let extra: i64 = self
            .leap
            .as_ref()
            .map_or(0, |rule| i64::from(self.months[rule.month].leap_days));
        year.saturating_mul(normal)
            .saturating_add(self.leaps_before(year).saturating_mul(extra))
    }

    pub fn celestial_position(&self, continuous_day: i64) -> CelestialPosition {
        let cycle_position = |cycle: Option<u64>| {
            cycle
                .filter(|cycle| *cycle > 0)
                .map(|cycle| continuous_day.rem_euclid(cycle as i64) as u64)
        };
        CelestialPosition {
            weekday: continuous_day.rem_euclid(i64::from(self.week_length.max(1))) as u32,
            season_day: cycle_position(self.season_cycle_days),
            moon_day: cycle_position(self.moon_cycle_days),
        }
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        if self.months.is_empty() || self.months.iter().any(|month| month.days == 0) {
            return Err("months must be nonempty");
        }
        if self.week_length == 0
            || self
                .leap
                .as_ref()
                .is_some_and(|rule| rule.month >= self.months.len())
        {
            return Err("invalid calendar rule");
        }
        Ok(())
    }

    /// Day zero is the author's chosen epoch. Negative years are supported.
    pub fn to_day(&self, date: Date) -> Result<i64, &'static str> {
        self.validate()?;
        if date.month >= self.months.len() || date.day == 0 {
            return Err("invalid date");
        }
        let year_start = self.days_before_year(date.year);
        let month_start: i64 = self.months[..date.month]
            .iter()
            .map(|month| i64::from(month.days))
            .sum::<i64>()
            + if self.is_leap(date.year)
                && self
                    .leap
                    .as_ref()
                    .is_some_and(|rule| rule.month < date.month)
            {
                self.leap
                    .as_ref()
                    .map_or(0, |rule| i64::from(self.months[rule.month].leap_days))
            } else {
                0
            };
        let max = i64::from(self.months[date.month].days)
            + if self.is_leap(date.year) {
                i64::from(self.months[date.month].leap_days)
            } else {
                0
            };
        if i64::from(date.day) > max {
            return Err("invalid date");
        }
        Ok(year_start + month_start + i64::from(date.day - 1))
    }

    pub fn from_day(&self, mut day: i64) -> Result<Date, &'static str> {
        self.validate()?;
        let mut low = day.div_euclid(self.days_in_year(0).max(1) as i64).min(0) - 4;
        let mut high = day.div_euclid(self.days_in_year(0).max(1) as i64).max(0) + 4;
        while low + 1 < high {
            let mid = low + (high - low) / 2;
            if self.days_before_year(mid) <= day {
                low = mid;
            } else {
                high = mid;
            }
        }
        let year = low;
        day -= self.days_before_year(year);
        for (month, spec) in self.months.iter().enumerate() {
            let length = u64::from(
                spec.days
                    + if self.is_leap(year) {
                        spec.leap_days
                    } else {
                        0
                    },
            ) as i64;
            if day < length {
                return Ok(Date {
                    year,
                    month,
                    day: day as u32 + 1,
                });
            }
            day -= length;
        }
        Err("day outside calendar")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn calendar() -> CalendarSpec {
        CalendarSpec {
            id: "synthetic".into(),
            name: "Synthetic Era".into(),
            months: vec![
                Month {
                    name: "Aster".into(),
                    days: 30,
                    leap_days: 1,
                },
                Month {
                    name: "Brume".into(),
                    days: 30,
                    leap_days: 0,
                },
            ],
            leap: Some(LeapRule {
                every_years: 4,
                offset: 0,
                month: 0,
            }),
            eras: vec![
                Era {
                    name: "Before Dawn".into(),
                    start_year: -10,
                },
                Era {
                    name: "Dawn".into(),
                    start_year: 1,
                },
            ],
            week_length: 6,
            season_cycle_days: Some(60),
            moon_cycle_days: Some(15),
        }
    }

    #[test]
    fn round_trips_negative_years_and_leap_months() {
        let cal = calendar();
        for date in [
            Date {
                year: -3,
                month: 1,
                day: 20,
            },
            Date {
                year: 0,
                month: 0,
                day: 31,
            },
            Date {
                year: 4,
                month: 0,
                day: 31,
            },
        ] {
            assert_eq!(cal.from_day(cal.to_day(date).unwrap()).unwrap(), date);
        }
    }
}
