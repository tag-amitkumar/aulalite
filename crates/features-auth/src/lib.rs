// crates/features-auth/src/lib.rs
//! Auth screens.
#![allow(clippy::redundant_locals, clippy::too_many_arguments)]

pub mod forgot;
pub mod login;
pub mod signup;

pub use forgot::ForgotPassword;
pub use login::{login_internals, Login, LoginProps};
pub use signup::{Signup, SignupProps};
