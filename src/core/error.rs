use std::any::TypeId;

use color_eyre::eyre::Report;
use stardust_xr::{
	messenger::MessengerError,
	schemas::flex::{
		FlexSerializeError,
		flexbuffers::{DeserializationError, ReaderError},
	},
};
use thiserror::Error;

pub type Result<T, E = ServerError> = std::result::Result<T, E>;

#[derive(Error, Debug)]
pub enum ServerError {
	#[error("Internal: Unable to get client")]
	NoClient,
	#[error("Messenger does not exist for this node")]
	NoMessenger,
	#[error("Messenger error: {0}")]
	MessengerError(#[from] MessengerError),
	#[error("Remote method error: {0}")]
	RemoteMethodError(String),
	#[error("Serialization error: {0}")]
	SerializationError(#[from] FlexSerializeError),
	#[error("Deserialization error: {0}")]
	DeserializationError(#[from] DeserializationError),
	#[error("Reader error: {0}")]
	ReaderError(#[from] ReaderError),
	#[error("Aspect {} does not exist for node", 0.to_string())]
	NoAspect(TypeId),
	#[error("{0}")]
	Report(#[from] Report),
}

#[macro_export]
macro_rules! bail {
    ($msg:literal $(,)?) => {
        return Err($crate::core::error::ServerError::from(color_eyre::eyre::eyre!($msg)));
    };
    ($err:expr $(,)?) => {
        return Err($crate::core::error::ServerError::from(color_eyre::eyre::eyre!($err)));
    };
    ($fmt:expr, $($arg:tt)*) => {
        return Err($crate::core::error::ServerError::from(color_eyre::eyre::eyre!($fmt, $($arg)*)));
    };
}

#[macro_export]
macro_rules! ensure {
    ($cond:expr $(,)?) => {
        if !$cond {
            $crate::ensure!($cond, concat!("Condition failed: `", stringify!($cond), "`"))
        }
    };
    ($cond:expr, $msg:literal $(,)?) => {
        if !$cond {
            return Err($crate::core::error::ServerError::from(color_eyre::eyre::eyre!($msg)));
        }
    };
    ($cond:expr, $err:expr $(,)?) => {
        if !$cond {
            return Err($crate::core::error::ServerError::from(color_eyre::eyre::eyre!($err)));
        }
    };
    ($cond:expr, $fmt:expr, $($arg:tt)*) => {
        if !$cond {
            return Err($crate::core::error::ServerError::from(color_eyre::eyre::eyre!($fmt, $($arg)*)));
        }
    };
}
