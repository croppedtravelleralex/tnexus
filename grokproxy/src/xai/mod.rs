//! Talking to xAI's account and auth services over HTTP.
//!
//! Two hosts, two clients, for a measured reason:
//!
//! * auth.x.ai (device code, token) answers any client; plain reqwest is used.
//! * accounts.x.ai (SSO validation, device consent) refuses reqwest's TLS
//!   signature with 403 while answering a browser signature with 307. Verified
//!   from an egress that works: curl_cffi impersonating Chrome got 200, curl
//!   without impersonation got 403, and reqwest got 403 from the same machine.
//!   Those calls therefore go through wreq with a Chrome emulation profile.
//!
//! Getting this wrong is not a slow path, it is a hard block, so the split is
//! structural rather than a fallback.

pub mod consent;
pub mod device;
pub mod mint;
pub mod scrape;
