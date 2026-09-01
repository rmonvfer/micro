//! Remote control: handing a running session to a phone.

mod bridge;
mod crypto;
mod pairing;
mod protocol;
mod relay;

pub use bridge::AvailableModel;
pub use bridge::Bridge;
pub use bridge::Delivery;
pub use bridge::Session;
pub use bridge::SessionState;
pub use bridge::SlashCommand;
pub use crypto::derive_key;
pub use crypto::open;
pub use crypto::seal;
pub use crypto::seal_with_nonce;
pub use crypto::CryptoError;
pub use crypto::Direction;
pub use crypto::WireFrame;
pub use pairing::create as create_pairing;
pub use pairing::load as load_pairing;
pub use pairing::path_in;
pub use pairing::qr_lines;
pub use pairing::Pairing;
pub use protocol::FrameDecoder;
pub use protocol::FrameEncoder;
pub use protocol::MachinePayload;
pub use protocol::PhoneCommand;
pub use protocol::PhonePayload;
pub use protocol::PushKind;
pub use protocol::PushPayload;
pub use relay::register;
#[doc(hidden)]
pub use relay::register_with_client;
pub use relay::ConnectionState;
pub use relay::RelayClient;
pub use relay::RelayConfig;
pub use relay::RelayEvent;
pub use relay::Role;
