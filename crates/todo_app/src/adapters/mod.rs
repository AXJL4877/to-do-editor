pub mod agent_bridge;
pub mod desktop;
// Drives the application director against the real host, renderer and workspace.
pub mod director;
pub mod editing_proxies;
// Timeline export runs on the dedicated worker and is also used by offline tests.
pub mod export;
// Offscreen program frames, for agents reviewing their own edit.
pub mod frame;
// The on-disk motion library (user presets and extension presets).
pub mod motions;
pub mod paths;
pub mod preview;
pub mod production;
pub mod workflow;
pub mod workers;
