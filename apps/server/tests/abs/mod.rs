//! The Audiobookshelf profile, mounted on a real server.
//!
//! The route surface is exercised in `stump_abs` against an in-memory
//! backend; what is asserted here is the part only the server can answer:
//! that the profile mounts where an Audiobookshelf client looks for it, that
//! the socket endpoint is reachable without a credential (a socket.io client
//! cannot put a header on its upgrade), and that the `abs` feature plus
//! `STUMP_ENABLE_ABS` are what put both there.

mod mount;
