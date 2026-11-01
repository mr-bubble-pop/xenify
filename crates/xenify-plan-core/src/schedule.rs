//! Working out every issue's dates from what people chose: estimates, the
//! backlog's order, "waits for" links, who does it, held-back starts,
//! working days, public holidays, company days and time away.
//!
//! Nothing here is stored: every copy of a plan works the same dates out.

use crate::model::{Id, Outside, Plan, Resource, ResourceKind, Task, is_outside};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use xenify_holidays::{Date, Days};

/// What the schedule needs from outside the plan file.
#[derive(Clone, Default)]
pub struct World {
    pub today: Date,
    /// Company days off: first, last, name.
    pub company_days: Vec<(Date, Date, String)>,
    /// People, groups and things from the server's register (they replace
    /// the plan's own copies with the same id).
    pub resources: Vec<Resource>,
    /// Other plans' bookings (Resources adds them up).
    pub bookings: Vec<Booking>,
    /// Issues in other plans this one waits for, from the server
    /// ("n<plan>:<issue>"; "n<plan>:*" is a whole plan, start to finish).
    pub outside: HashMap<Id, Outside>,
    /// Holiday calendars, by "country/region" (kept between schedules).
    pub days: Arc<std::sync::Mutex<HashMap<String, Arc<Days>>>>,
    /// The server's holidays (Admin › Holidays), for plans that chose none.
    pub default_country: String,
    pub default_region: String,
}

/// Someone (or something) booked for a stretch of days at a share of their time.
#[derive(Clone, Debug, PartialEq, Serialize, serde::Deserialize)]
pub struct Booking {
    pub resource: Id,
    /// The plan (file) and task.
    #[serde(default)]
    pub plan: String,
    pub task: Id,
    #[serde(default)]
    pub title: String,
    pub from: Date,
    pub to: Date,
    pub percent: u32,
    /// Its plan goes first (made earlier): this plan works around it. A
    /// plan made later doesn't push this one's work back (it works around
    /// this one instead), so two plans never keep moving each other; it
    /// still counts in People.
    #[serde(default = "first")]
    pub first: bool,
    /// Its issue is set to go ahead anyway (Task.overbook): it doesn't
    /// wait for the plans before it, so they warn about it too.
    #[serde(default)]
    pub over: bool,
}

/// Someone's time this issue meets in another plan: what made it wait, or
/// (over-booked) what it now overlaps.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Clash {
    pub who: String,
    /// The other plan's issue (its pieces are one clash).
    #[serde(skip)]
    pub task: Id,
    pub plan: String,
    pub title: String,
    pub from: Date,
    pub to: Date,
    /// Both at once (one of the two is set to go ahead anyway).
    pub over: bool,
}

fn first() -> bool {
    true
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Dates {
    pub start: Date,
    pub finish: Date,
    /// Stretches worked on: more than one when time off splits the work.
    pub pieces: Vec<(Date, Date)>,
    /// On the Timeline: it has an estimate (or is a milestone, or an epic
    /// with something placed). Not placed: no dates to show.
    pub placed: bool,
    /// An epic: spans its issues.
    pub summary: bool,
    pub milestone: bool,
    /// Finishes after its due date, by this many working days.
    pub late: bool,
    pub late_by: u32,
    /// What it waits for that isn't on the Timeline yet.
    pub waits: Vec<Id>,
    /// Working days from start to finish.
    pub days: u32,
    /// Percent done (epics: weighted by work).
    pub done: u32,
    /// The estimate in minutes (epics: their issues' total).
    pub work: u32,
    /// Who does it: the issue's person, or for an issue given to a group,
    /// the member who can finish it first.
    pub by: Option<Id>,
    pub warnings: Vec<String>,
    /// Its person's time taken in other plans while it's waiting or worked on.
    pub clashes: Vec<Clash>,
}

pub struct Calendar<'a> {
    plan: &'a Plan,
    world: &'a World,
    plan_days: Arc<Days>,
    resources: HashMap<&'a str, &'a Resource>,
}

impl<'a> Calendar<'a> {
    pub fn new(plan: &'a Plan, world: &'a World) -> Calendar<'a> {
        let (country, region) = if plan.settings.country.is_empty() {
            (world.default_country.as_str(), world.default_region.as_str())
        } else {
            (plan.settings.country.as_str(), plan.settings.region.as_str())
        };
        let plan_days = days_for(world, country, region);
        let mut resources: HashMap<&str, &Resource> = plan.resources.iter().map(|r| (r.id.as_str(), r)).collect();
        for r in &world.resources {
            resources.insert(r.id.as_str(), r);
        }
        Calendar { plan, world, plan_days, resources }
    }

    pub fn resource(&self, id: &str) -> Option<&'a Resource> {
        self.resources.get(id).copied()
    }

    fn company(&self, d: Date) -> bool {
        self.world.company_days.iter().chain(self.plan.frozen_days.iter()).any(|(a, b, _)| *a <= d && d <= *b)
    }

    /// A working day for the plan as a whole.
    pub fn working(&self, d: Date) -> bool {
        self.plan.settings.work_days[d.weekday() as usize] && !self.plan_days.is_holiday(d) && !self.company(d)
    }

    /// Why `d` isn't a working day ("Christmas Day"), if it isn't.
    pub fn day_off(&self, d: Date) -> Option<String> {
        if let Some((_, _, n)) =
            self.world.company_days.iter().chain(self.plan.frozen_days.iter()).find(|(a, b, _)| *a <= d && d <= *b)
        {
            return Some(n.clone());
        }
        self.plan_days.name(d).filter(|_| self.plan.settings.work_days[d.weekday() as usize])
    }

    /// `d`, or the first working day after it.
    pub fn on_or_after(&self, mut d: Date) -> Date {
        for _ in 0..3660 {
            if self.working(d) {
                return d;
            }
            d = d.add(1);
        }
        d
    }

    pub fn on_or_before(&self, mut d: Date) -> Date {
        for _ in 0..3660 {
            if self.working(d) {
                return d;
            }
            d = d.add(-1);
        }
        d
    }

    /// `n` working days after (or before, n < 0) the working day `d`.
    pub fn add(&self, d: Date, n: i32) -> Date {
        let step = if n < 0 { -1 } else { 1 };
        let mut d = if n < 0 { self.on_or_before(d) } else { self.on_or_after(d) };
        let mut left = n.abs();
        let mut guard = 0;
        while left > 0 && guard < 20000 {
            d = d.add(step);
            if self.working(d) {
                left -= 1;
            }
            guard += 1;
        }
        d
    }

    /// Working days from `a` to `b`, counting both ends (0 if b < a).
    pub fn count(&self, a: Date, b: Date) -> u32 {
        if b < a {
            return 0;
        }
        let mut n = 0;
        let mut d = a;
        while d <= b && n < 20000 {
            if self.working(d) {
                n += 1;
            }
            d = d.add(1);
        }
        n
    }

    /// Working days to move from `a` to `b` (negative: earlier).
    pub fn between(&self, a: Date, b: Date) -> i32 {
        if b >= a { self.count(a.add(1), b) as i32 } else { -(self.count(b, a.add(-1)) as i32) }
    }

    /// Minutes someone can work on `d` (0 for things).
    pub fn minutes(&self, id: &str, d: Date) -> u32 {
        let Some(r) = self.resource(id) else { return 0 };
        match r.kind {
            ResourceKind::Thing => 0,
            ResourceKind::Group => r
                .members
                .iter()
                .filter(|m| m.from.is_none_or(|f| f <= d) && m.to.is_none_or(|t| d <= t))
                .map(|m| self.minutes(&m.person, d))
                .sum(),
            ResourceKind::Person => {
                if !self.plan.settings.work_days[d.weekday() as usize] || self.company(d) {
                    return 0;
                }
                if r.away.iter().any(|(a, b)| *a <= d && d <= *b) {
                    return 0;
                }
                let own = self.plan.settings.own_regions && !r.country.is_empty();
                let off = if own {
                    days_for(self.world, &r.country, &r.region).is_holiday(d)
                } else {
                    self.plan_days.is_holiday(d)
                };
                if off {
                    return 0;
                }
                if r.day_minutes > 0 { r.day_minutes } else { self.plan.settings.day_minutes }
            }
        }
    }
}

fn days_for(world: &World, country: &str, region: &str) -> Arc<Days> {
    let key = format!("{country}/{region}");
    let mut m = world.days.lock().unwrap_or_else(|e| e.into_inner());
    m.entry(key).or_insert_with(|| Arc::new(Days::new(country, region))).clone()
}

/// Minutes booked on each day.
type Booked = Vec<(Date, u32)>;

pub struct Schedule {
    pub tasks: BTreeMap<Id, Dates>,
    /// Minutes each person is booked for in this plan, by day.
    pub booked: BTreeMap<Id, BTreeMap<Date, u32>>,
    /// Minutes of each issue done on each day.
    pub work: BTreeMap<Id, BTreeMap<Date, u32>>,
}

/// What issue `id` waits for, its epics' links included (issues here by id,
/// in other plans as their references): what the server follows to find
/// loops across plans.
pub fn waits_of(plan: &Plan, id: &str) -> Vec<String> {
    Waits::new(plan).of(id)
}

/// [`waits_of`] for many issues of one plan: the plan's breakdown is
/// looked at once, not once per issue (big plans would take long).
pub struct Waits<'p>(Tree<'p>);

impl<'p> Waits<'p> {
    pub fn new(plan: &'p Plan) -> Self {
        Waits(Tree::new(plan))
    }

    pub fn of(&self, id: &str) -> Vec<String> {
        self.0.plan.tasks.get(id).map(|t| self.0.waits_for(t).into_iter().map(|(k, _)| k).collect()).unwrap_or_default()
    }
}

/// A plan's breakdown, worked out once: which tasks have others inside
/// them, each one's children, and where each is in the order.
pub(crate) struct Tree<'p> {
    plan: &'p Plan,
    /// Tasks some task is inside.
    parents: HashSet<&'p str>,
    /// Each task's children (all of them, in no particular order).
    down: HashMap<&'p str, Vec<&'p str>>,
    /// Each task's children in the plan's order (as `Plan::children`).
    kids: HashMap<&'p str, Vec<&'p Task>>,
    /// Where each id is in the plan's order.
    at: HashMap<&'p str, Vec<usize>>,
}

impl<'p> Tree<'p> {
    pub(crate) fn new(plan: &'p Plan) -> Self {
        let mut parents = HashSet::new();
        let mut down: HashMap<&str, Vec<&str>> = HashMap::new();
        for (k, t) in &plan.tasks {
            if let Some(p) = t.parent.as_deref() {
                parents.insert(p);
                down.entry(p).or_default().push(k.as_str());
            }
        }
        let mut kids: HashMap<&str, Vec<&Task>> = HashMap::new();
        let mut at: HashMap<&str, Vec<usize>> = HashMap::new();
        for (i, id) in plan.order.iter().enumerate() {
            at.entry(id.as_str()).or_default().push(i);
            if let Some(t) = plan.tasks.get(id)
                && let Some(p) = t.parent.as_deref()
            {
                kids.entry(p).or_default().push(t);
            }
        }
        Tree { plan, parents, down, kids, at }
    }

    /// `Plan::is_epic`, without looking through every task.
    pub(crate) fn is_epic(&self, t: &Task) -> bool {
        t.kind == "epic" || self.parents.contains(t.id.as_str())
    }

    pub(crate) fn children(&self, id: &str) -> impl Iterator<Item = &'p Task> + '_ {
        self.kids.get(id).into_iter().flatten().copied()
    }

    /// The ids in the plan's order that are inside `x` (`Plan::within`, so
    /// at most 64 levels down), `x` left out, in that order.
    fn under(&self, x: &str) -> Vec<&'p str> {
        let mut seen: HashSet<&str> = HashSet::new();
        let mut level: Vec<&str> = vec![x];
        seen.insert(x);
        for _ in 0..64 {
            let mut next = Vec::new();
            for p in level {
                for &k in self.down.get(p).into_iter().flatten() {
                    if seen.insert(k) {
                        next.push(k);
                    }
                }
            }
            if next.is_empty() {
                break;
            }
            level = next;
        }
        seen.remove(x);
        let mut places: Vec<usize> = seen.iter().filter_map(|k| self.at.get(k)).flatten().copied().collect();
        places.sort_unstable();
        places.into_iter().map(|i| self.plan.order[i].as_str()).collect()
    }

    /// An issue's links as the issues it waits for: its own and its epics'
    /// (what an epic waits for, everything in it waits for); a link to an
    /// epic is to everything in it.
    pub(crate) fn waits_for(&self, t: &Task) -> Vec<(String, i32)> {
        let plan = self.plan;
        let mut out = Vec::new();
        let mut at = Some(t);
        let mut depth = 0;
        while let Some(e) = at {
            // An epic waiting for something inside itself holds nothing up.
            let inside = |k: &str| e.id != t.id && plan.within(k, &e.id);
            for d in &e.after {
                if is_outside(&d.task) {
                    out.push((d.task.clone(), d.lag));
                    continue;
                }
                let Some(x) = plan.tasks.get(&d.task) else { continue };
                if self.is_epic(x) {
                    for k in self.under(&x.id).into_iter().filter(|k| !inside(k)) {
                        if plan.tasks.get(k).is_some_and(|y| !self.is_epic(y)) {
                            out.push((k.to_string(), d.lag));
                        }
                    }
                } else if !inside(&x.id) {
                    out.push((x.id.clone(), d.lag));
                }
            }
            depth += 1;
            at = e.parent.as_deref().and_then(|p| plan.tasks.get(p)).filter(|_| depth < 64);
        }
        out
    }
}

/// Backlog order, with whatever an issue waits for first (depth first,
/// each issue's links in their order). It keeps its own stack: a chain of
/// many thousands of links would overflow the program's.
fn wait_order<'p>(plan: &'p Plan, issues: &[&'p Task], deps: &HashMap<&str, Vec<(String, i32)>>) -> Vec<&'p str> {
    let mut order: Vec<&str> = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();
    let mut on_stack: HashSet<&str> = HashSet::new();
    // Each issue being looked at, and how many of its links are done.
    let mut stack: Vec<(&'p str, usize)> = Vec::new();
    for t in issues {
        let start = t.id.as_str();
        if seen.contains(start) {
            continue;
        }
        on_stack.insert(start);
        stack.push((start, 0));
        while let Some((id, i)) = stack.last_mut() {
            let links = deps.get(*id).map(Vec::as_slice).unwrap_or(&[]);
            let next = links[*i..].iter().position(|(d, _)| plan.tasks.contains_key(d)).map(|j| *i + j);
            match next {
                Some(j) => {
                    *i = j + 1;
                    let (k, _) = plan.tasks.get_key_value(&links[j].0).expect("checked above");
                    let k = k.as_str();
                    // Done already, or a loop back to one being looked at.
                    if !seen.contains(k) && on_stack.insert(k) {
                        stack.push((k, 0));
                    }
                }
                None => {
                    let id = *id;
                    stack.pop();
                    on_stack.remove(id);
                    if seen.insert(id) {
                        order.push(id);
                    }
                }
            }
        }
    }
    order
}

/// Every issue's dates. Issues are taken in the backlog's order (the plan's
/// order is the priority), each after everything it waits for: it starts
/// when those are finished and its person has time, and its estimate fills
/// that person's working hours from there. Time off can split the work into
/// pieces; other work can't (people finish one thing before the next), so
/// an issue only goes into a gap between other work when it fits whole.
/// Nothing not yet done is planned before today.
pub fn schedule(plan: &Plan, world: &World) -> Schedule {
    let cal = Calendar::new(plan, world);
    let today = world.today;
    let tree = Tree::new(plan);
    let issues: Vec<&Task> =
        plan.order.iter().filter_map(|id| plan.tasks.get(id)).filter(|t| !tree.is_epic(t)).collect();
    let deps: HashMap<&str, Vec<(String, i32)>> = issues
        .iter()
        .map(|t| {
            // A mirror waits for nothing: it only shows another's dates.
            let w = if t.mirror.is_some() { Vec::new() } else { tree.waits_for(t) };
            (t.id.as_str(), w)
        })
        .collect();

    // Backlog order, with whatever an issue waits for first.
    let order = wait_order(plan, &issues, &deps);

    // Other plans' bookings: share of a person's day already taken.
    let other = |who: &str, d: Date| -> f64 {
        world
            .bookings
            .iter()
            .chain(plan.frozen_bookings.iter())
            .filter(|b| b.first && b.resource == who && b.from <= d && d <= b.to)
            .map(|b| b.percent as f64 / 100.0)
            .sum()
    };
    let mut booked: BTreeMap<Id, BTreeMap<Date, u32>> = BTreeMap::new();
    let mut work: BTreeMap<Id, BTreeMap<Date, u32>> = BTreeMap::new();
    let mut out: BTreeMap<Id, Dates> = BTreeMap::new();
    for id in order {
        let t = &plan.tasks[id];
        let ms = plan.is_milestone(t);
        let est = if ms { 0 } else { t.work.unwrap_or(0) };
        let mut dates = Dates {
            milestone: ms,
            work: est,
            done: if plan.is_done(t) { 100 } else { t.done.min(100) },
            ..Dates::default()
        };
        if let Some(r) = &t.mirror {
            // A mirror: its dates are the other one's, as the server has them.
            dates.work = 0;
            match world.outside.get(r).or_else(|| plan.frozen.get(r)) {
                Some(x) if x.gone => dates.warnings.push("What it mirrors isn’t there any more".into()),
                Some(x) => {
                    dates.done = if x.done { 100 } else { 0 };
                    if let (true, Some(s), Some(f)) = (x.placed, x.start, x.finish) {
                        dates.start = s;
                        dates.finish = f.max(s);
                        dates.pieces = vec![(s, dates.finish)];
                        dates.placed = true;
                        dates.days = cal.count(s, dates.finish);
                        dates.summary = true;
                    }
                }
                None => {}
            }
            out.insert(id.to_string(), dates);
            continue;
        }
        if plan.is_done(t) {
            // Done: where it was worked on, when known.
            if let Some(s) = t.started.or(t.finished) {
                let f = t.finished.unwrap_or(today).max(s);
                dates.start = s;
                dates.finish = f;
                dates.pieces = vec![(s, f)];
                dates.placed = true;
                dates.days = cal.count(s, f);
            }
            out.insert(id.to_string(), dates);
            continue;
        }
        if !ms && t.work.is_none() {
            out.insert(id.to_string(), dates);
            continue;
        }
        // The earliest day the links allow.
        let mut earliest = today;
        for (d, lag) in deps.get(id).map(Vec::as_slice).unwrap_or(&[]) {
            if is_outside(d) {
                match world.outside.get(d).or_else(|| plan.frozen.get(d)) {
                    Some(x) if x.gone => dates.warnings.push(format!(
                        "Starts after {} in another plan, which isn’t there any more",
                        if x.key.is_empty() { "an issue" } else { &x.key }
                    )),
                    Some(x) if x.looped => dates.warnings.push(format!(
                        "Starts after {}, which itself waits for this plan: a loop, so that link is left out until one is removed",
                        if d.ends_with(":*") {
                            format!("the plan “{}”", if x.plan.is_empty() { &x.title } else { &x.plan })
                        } else if x.key.is_empty() {
                            "an issue in another plan".to_string()
                        } else {
                            x.key.clone()
                        }
                    )),
                    Some(Outside { placed: true, finish: Some(f), milestone, .. }) => {
                        let next = if *milestone { *f } else { cal.add(*f, 1) };
                        earliest = earliest.max(if *lag != 0 { cal.add(next, *lag) } else { next });
                    }
                    Some(x) if x.done => {}
                    Some(_) => dates.waits.push(d.clone()),
                    None => dates.waits.push(d.clone()),
                }
                continue;
            }
            match out.get(d) {
                Some(x) if x.placed => {
                    let next = if x.milestone { x.finish } else { cal.add(x.finish, 1) };
                    earliest = earliest.max(if *lag != 0 { cal.add(next, *lag) } else { next });
                }
                Some(_) => dates.waits.push(d.clone()),
                None => dates.warnings.push("Part of a loop of links".into()),
            }
        }
        // Started already, yet what it waits for isn't done: say so (a link
        // the wrong way round, usually) instead of drawing it from the start.
        let waits_still = earliest > today;
        if waits_still && plan.is_started(t) {
            dates.warnings.push("It’s started, but what it starts after isn’t done yet: is the link the right way round?".into());
        }
        if let Some(pin) = t.start {
            earliest = earliest.max(pin);
        }
        if ms {
            let d = cal.on_or_after(earliest);
            dates.start = d;
            dates.finish = d;
            dates.placed = true;
        } else {
            let left =
                if plan.is_started(t) { est as u64 * (100 - dates.done.min(99)) as u64 / 100 } else { est as u64 }
                    as u32;
            // Who could do it: its person, or every member of its group
            // (the one who'd finish first gets it); nobody: the plan's days.
            let candidates: Vec<Option<&str>> = match t.who.as_deref().and_then(|w| cal.resource(w)) {
                Some(r) if r.kind == ResourceKind::Person => vec![Some(r.id.as_str())],
                Some(r) if r.kind == ResourceKind::Group => {
                    let mut v: Vec<Option<&str>> = Vec::new();
                    for m in &r.members {
                        if cal.resource(&m.person).is_some_and(|p| p.kind == ResourceKind::Person)
                            && !v.contains(&Some(m.person.as_str()))
                        {
                            v.push(Some(m.person.as_str()));
                        }
                    }
                    if v.is_empty() {
                        dates.warnings.push(format!("Nobody is in {}", r.name));
                        vec![None]
                    } else {
                        v
                    }
                }
                _ => vec![None],
            };
            let group = t.who.as_deref().and_then(|w| cal.resource(w)).filter(|r| r.kind == ResourceKind::Group);
            let place = |person: Option<&str>| -> Option<Booked> {
                let mine = person.map(|p| booked.get(p).cloned().unwrap_or_default()).unwrap_or_default();
                let cap = |d: Date| -> u32 {
                    match person {
                        Some(p) => {
                            // In a group: only while they're a member.
                            if let Some(g) = group
                                && !g.members.iter().any(|m| {
                                    m.person == p && m.from.is_none_or(|f| f <= d) && m.to.is_none_or(|x| d <= x)
                                })
                            {
                                return 0;
                            }
                            let m = cal.minutes(p, d);
                            // Set to go ahead anyway: other plans don't hold it back.
                            if t.overbook {
                                return m;
                            }
                            (m as f64 * (1.0 - other(p, d)).max(0.0)) as u32
                        }
                        None => {
                            if cal.working(d) {
                                plan.settings.day_minutes
                            } else {
                                0
                            }
                        }
                    }
                };
                // From `from`: the days and minutes; Err(true) when other work would
                // split it, Err(false) when there isn't the time at all (no later
                // start has more).
                let fill = |from: Date| -> Result<Booked, bool> {
                    let mut need = left.max(1);
                    let mut d = from;
                    let mut got: Vec<(Date, u32)> = Vec::new();
                    let mut guard = 0;
                    while need > 0 && guard < 2000 {
                        let c = cap(d);
                        if c > 0 {
                            let free = c.saturating_sub(*mine.get(&d).unwrap_or(&0));
                            if free == 0 {
                                if !got.is_empty() {
                                    return Err(true);
                                }
                            } else {
                                let n = free.min(need);
                                got.push((d, n));
                                need -= n;
                            }
                        }
                        d = d.add(1);
                        guard += 1;
                    }
                    if need > 0 { Err(false) } else { Ok(got) }
                };
                let mut from = earliest;
                for _ in 0..800 {
                    match fill(from) {
                        Ok(g) => return Some(g),
                        Err(true) => from = from.add(1),
                        Err(false) => return None,
                    }
                }
                None
            };
            // The earliest finish wins; a tie goes to the one listed first.
            let mut best: Option<(Option<&str>, Booked)> = None;
            for c in &candidates {
                if let Some(g) = place(*c) {
                    let end = g.last().map(|x| x.0);
                    if best.as_ref().is_none_or(|(_, b)| end < b.last().map(|x| x.0)) {
                        best = Some((*c, g));
                    }
                }
            }
            let Some((person, got)) = best else {
                dates.warnings.push("Nobody has time for this work".into());
                out.insert(id.to_string(), dates);
                continue;
            };
            dates.by = person.map(str::to_string);
            if let Some(p) = person {
                let m = booked.entry(p.to_string()).or_default();
                for (d, n) in &got {
                    *m.entry(*d).or_insert(0) += n;
                }
            }
            work.insert(id.to_string(), got.iter().cloned().collect());
            // A new piece wherever a working day of the plan's was skipped.
            let mut pieces: Vec<(Date, Date)> = Vec::new();
            for (d, _) in &got {
                match pieces.last_mut() {
                    Some(last) if cal.count(last.1.add(1), d.add(-1)) == 0 => last.1 = *d,
                    _ => pieces.push((*d, *d)),
                }
            }
            if let (Some(s), Some(first)) = (t.started.filter(|_| plan.is_started(t) && !waits_still), pieces.first_mut())
                && s < first.0
            {
                first.0 = s;
            }
            dates.start = pieces.first().map(|p| p.0).unwrap_or(earliest);
            dates.finish = pieces.last().map(|p| p.1).unwrap_or(earliest);
            dates.pieces = pieces;
            dates.placed = true;
            dates.days = cal.count(dates.start, dates.finish);
            if let Some(r) = person.and_then(|p| cal.resource(p))
                && let Some((a, b)) = r.away.iter().find(|(a, b)| *a <= dates.finish && dates.start <= *b)
            {
                dates.warnings.push(format!("{} is away {} – {}", r.name, short(*a), short(*b)));
            }
            if let Some(p) = person {
                clashes(&cal, world, plan, t, p, earliest, &mut dates);
            }
        }
        if let Some(due) = t.due
            && dates.placed
            && dates.finish > due
        {
            dates.late = true;
            dates.late_by = cal.count(due.add(1), dates.finish).max(1);
        }
        out.insert(id.to_string(), dates);
    }

    // Epics span their issues (deepest first).
    let mut epics: Vec<&Task> =
        plan.order.iter().filter_map(|id| plan.tasks.get(id)).filter(|t| tree.is_epic(t)).collect();
    epics.sort_by_key(|t| std::cmp::Reverse(plan.depth(&t.id)));
    for e in epics {
        let kids: Vec<&Dates> = tree.children(&e.id).filter_map(|c| out.get(&c.id)).collect();
        let placed: Vec<&&Dates> = kids.iter().filter(|d| d.placed).collect();
        let work_total: u32 = kids.iter().map(|d| d.work).sum();
        let done = if work_total > 0 {
            (kids.iter().map(|d| d.work as u64 * d.done as u64).sum::<u64>() / work_total as u64) as u32
        } else {
            0
        };
        let mut d = Dates { summary: true, work: work_total, done, ..Dates::default() };
        if !placed.is_empty() {
            d.start = placed.iter().map(|x| x.start).min().unwrap();
            d.finish = placed.iter().map(|x| x.finish).max().unwrap();
            d.placed = true;
            d.days = cal.count(d.start, d.finish);
            d.late = placed.iter().any(|x| x.late);
        }
        out.insert(e.id.clone(), d);
    }
    Schedule { tasks: out, booked, work }
}

/// Other plans' bookings of `p` that `t` meets: those before it that it
/// waited for (from `earliest`), or overlaps when set to go ahead anyway,
/// and later plans' issues set to go ahead over this one.
fn clashes(cal: &Calendar, world: &World, plan: &Plan, t: &Task, p: &str, earliest: Date, dates: &mut Dates) {
    let name = cal.resource(p).map(|r| r.name.clone()).unwrap_or_else(|| "They".into());
    let from = if t.overbook { dates.start } else { earliest.min(dates.start) };
    let mut found: Vec<Clash> = Vec::new();
    for b in world.bookings.iter().chain(plan.frozen_bookings.iter()) {
        if b.resource != p || b.to < from || b.from > dates.finish {
            continue;
        }
        let over = if b.first { t.overbook } else { b.over && b.to >= dates.start };
        if !b.first && !over {
            continue;
        }
        // One issue's pieces (split by weekends or time away) are one clash.
        match found.iter_mut().find(|c| c.task == b.task && c.plan == b.plan && c.over == over) {
            Some(c) => {
                c.from = c.from.min(b.from);
                c.to = c.to.max(b.to);
            }
            None => found.push(Clash {
                who: name.clone(),
                task: b.task.clone(),
                plan: b.plan.clone(),
                title: b.title.clone(),
                from: b.from,
                to: b.to,
                over,
            }),
        }
    }
    for c in found.iter().filter(|c| c.over) {
        dates.warnings.push(format!(
            "Over-booked: {name} is also booked in {}{} {} – {}",
            if c.plan.is_empty() { "another plan".to_string() } else { format!("“{}”", c.plan) },
            if c.title.is_empty() { String::new() } else { format!(" ({})", c.title) },
            short(c.from.max(dates.start)),
            short(c.to.min(dates.finish))
        ));
    }
    dates.clashes = found;
}

/// "21 Dec".
pub fn short(d: Date) -> String {
    let (_, m, day) = d.ymd();
    const M: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    format!("{day} {}", M[(m - 1) as usize])
}

/// "Mon 16 Nov".
pub fn day_name(d: Date) -> String {
    const W: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    format!("{} {}", W[d.weekday() as usize], short(d))
}

/// Everyone booked in this plan (for the other plans and the server's
/// register): one booking per piece of work, at the share of their day
/// (their own working day: other plans take that share of it).
pub fn bookings(plan: &Plan, world: &World, s: &Schedule) -> Vec<Booking> {
    let cal = Calendar::new(plan, world);
    let mut out = Vec::new();
    for t in plan.tasks.values() {
        // Group work is booked to the member who got it.
        let Some(d) = s.tasks.get(&t.id) else { continue };
        let Some(who) = d.by.as_ref().or(t.who.as_ref()) else { continue };
        if d.summary || !d.placed || plan.is_done(t) {
            continue;
        }
        let Some(days) = s.work.get(&t.id) else { continue };
        // Only the days it's worked on: a started issue's piece reaches back
        // to when it started, but nobody is booked for it before today.
        for (a, b) in &d.pieces {
            let worked: Vec<(&Date, &u32)> = days.range(*a..=*b).collect();
            let (Some(first), Some(last)) = (worked.first(), worked.last()) else { continue };
            let mins: u32 = worked.iter().map(|(_, n)| **n).sum();
            let have: u32 = worked
                .iter()
                .map(|(d, _)| match cal.minutes(who, **d) {
                    0 => plan.settings.day_minutes,
                    m => m,
                })
                .sum();
            let percent = (mins * 100 / have.max(1)).clamp(1, 100);
            out.push(Booking {
                resource: who.clone(),
                plan: String::new(),
                task: t.id.clone(),
                title: t.title.clone(),
                from: *first.0,
                to: *last.0,
                percent,
                first: true,
                over: t.overbook,
            });
        }
    }
    out.sort_by(|a, b| (&a.resource, a.from, &a.task).cmp(&(&b.resource, b.from, &b.task)));
    out
}

#[cfg(test)]
mod order_tests {
    use super::*;
    use crate::model::Dep;

    /// The order as it was worked out before (by recursion), to compare.
    fn old_order<'p>(plan: &'p Plan, issues: &[&'p Task], deps: &HashMap<&str, Vec<(String, i32)>>) -> Vec<&'p str> {
        let mut order: Vec<&str> = Vec::new();
        let mut seen: HashSet<&str> = HashSet::new();
        fn visit<'p>(
            id: &'p str,
            plan: &'p Plan,
            deps: &HashMap<&str, Vec<(String, i32)>>,
            seen: &mut HashSet<&'p str>,
            stack: &mut HashSet<String>,
            order: &mut Vec<&'p str>,
        ) {
            if seen.contains(id) || !stack.insert(id.to_string()) {
                return;
            }
            for (d, _) in deps.get(id).map(Vec::as_slice).unwrap_or(&[]) {
                if let Some((k, _)) = plan.tasks.get_key_value(d) {
                    visit(k.as_str(), plan, deps, seen, stack, order);
                }
            }
            stack.remove(id);
            if seen.insert(id) {
                order.push(id);
            }
        }
        for t in issues {
            visit(t.id.as_str(), plan, deps, &mut seen, &mut HashSet::new(), &mut order);
        }
        order
    }

    fn plan(n: usize, links: impl Fn(usize) -> Vec<usize>) -> Plan {
        let mut p = Plan::default();
        for i in 0..n {
            let id = format!("t{i}");
            let after = links(i).into_iter().map(|j| Dep { task: format!("t{j}"), ..Default::default() }).collect();
            p.tasks.insert(id.clone(), Task { id: id.clone(), after, ..Default::default() });
            p.order.push(id);
        }
        p
    }

    fn both(p: &Plan) -> (Vec<String>, Vec<String>) {
        let tree = Tree::new(p);
        let issues: Vec<&Task> = p.order.iter().filter_map(|id| p.tasks.get(id)).collect();
        let deps: HashMap<&str, Vec<(String, i32)>> = issues.iter().map(|t| (t.id.as_str(), tree.waits_for(t))).collect();
        let new = wait_order(p, &issues, &deps).into_iter().map(String::from).collect();
        let old = old_order(p, &issues, &deps).into_iter().map(String::from).collect();
        (new, old)
    }

    #[test]
    fn same_order_as_before() {
        // Links forwards, backwards, missing and in loops.
        let mut seed = 7u64;
        let mut rnd = move |m: usize| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) as usize % m
        };
        let links: Vec<Vec<usize>> = (0..400).map(|_| (0..rnd(4)).map(|_| rnd(420)).collect()).collect();
        let p = plan(400, |i| links[i].clone());
        let (new, old) = both(&p);
        assert_eq!(new, old);
        assert_eq!(new.len(), 400);
    }

    #[test]
    fn a_long_chain_of_links_is_fine() {
        // Each waits for the one after it: 100,000 deep.
        let n = 100_000;
        let p = plan(n, |i| if i + 1 < n { vec![i + 1] } else { vec![] });
        let tree = Tree::new(&p);
        let issues: Vec<&Task> = p.order.iter().filter_map(|id| p.tasks.get(id)).collect();
        let deps: HashMap<&str, Vec<(String, i32)>> = issues.iter().map(|t| (t.id.as_str(), tree.waits_for(t))).collect();
        let order = wait_order(&p, &issues, &deps);
        assert_eq!(order.len(), n);
        assert_eq!(order[0], format!("t{}", n - 1));
    }

    #[test]
    fn epics_links_reach_what_is_in_them() {
        let mut p = plan(6, |i| if i == 5 { vec![0] } else { vec![] });
        for k in ["t1", "t2"] {
            p.tasks.get_mut(k).unwrap().parent = Some("t0".into());
        }
        p.tasks.get_mut("t3").unwrap().parent = Some("t1".into());
        let tree = Tree::new(&p);
        let t5 = &p.tasks["t5"];
        // t0 is an epic: t5 waits for the issues in it (t1 is an epic too).
        let got: Vec<String> = tree.waits_for(t5).into_iter().map(|(k, _)| k).collect();
        assert_eq!(got, ["t2", "t3"]);
        assert!(tree.is_epic(&p.tasks["t1"]) && !tree.is_epic(&p.tasks["t2"]));
        assert_eq!(tree.children("t0").map(|t| t.id.as_str()).collect::<Vec<_>>(), ["t1", "t2"]);
    }
}
