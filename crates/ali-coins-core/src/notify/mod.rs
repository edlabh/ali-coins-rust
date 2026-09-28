//! Notificações: HTTP seguro, heartbeat e (em seguida) Telegram/webhooks.

pub mod heartbeat;
pub mod http;

pub use heartbeat::{
    HeartbeatAction, HeartbeatConfig, HeartbeatResult, build_action_body, build_action_url,
    mask_heartbeat_url, normalize_base_url, send_heartbeat,
};
pub use http::{HttpError, SafeHttpClient, SafeResponse};
