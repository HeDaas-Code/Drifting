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

impl CalendarSpec {
    pub fn days_in_year(&self, year: i64) -> u64 {
        let leap = self.is_leap(year);
        self.months.iter().map(|month| u64::from(month.days + if leap { month.leap_days } else { 0 })).sum()
    }

    pub fn is_leap(&self, year: i64) -> bool {
        self.leap.as_ref().is_some_and(|rule| {
            rule.every_years > 0 && (year - rule.offset).rem_euclid(i64::from(rule.every_years)) == 0
        })
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        if self.months.is_empty() || self.months.iter().any(|month| month.days == 0) { return Err("months must be nonempty"); }
        if self.week_length == 0 || self.leap.as_ref().is_some_and(|rule| rule.month >= self.months.len()) { return Err("invalid calendar rule"); }
        Ok(())
    }

    /// Day zero is the author's chosen epoch. Negative years are supported.
    pub fn to_day(&self, date: Date) -> Result<i64, &'static str> {
        self.validate()?;
        if date.month >= self.months.len() || date.day == 0 { return Err("invalid date"); }
        let year_start = if date.year >= 0 {
            (0..date.year).map(|year| self.days_in_year(year) as i64).sum::<i64>()
        } else {
            -(date.year..0).map(|year| self.days_in_year(year - 1) as i64).sum::<i64>()
        };
        let month_start: i64 = self.months[..date.month].iter().map(|month| i64::from(month.days)).sum::<i64>()
            + if self.is_leap(date.year) && self.leap.as_ref().is_some_and(|rule| rule.month < date.month) {
                self.leap.as_ref().map_or(0, |rule| i64::from(self.months[rule.month].leap_days))
            } else { 0 };
        let max = i64::from(self.months[date.month].days) + if self.is_leap(date.year) { i64::from(self.months[date.month].leap_days) } else { 0 };
        if i64::from(date.day) > max { return Err("invalid date"); }
        Ok(year_start + month_start + i64::from(date.day - 1))
    }

    pub fn from_day(&self, mut day: i64) -> Result<Date, &'static str> {
        self.validate()?;
        let mut year = 0i64;
        if day >= 0 { while day >= self.days_in_year(year) as i64 { day -= self.days_in_year(year) as i64; year += 1; } }
        else { while day < 0 { year -= 1; day += self.days_in_year(year) as i64; } }
        for (month, spec) in self.months.iter().enumerate() {
            let length = u64::from(spec.days + if self.is_leap(year) { spec.leap_days } else { 0 }) as i64;
            if day < length { return Ok(Date { year, month, day: day as u32 + 1 }); }
            day -= length;
        }
        Err("day outside calendar")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn calendar() -> CalendarSpec {
        CalendarSpec { id: "synthetic".into(), name: "Synthetic Era".into(), months: vec![Month { name: "Aster".into(), days: 30, leap_days: 1 }, Month { name: "Brume".into(), days: 30, leap_days: 0 }], leap: Some(LeapRule { every_years: 4, offset: 0, month: 0 }), eras: vec![Era { name: "Before Dawn".into(), start_year: -10 }, Era { name: "Dawn".into(), start_year: 1 }], week_length: 6, season_cycle_days: Some(60), moon_cycle_days: Some(15) }
    }

    #[test]
    fn round_trips_negative_years_and_leap_months() {
        let cal = calendar();
        for date in [Date { year: -3, month: 1, day: 20 }, Date { year: 0, month: 0, day: 31 }, Date { year: 4, month: 0, day: 31 }] {
            assert_eq!(cal.from_day(cal.to_day(date).unwrap()).unwrap(), date);
        }
    }
}
