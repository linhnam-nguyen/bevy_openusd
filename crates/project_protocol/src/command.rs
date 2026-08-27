use serde::{Deserialize, Serialize};

use crate::{ProjectReadError, ProjectReadRequest, ProjectReadResponse};

/// Version of the shared Project read command boundary.
pub const PROJECT_READ_PROTOCOL_VERSION: u16 = 1;

/// Versioned command envelope used by native hosts and frontend adapters.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectReadCommand {
    pub protocol_version: u16,
    pub request: ProjectReadRequest,
}

impl ProjectReadCommand {
    pub fn new(request: ProjectReadRequest) -> Self {
        Self {
            protocol_version: PROJECT_READ_PROTOCOL_VERSION,
            request,
        }
    }

    pub fn validate(&self) -> Result<(), ProjectReadError> {
        if self.protocol_version != PROJECT_READ_PROTOCOL_VERSION {
            return Err(ProjectReadError::UnsupportedProtocolVersion {
                expected: PROJECT_READ_PROTOCOL_VERSION,
                actual: self.protocol_version,
            });
        }
        Ok(())
    }
}

/// Versioned response envelope. The result keeps success and typed failure on
/// the same transport shape without leaking backend implementation errors.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectReadReply {
    pub protocol_version: u16,
    pub result: Result<ProjectReadResponse, ProjectReadError>,
}

impl ProjectReadReply {
    pub fn success(response: ProjectReadResponse) -> Self {
        Self {
            protocol_version: PROJECT_READ_PROTOCOL_VERSION,
            result: Ok(response),
        }
    }

    pub fn failure(error: ProjectReadError) -> Self {
        Self {
            protocol_version: PROJECT_READ_PROTOCOL_VERSION,
            result: Err(error),
        }
    }
}
