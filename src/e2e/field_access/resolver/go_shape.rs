use super::super::types::FieldResolver;

impl FieldResolver {
    /// Whether the Go binding emits a JSON byte payload whose length can be tested.
    pub fn target_field_is_raw_message(&self, field: &str) -> bool {
        super::super::ir_result_fields::raw_message_at_path(
            &self.ir_result_field_map,
            &self.result_relative_path(field),
        )
        .unwrap_or(false)
    }
}
