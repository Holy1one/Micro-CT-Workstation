//! Physical-device boundary for the production engine.
//!
//! Each child directory owns one device family and may evolve independently.
//! The current implementations are PC-direct drivers. A future controller-
//! managed topology must implement the same domain behavior without moving
//! scan state or safety authority out of `ct-engine`.

pub mod camera;
pub mod turntable;
pub mod xray;
