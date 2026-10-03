//! Where the data comes from. Each source is a tokio task that pushes `Event`s into the
//! loop; nothing here touches `App` or the UI.

pub mod clickhouse;
pub mod redash;
