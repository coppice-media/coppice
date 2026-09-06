pub mod audio;
pub mod epub;
pub mod mobi;
#[cfg(feature = "pdf")]
pub mod pdf;
#[cfg(feature = "rar")]
pub mod rar;
pub mod zip;
