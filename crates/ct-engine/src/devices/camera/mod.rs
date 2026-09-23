//! Camera device module.
//!
//! `digicam_control` is the current Nikon D7100 implementation. Future camera
//! transports belong beside it and must preserve host-side file confirmation.

mod digicam_control;

pub use digicam_control::{CameraError, CameraHealth, DigiCamControlAdapter, valid_exposure_ms, EXPOSURE_MIN_MS, EXPOSURE_MAX_MS};
