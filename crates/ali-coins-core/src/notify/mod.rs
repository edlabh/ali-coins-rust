//! Notificações: HTTP seguro, Telegram, heartbeat e webhooks.

pub mod heartbeat;
pub mod http;
pub mod telegram;
pub mod webhooks;

pub use heartbeat::{
    HeartbeatAction, HeartbeatConfig, HeartbeatResult, build_action_body, build_action_url,
    mask_heartbeat_url, normalize_base_url, send_heartbeat,
};
pub use http::{HttpError, SafeHttpClient, SafeResponse};
pub use telegram::{
    TelegramConfig, TelegramContext, TelegramEvent, TelegramSendResult, build_message,
    build_unified_report_message, escape_html, send_telegram, truncate_telegram_message,
};
pub use webhooks::{
    WebhookTracker, encode_webhook_payload, format_webhook_body, sanitize_webhook_payload,
    send_webhook,
};
