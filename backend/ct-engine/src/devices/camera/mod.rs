//! Native camera transport and single-owner controls. No external camera bridge.
mod actor;
mod capture;
mod live_view;
mod nikon;
mod ptp;
mod properties;
mod types;
mod wpd;

pub use actor::CameraAdapter;
pub use live_view::{
    decode_live_view_frame, encode_live_view_frame, encode_live_view_ack, LiveViewFrame, LiveViewPipeDescriptor,
    LIVE_VIEW_ACK_BYTES, LIVE_VIEW_HEADER_BYTES, LIVE_VIEW_PIPE_VERSION, MAX_LIVE_VIEW_BYTES,
};
pub use nikon::{
    exact_shutter_value, profile_readback, independent_readback, shutter_ms, shutter_property_ms, validate_properties,
    PROFILE_REQUIRED_KEYS,
};
pub use properties::{capture_readiness, property_rejection, PropertyRejection};
pub use types::*;

#[cfg(test)]
mod tests;
#[cfg(test)]
pub(crate) use tests::{scripted_camera,scripted_camera_with_probe,ScriptedCameraConfig};
