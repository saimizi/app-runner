use std::{error::Error, fmt::Display};

#[derive(Debug)]
pub enum ArunError {
    InvalidValue,
    DockerErr,
    Unknown,
}

impl Display for ArunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let err_str = match self {
            ArunError::InvalidValue => "Invalid Parameter",
            ArunError::DockerErr => "Docker error",
            ArunError::Unknown => "Unknown error",
        };

        write!(f, "{}", err_str)
    }
}

impl Error for ArunError {}