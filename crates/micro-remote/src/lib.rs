//! Remote control: handing a running session to a phone.
//!
//! A session on this machine can be watched and driven from a phone across the room or
//! across the world. The phone reaches it through a relay, which routes frames it
//! cannot read: everything between the two ends is sealed under a key derived from a
//! secret they share and the relay never sees ([`crypto`]).
//!
//! This is the machine's half. It is built in rather than shipped as an extension
//! because the extension host is someone else's code running beside micro, and the
//! thing being handed over is the session itself.

mod bridge;
mod crypto;
mod enrol;
mod pairing;
mod protocol;
mod relay;

pub use bridge::AvailableModel;
pub use bridge::Bridge;
pub use bridge::Delivery;
pub use bridge::Session;
pub use bridge::SessionState;
pub use bridge::SlashCommand;
pub use enrol::begin as begin_enrolment;
pub use enrol::Code;
pub use enrol::Enrolment;
pub use enrol::Half;
pub use enrol::CODE_LIFETIME_SECONDS;
pub use crypto::derive_key;
pub use crypto::open;
pub use crypto::seal;
pub use crypto::seal_with_nonce;
pub use crypto::CryptoError;
pub use crypto::Direction;
pub use crypto::WireFrame;
pub use pairing::create as create_pairing;
pub use pairing::write as write_pairing;
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
pub use relay::ConnectionState;
pub use relay::RelayClient;
pub use relay::RelayConfig;
pub use relay::RelayEvent;
pub use relay::Role;
