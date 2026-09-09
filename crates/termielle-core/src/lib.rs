mod config;
mod event_log;
mod island;
mod protocol;
mod reducer;

pub use config::{
    AppConfig, AssetCatalog, ConfigError, ReducedMotion, RenderMode, WindowPosition, load_config,
    save_config_atomic,
};
pub use event_log::{DEFAULT_EVENT_LOG_MAX_BYTES, EventLog, EventLogError};
pub use island::{
    BarConfig, BarPosition, GlassConfig, IslandConfig, IslandGeometry, IslandLayout, SpringParams,
    bar_anchored_position, island_anchored_position, spring_params,
};
pub use protocol::{
    EventKind, EventMessage, MAX_EVENT_BYTES, MAX_SESSION_ID_BYTES, MAX_SOURCE_BYTES,
    PROTOCOL_VERSION, ProtocolError, Source, decode_event_line, encode_event_line,
};
pub use reducer::{ApplyOutcome, SessionReducer, VisualState};
