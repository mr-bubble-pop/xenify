//! Editing together.
//!
//! The server keeps one order of changes per document and never looks
//! inside them (they may be encrypted). Each app keeps its own changes that
//! the server hasn't taken yet; when someone else's change arrives first,
//! the app adjusts its own to come after it ([`Transform`]) and adjusts the
//! incoming change to come after its own, so everyone ends up with the
//! same document.
//!
//! [`Client`] does the bookkeeping: one batch out at a time, the rest
//! waiting; incoming changes in server order (early arrivals wait their
//! turn); "behind" answers resent after adjusting.

use std::collections::BTreeMap;

/// A change that can be adjusted to come after another made at the same time.
pub trait Transform: Clone {
    /// `a` adjusted to apply after `b` (both made on the same document).
    /// `later`: in the final order `a` comes after `b`. Ties go by it (two
    /// inserts at one place: the earlier first; two settings of one value:
    /// the later wins). May give none (nothing left to do) or several.
    fn transform(a: &Self, b: &Self, later: bool) -> Vec<Self>;
}

/// `a` (later) and `b` (earlier) made on the same document: returns `a`
/// adjusted to follow `b`, and `b` adjusted to follow `a`.
pub fn transform<T: Transform>(a: &[T], b: &[T]) -> (Vec<T>, Vec<T>) {
    match (a.len(), b.len()) {
        (0, _) => (Vec::new(), b.to_vec()),
        (_, 0) => (a.to_vec(), Vec::new()),
        (1, 1) => (T::transform(&a[0], &b[0], true), T::transform(&b[0], &a[0], false)),
        (1, _) => {
            let (a1, b0) = transform(a, &b[..1]);
            let (a2, rest) = transform(&a1, &b[1..]);
            (a2, [b0, rest].concat())
        }
        _ => {
            let (a0, b1) = transform(&a[..1], b);
            let (rest, b2) = transform(&a[1..], &b1);
            ([a0, rest].concat(), b2)
        }
    }
}

/// Changes to send: made on top of server change `base`.
#[derive(Clone, Debug, PartialEq)]
pub struct Batch<T> {
    pub client: String,
    pub base: i64,
    pub ops: Vec<T>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// A change is missing and won't come: open the document again.
    Gap { have: i64, got: i64 },
}

/// One window's side of editing together.
#[derive(Clone, Debug)]
pub struct Client<T> {
    /// Server changes applied here.
    seq: i64,
    /// Sent, not answered yet: (batch id, the changes as they now stand).
    inflight: Option<(String, Vec<T>)>,
    /// Made here, not sent yet.
    pending: Vec<T>,
    /// Arrived early (ahead of a missing one, or of our own batch's answer).
    early: BTreeMap<i64, T>,
    name: String,
    batches: u64,
}

impl<T: Transform> Client<T> {
    /// After opening a document whose server changes go up to `seq`.
    /// `name` keeps this window's batch ids apart from others'.
    pub fn new(seq: i64, name: &str) -> Client<T> {
        Client { seq, inflight: None, pending: Vec::new(), early: BTreeMap::new(), name: name.into(), batches: 0 }
    }

    pub fn seq(&self) -> i64 {
        self.seq
    }

    /// Changes made here that the server hasn't taken yet.
    pub fn unsent(&self) -> usize {
        self.inflight.as_ref().map_or(0, |(_, o)| o.len()) + self.pending.len()
    }

    /// Whether everything made here is on the server.
    pub fn settled(&self) -> bool {
        self.unsent() == 0
    }

    fn batch(&mut self) -> Option<Batch<T>> {
        if self.inflight.is_some() || self.pending.is_empty() {
            return None;
        }
        self.batches += 1;
        let client = format!("{}-{}", self.name, self.batches);
        let ops = std::mem::take(&mut self.pending);
        self.inflight = Some((client.clone(), ops.clone()));
        Some(Batch { client, base: self.seq, ops })
    }

    /// Changes just made (and applied) here. Returns a batch to send now,
    /// if nothing else is out.
    pub fn local(&mut self, ops: Vec<T>) -> Option<Batch<T>> {
        self.pending.extend(ops);
        self.batch()
    }

    /// Someone else's change number `seq`. Returns what to apply here, in
    /// order (it may be nothing yet if earlier ones are still missing).
    pub fn remote(&mut self, seq: i64, op: T) -> Vec<T> {
        if seq <= self.seq {
            return Vec::new();
        }
        self.early.insert(seq, op);
        self.drain()
    }

    fn drain(&mut self) -> Vec<T> {
        let mut out = Vec::new();
        while let Some(op) = self.early.remove(&(self.seq + 1)) {
            self.seq += 1;
            let mut incoming = vec![op];
            if let Some((_, inflight)) = &mut self.inflight {
                let (mine, theirs) = transform(inflight, &incoming);
                *inflight = mine;
                incoming = theirs;
            }
            let (mine, theirs) = transform(&self.pending, &incoming);
            self.pending = mine;
            out.extend(theirs);
        }
        out
    }

    /// The server took our batch as changes `from..=to`. Returns what to
    /// apply here (early arrivals now in turn) and the next batch to send.
    pub fn accepted(&mut self, client: &str, from: i64, to: i64) -> Result<(Vec<T>, Option<Batch<T>>), Error> {
        if self.inflight.as_ref().is_none_or(|(c, _)| c != client) {
            return Ok((Vec::new(), None));
        }
        if from != self.seq + 1 {
            return Err(Error::Gap { have: self.seq, got: from });
        }
        self.inflight = None;
        self.seq = to;
        self.early.retain(|s, _| *s > to);
        let apply = self.drain();
        Ok((apply, self.batch()))
    }

    /// Our batch came too late; `missed` are the changes before it. Returns
    /// what to apply here and the batch to send again.
    pub fn behind(&mut self, client: &str, missed: Vec<(i64, T)>) -> (Vec<T>, Option<Batch<T>>) {
        let mut apply = Vec::new();
        for (seq, op) in missed {
            apply.extend(self.remote(seq, op));
        }
        if self.inflight.as_ref().is_some_and(|(c, _)| c == client) {
            let (_, ops) = self.inflight.take().unwrap();
            // Our batch goes first again, ahead of anything made since.
            let rest = std::mem::take(&mut self.pending);
            self.pending = ops;
            self.pending.extend(rest);
        }
        (apply, self.batch())
    }

    /// After reconnecting: the batch that was out goes again under the same
    /// id (the server answers `accepted` if it had already taken it), or the
    /// next one if none was out.
    pub fn resend(&mut self) -> Option<Batch<T>> {
        match &self.inflight {
            Some((client, ops)) => Some(Batch { client: client.clone(), base: self.seq, ops: ops.clone() }),
            None => self.batch(),
        }
    }
}

#[cfg(test)]
mod tests;
