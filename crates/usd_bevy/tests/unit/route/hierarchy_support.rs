use super::*;

impl HierarchyMetadataIndex {
    pub(in crate::route) fn display_name(&self, path: &str) -> Option<&str> {
        self.display_names.get(path).map(String::as_str)
    }
}
