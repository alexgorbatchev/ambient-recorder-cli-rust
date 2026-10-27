mod app;
pub mod cli;
mod completion;
mod output;
pub use ambient_recorder::{Error, Result};
pub use app::{App, Prepared, prepare};
