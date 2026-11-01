//! How busy people are, week by week: the hours this plan books for them
//! (its schedule) and other plans' bookings, against the hours they can
//! work (their working day, holidays, time away).

use crate::model::{Plan, ResourceKind};
use crate::schedule::{Booking, Calendar, Schedule, World};
use serde::Serialize;
use xenify_holidays::Date;

#[derive(Clone, Debug, Serialize)]
pub struct Cell {
    /// Monday of the week.
    pub week: Date,
    /// Minutes booked (this plan and the others) and minutes they can work.
    pub booked: u32,
    pub have: u32,
    /// Days away (or on holiday) that week, when they would otherwise work.
    pub away: u32,
}

#[derive(Clone, Debug, Serialize)]
pub struct Row {
    pub id: String,
    pub name: String,
    /// Minutes in their working day.
    pub day: u32,
    pub cells: Vec<Cell>,
}

/// Every person this plan knows, `weeks` weeks from the week of `from`.
pub fn rows(plan: &Plan, world: &World, s: &Schedule, from: Date, weeks: u32) -> Vec<Row> {
    let cal = Calendar::new(plan, world);
    let monday = from.add(-(from.weekday() as i32));
    let mut ids: Vec<String> = Vec::new();
    for r in world.resources.iter().chain(plan.resources.iter()) {
        if r.kind == ResourceKind::Person && !ids.contains(&r.id) {
            ids.push(r.id.clone());
        }
    }
    let mut out = Vec::new();
    for id in ids {
        let Some(r) = cal.resource(&id) else { continue };
        let mine = s.booked.get(&id);
        let mut cells = Vec::new();
        for w in 0..weeks {
            let start = monday.add(7 * w as i32);
            let (mut booked, mut have, mut away) = (0u32, 0u32, 0u32);
            for k in 0..7 {
                let d = start.add(k);
                let m = cal.minutes(&id, d);
                if m == 0 {
                    if plan.settings.work_days[d.weekday() as usize] {
                        away += 1;
                    }
                    continue;
                }
                have += m;
                booked += mine.and_then(|x| x.get(&d)).copied().unwrap_or(0);
                let other: f64 = world
                    .bookings
                    .iter()
                    .chain(plan.frozen_bookings.iter())
                    .filter(|b| b.resource == id && b.from <= d && d <= b.to)
                    .map(|b| b.percent as f64 / 100.0)
                    .sum();
                booked += (other * m as f64).round() as u32;
            }
            cells.push(Cell { week: start, booked, have, away });
        }
        let day = if r.day_minutes > 0 { r.day_minutes } else { plan.settings.day_minutes };
        out.push(Row { id: id.clone(), name: r.name.clone(), day, cells });
    }
    out
}

#[derive(Clone, Debug, Serialize)]
pub struct WeekItem {
    /// This plan's issue (empty for another plan's booking).
    pub task: String,
    pub key: String,
    pub title: String,
    pub plan: String,
    pub from: Date,
    pub to: Date,
    pub minutes: u32,
}

/// What fills a person's week: this plan's issues and other plans' bookings.
pub fn week_detail(plan: &Plan, world: &World, s: &Schedule, id: &str, week: Date) -> Vec<WeekItem> {
    let end = week.add(6);
    let cal = Calendar::new(plan, world);
    let mut out = Vec::new();
    for tid in &plan.order {
        let Some(t) = plan.tasks.get(tid) else { continue };
        // Group work counts for the member who got it.
        let by = s.tasks.get(tid).and_then(|d| d.by.as_deref()).or(t.who.as_deref());
        if by != Some(id) {
            continue;
        }
        let Some(days) = s.work.get(tid) else { continue };
        let inside: Vec<(&Date, &u32)> = days.range(week..=end).collect();
        if inside.is_empty() {
            continue;
        }
        out.push(WeekItem {
            task: tid.clone(),
            key: t.key.clone(),
            title: t.title.clone(),
            plan: String::new(),
            from: *inside[0].0,
            to: *inside[inside.len() - 1].0,
            minutes: inside.iter().map(|(_, n)| **n).sum(),
        });
    }
    let others: Vec<&Booking> = world
        .bookings
        .iter()
        .chain(plan.frozen_bookings.iter())
        .filter(|b| b.resource == id && b.from <= end && week <= b.to)
        .collect();
    for b in others {
        let (a, z) = (b.from.max(week), b.to.min(end));
        let mut minutes = 0;
        let mut d = a;
        while d <= z {
            minutes += cal.minutes(id, d) * b.percent / 100;
            d = d.add(1);
        }
        out.push(WeekItem {
            task: String::new(),
            key: String::new(),
            title: b.title.clone(),
            plan: b.plan.clone(),
            from: a,
            to: z,
            minutes,
        });
    }
    out
}
