//! Send-to-Kindle: mail one book to a device's Amazon address, or hand it out
//! as the file a Kindle mounted over USB can open.
//!
//! A Kindle speaks no sync protocol. Amazon's *Send to Kindle* service takes
//! an e-mail from an approved sender with the book attached and delivers it to
//! the device that owns the recipient address, so a Kindle needs no
//! credential, no session and no endpoint — only an address, kept on the
//! device row as `devices.kindle_email` by
//! [`stump_devices`](stump_devices::kindle).
//!
//! Two lanes, one format policy:
//!
//! - **E-mail** ([`KindleSend::send_to_kindle`]). Amazon accepts an EPUB and
//!   converts it itself, so an unconverted EPUB is a *successful* send and the
//!   reason it went out unconverted is reported as a note, never an error.
//!   AZW3 is preferred when the operator's `boko` is present because it *is*
//!   KF8: nothing depends on Amazon's conversion.
//! - **USB** ([`prepare_file`] with [`FormatPolicy::RequireKindle`]). A Kindle
//!   mounted as a disk has no gateway to convert anything, so an EPUB that
//!   cannot be converted is refused instead of copied onto a device that
//!   cannot open it.
//!
//! Every completed lane writes a `kindle_deliveries` row — including a failed
//! send, whose `error` is the whole point of a delivery log — and moves the
//! device's `last_sync_at`/`last_sync_summary` through
//! [`stump_devices::DeviceService::record_delivery`].
//!
//! The crate owns no transport: [`KindleMailer`] is implemented by the caller
//! (the GraphQL resolver wires it to the primary emailer and
//! `stump_notify::EmailChannel`), which is what keeps one SMTP client, one
//! forbidden-recipient list and one send history for the whole server.
//!
//! Decisions, layout, and verification commands: `crates/kindle/README.md`.

mod convert;
mod error;
mod file;
mod send;

#[cfg(test)]
mod tests;

pub use convert::{BokoConverter, Conversion, KindleConverter};
pub use error::{KindleError, KindleResult};
pub use file::{
	prepare_file, FormatPolicy, KindleFile, AMAZON_MAX_ATTACHMENT_BYTES, KINDLE_FORMAT,
	KINDLE_FORMATS,
};
pub use models::entity::kindle_delivery::Model as KindleDeliveryRow;
pub use send::{deliveries, subject, Delivery, KindleMailer, KindleSend};
