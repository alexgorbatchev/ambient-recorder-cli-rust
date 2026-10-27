mod runtime;
mod sink;
mod source;
mod watchdog;
use crate::{Error, Result};
pub use runtime::{RunConfig, Timing, run};
pub use sink::{Event, Sink};
pub use source::Source;
pub use watchdog::Watchdog;

/// Divide before multiplying so recordings spanning months cannot overflow a
/// nanosecond intermediate even when the final duration is representable.
pub fn frame_duration(frames: u64, rate: u32) -> Result<chrono::Duration> {
    if rate == 0 {
        return Err(Error::Invalid("sample rate is zero".into()));
    }
    let seconds = frames / u64::from(rate);
    let nanos = frames % u64::from(rate) * 1_000_000_000 / u64::from(rate);
    chrono::Duration::new(
        i64::try_from(seconds).map_err(|_| Error::Invalid("frame duration overflow".into()))?,
        nanos as u32,
    )
    .ok_or_else(|| Error::Invalid("frame duration overflow".into()))
}
