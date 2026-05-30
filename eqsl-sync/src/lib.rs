pub mod fetch;
pub mod upload;

pub use fetch::{EqslFetchClient, EqslFetchConfig, EqslInboxRecord, FetchError};
pub use upload::{EqslUploadClient, EqslUploadConfig, UploadError, UploadOutcome};
