//! Domain layer: lifecycle rules and the types they operate on.
//!
//! Nothing here knows about HTTP or SQL. That is what makes expiry, burn and
//! capability checks testable without a server and without a database.

pub mod abuse;
pub mod capability;
pub mod clock;
pub mod id;
pub mod paste;
pub mod purge;

pub use capability::{VERIFIER_BYTES, Verifier};
pub use clock::{Clock, SystemClock};
pub use id::{ID_BYTES, ID_CHARS, PasteId};
pub use paste::{Availability, NewPaste, Paste};
pub use purge::{PurgeOutcome, PurgePolicy};
