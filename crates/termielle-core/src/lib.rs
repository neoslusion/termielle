mod config;
mod protocol;
mod reducer;

pub use config::{
    AppConfig, AssetCatalog, ConfigError, ReducedMotion, RenderMode, WindowPosition, load_config,
    save_config_atomic,
};
pub use protocol::{
    EventKind, EventMessage, MAX_EVENT_BYTES, MAX_SESSION_ID_BYTES, PROTOCOL_VERSION,
    ProtocolError, Source, decode_event_line, encode_event_line,
};
pub use reducer::{ApplyOutcome, SessionReducer, VisualState};
