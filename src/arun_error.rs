use std::{error::Error, fmt::Display};

#[derive(Debug)]
pub enum ArunError {
    InvalidValue,
    DockerErr,
    ConflictedWithOther,
    #[cfg(feature = "ctlif-ipcon")]
    IpconError,
    Unknown,
}

impl Display for ArunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let err_str = match self {
            ArunError::InvalidValue => "Invalid Parameter",
            ArunError::DockerErr => "Docker error",
            ArunError::ConflictedWithOther => "Another app with same name exists",

            #[cfg(feature = "ctlif-ipcon")]
            ArunError::IpconError => "Ipcon error",

            ArunError::Unknown => "Unknown error",
        };

        write!(f, "{}", err_str)
    }
}

impl Error for ArunError {}
