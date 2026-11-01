//! Calendars for Xenify Mail: iCalendar reading and writing, repeats,
//! time zones and invitations.

#[cfg(feature = "net")]
pub mod caldav;
#[cfg(all(feature = "net", feature = "store"))]
pub mod davsync;
pub mod event;
pub mod expand;
pub mod ics;
pub mod itip;
#[cfg(feature = "store")]
pub mod store;
#[cfg(feature = "net")]
pub mod subscribe;
pub mod tz;
