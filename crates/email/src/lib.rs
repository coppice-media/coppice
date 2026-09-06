//! Email module for sending emails using SMTP. This module uses the `lettre` crate to send emails
//! with plain text bodies until I have a need for HTML
//!
//! See `crates/email/README.md` for the crate contract and decisions.

#![warn(clippy::dbg_macro)]

/// A module containing the emailer client and its configuration, as well as the sending of emails
mod emailer;
/// A module containing the error type for this crate
mod error;

pub use emailer::{AttachmentPayload, EmailerClient, EmailerClientConfig};
pub use error::{EmailError, EmailResult};

pub use lettre::message::header::ContentType as EmailContentType;
