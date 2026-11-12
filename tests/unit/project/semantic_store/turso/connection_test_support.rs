use super::*;

impl TursoSemanticStore {
    pub(crate) fn connection(&self) -> &turso::Connection {
        &self.connection
    }
}
